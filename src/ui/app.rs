use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyEventKind};
use crossterm::{event, execute, terminal};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::dao::Registry;
use crate::ui::key::{self, Action, format_hints, LIST_HINTS};
use crate::ui::prompt::{Prompt, PromptSubmit};
use crate::view::BrowserView;
use crate::watch::WatcherFactory;

/// Cluster connection state.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected { context: String, version: String },
    Error(String),
}

impl ConnectionState {
    fn label(&self) -> String {
        match self {
            Self::Disconnected => "No cluster".to_owned(),
            Self::Connecting   => "Connecting…".to_owned(),
            Self::Connected { context, version } => format!("ctx:{context}  {version}"),
            Self::Error(e)     => format!("Error: {}", &e[..e.len().min(40)]),
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::Disconnected => Color::DarkGray,
            Self::Connecting   => Color::Yellow,
            Self::Connected {..} => Color::Green,
            Self::Error(_)     => Color::Red,
        }
    }
}

/// Active input mode.
#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Browse,
    Command,
}

/// Full application state.
pub struct App {
    pub config:     Config,
    pub connection: ConnectionState,
    pub registry:   Registry,
    mode:           Mode,
    prompt:         Prompt,
    should_quit:    bool,
    status:         Option<String>,
    status_expiry:  Option<Instant>,

    /// The current browser view, swapped when the user navigates to a new resource.
    pub browser:    Option<BrowserView>,
    /// Shared watcher factory — `None` until cluster connection is established.
    pub factory:    Option<Arc<RwLock<WatcherFactory>>>,
    /// Active namespace filter (`None` = all namespaces).
    pub namespace:  Option<String>,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            connection:    ConnectionState::Disconnected,
            registry:      Registry::with_builtins(),
            mode:          Mode::Browse,
            prompt:        Prompt::new(),
            should_quit:   false,
            status:        None,
            status_expiry: None,
            browser:       None,
            factory:       None,
            namespace:     None,
        }
    }

    pub fn quit(&mut self) { self.should_quit = true; }
    pub fn should_quit(&self) -> bool { self.should_quit }

    /// Flash a status message that auto-clears after `duration`.
    pub fn flash(&mut self, msg: impl Into<String>, duration: Duration) {
        self.status = Some(msg.into());
        self.status_expiry = Some(Instant::now() + duration);
    }

    /// Navigate to a resource type by alias (e.g. "po", "pods", "deploy").
    ///
    /// Swaps the current browser view and starts the watcher if needed.
    pub fn navigate(&mut self, alias: &str) {
        if let Some(view) = crate::view::browser_for_resource(alias, &self.registry) {
            self.browser = Some(view);
        } else {
            self.flash(
                format!("Unknown resource: {alias}"),
                Duration::from_secs(3),
            );
        }
    }

    fn tick(&mut self) {
        // Expire status messages.
        if let Some(expiry) = self.status_expiry {
            if Instant::now() >= expiry {
                self.status = None;
                self.status_expiry = None;
            }
        }
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

/// Run the terminal UI (synchronous wrapper around the async loop).
///
/// Initialises the raw-mode terminal, runs the event loop, then restores the
/// terminal whether the loop exits cleanly or via a panic.
pub fn run(config: Config) -> anyhow::Result<()> {
    // Run a minimal tokio runtime so the app can drive async watchers later.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_async(config))
}

async fn run_async(config: Config) -> anyhow::Result<()> {
    let mut terminal = init_terminal()?;
    let result = run_loop(&mut terminal, config).await;
    restore_terminal(&mut terminal)?;
    result
}

fn init_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
) -> anyhow::Result<()> {
    let mut app = App::new(config);

    // Start on the pods view by default.
    app.navigate("pods");

    loop {
        app.tick();
        terminal.draw(|frame| render(frame, &mut app))?;

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key_event(&mut app, key);
                }
            }
        }

        if app.should_quit() { break; }
    }

    Ok(())
}

// ─── Input handling ────────────────────────────────────────────────────────────

fn handle_key_event(app: &mut App, key: crossterm::event::KeyEvent) {
    if app.mode == Mode::Command {
        if let Some(submit) = app.prompt.handle_key(&key) {
            app.mode = Mode::Browse;
            match submit {
                PromptSubmit::Navigate(resource) => app.navigate(&resource),
                PromptSubmit::Namespace(ns) => {
                    let label = ns.as_deref().unwrap_or("(all)").to_owned();
                    app.namespace = ns;
                    app.flash(format!("Namespace: {label}"), Duration::from_secs(2));
                }
                PromptSubmit::Context(ctx) => {
                    app.flash(format!("Switching context: {ctx}"), Duration::from_secs(2));
                }
                PromptSubmit::Filter(f) => {
                    if let Some(b) = &mut app.browser {
                        b.set_filter(f.clone());
                    }
                    app.flash(format!("Filter: {f}"), Duration::from_secs(2));
                }
                PromptSubmit::Cancel => {}
            }
        }
        return;
    }

    let action = key::resolve(&key);
    match action {
        Action::Quit          => app.quit(),
        Action::CommandPrompt => { app.mode = Mode::Command; app.prompt.activate(); }
        Action::Up            => { if let Some(b) = &mut app.browser { b.up(); } }
        Action::Down          => { if let Some(b) = &mut app.browser { b.down(); } }
        Action::PageUp        => { if let Some(b) = &mut app.browser { b.page_up(); } }
        Action::PageDown      => { if let Some(b) = &mut app.browser { b.page_down(); } }
        Action::Top           => { if let Some(b) = &mut app.browser { b.top(); } }
        Action::Bottom        => { if let Some(b) = &mut app.browser { b.bottom(); } }
        Action::Filter        => {
            app.mode = Mode::Command;
            app.prompt.activate();
        }
        Action::Chat          => app.flash("AI chat window (Phase 12 — coming soon)", Duration::from_secs(3)),
        Action::Help          => app.flash("Help view (Phase 6)", Duration::from_secs(3)),
        Action::Describe      => {
            if let Some(name) = app.browser.as_ref().and_then(|b| b.selected_name()) {
                app.flash(format!("Describe: {name} (Phase 6)"), Duration::from_secs(3));
            }
        }
        _ => {}
    }
}

// ─── Rendering ────────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(0),    // main (browser table)
            Constraint::Length(1), // footer (prompt / status / hints)
        ])
        .split(area);

    render_header(frame, app, chunks[0]);
    render_main(frame, app, chunks[1]);
    render_footer(frame, app, chunks[2]);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(45)])
        .split(area);

    let resource_label = app.browser.as_ref()
        .map(|b| b.title.as_str())
        .unwrap_or("pods");

    let ns_label = app.namespace.as_deref().unwrap_or("(all)");

    let title = Paragraph::new(format!(
        " k7s  {}  › {}  ns:{}",
        env!("CARGO_PKG_VERSION"),
        resource_label,
        ns_label
    ))
    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(title, chunks[0]);

    let conn = Paragraph::new(format!(" {}", app.connection.label()))
        .style(Style::default().fg(app.connection.color()))
        .block(Block::default().borders(Borders::BOTTOM))
        .alignment(Alignment::Right);
    frame.render_widget(conn, chunks[1]);
}

fn render_main(frame: &mut Frame, app: &mut App, area: Rect) {
    if let Some(browser) = &mut app.browser {
        browser.render(frame, area);
    } else {
        let placeholder = Paragraph::new(concat!(
            "\n\n  Welcome to k7s — Security-First Kubernetes TUI\n\n",
            "  No cluster connected.\n\n",
            "  Press : to navigate (e.g. :pods, :nodes, :deploy)\n",
            "  Press Space to open the AI chat window\n",
            "  Press ? for help  •  Press q to quit\n",
        ))
        .style(Style::default().fg(Color::Gray));
        frame.render_widget(placeholder, area);
    }
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(prompt_text) = app.prompt.display() {
        frame.render_widget(
            Paragraph::new(prompt_text)
                .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            area,
        );
        return;
    }

    if let Some(status) = &app.status {
        frame.render_widget(
            Paragraph::new(format!(" {status}"))
                .style(Style::default().fg(Color::Yellow)),
            area,
        );
        return;
    }

    frame.render_widget(
        Paragraph::new(format_hints(LIST_HINTS))
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_starts_not_quitting() {
        let app = App::new(Config::default());
        assert!(!app.should_quit());
    }

    #[test]
    fn app_quits_after_quit_call() {
        let mut app = App::new(Config::default());
        app.quit();
        assert!(app.should_quit());
    }

    #[test]
    fn q_key_triggers_quit() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = App::new(Config::default());
        let key = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        handle_key_event(&mut app, key);
        assert!(app.should_quit());
    }

    #[test]
    fn colon_opens_command_mode() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = App::new(Config::default());
        handle_key_event(&mut app, KeyEvent {
            code: KeyCode::Char(':'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        assert_eq!(app.mode, Mode::Command);
    }

    #[test]
    fn navigate_pods_creates_browser() {
        let mut app = App::new(Config::default());
        app.navigate("pods");
        assert!(app.browser.is_some());
        assert_eq!(app.browser.as_ref().unwrap().title, "Pods");
    }

    #[test]
    fn navigate_unknown_alias_sets_status() {
        let mut app = App::new(Config::default());
        app.navigate("doesnotexist");
        assert!(app.status.is_some());
    }

    #[test]
    fn connection_labels() {
        assert!(ConnectionState::Disconnected.label().contains("No cluster"));
        let c = ConnectionState::Connected {
            context: "prod".into(),
            version: "v1.30".into(),
        };
        assert!(c.label().contains("prod"));
    }
}
