use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyEventKind};
use crossterm::{event, execute, terminal};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tokio::sync::{mpsc, RwLock};

use crate::ai::api_client::{ApiKeyProvider, ApiKeyProviderConfig};
use crate::ai::provider::{Provider, Role};
use crate::ai::session::ChatSession;
use crate::config::Config;
use crate::dao::Registry;
use crate::ui::chat::{ChatAction, ChatWidget};
use crate::ui::key::{self, format_hints, Action, LIST_HINTS};
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
            Self::Connecting => "Connecting…".to_owned(),
            Self::Connected { context, version } => format!("ctx:{context}  {version}"),
            Self::Error(e) => format!("Error: {}", &e[..e.len().min(40)]),
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::Disconnected => Color::DarkGray,
            Self::Connecting => Color::Yellow,
            Self::Connected { .. } => Color::Green,
            Self::Error(_) => Color::Red,
        }
    }
}

/// Active input mode.
#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Browse,
    Command,
    /// Chat window is open and capturing key input.
    Chat,
}

/// Result of an async AI call, sent back to the UI loop via mpsc.
#[derive(Debug)]
enum AiReply {
    /// Successful response from the LLM.
    Ok(String),
    /// The LLM call failed.
    Err(String),
}

/// Full application state.
pub struct App {
    pub config: Config,
    pub connection: ConnectionState,
    pub registry: Registry,
    mode: Mode,
    prompt: Prompt,
    should_quit: bool,
    status: Option<String>,
    status_expiry: Option<Instant>,

    /// The current browser view, swapped when the user navigates to a new resource.
    pub browser: Option<BrowserView>,
    /// Shared watcher factory — `None` until cluster connection is established.
    pub factory: Option<Arc<RwLock<WatcherFactory>>>,
    /// Active namespace filter (`None` = all namespaces).
    pub namespace: Option<String>,

    // ── AI Chat ───────────────────────────────────────────────────────────────
    /// Rendered chat window widget.
    chat: ChatWidget,
    /// Conversation state (history + token budget).
    chat_session: Option<ChatSession>,
    /// LLM provider — `None` when no API key / provider is configured.
    chat_provider: Option<Arc<dyn Provider>>,
    /// Channel for receiving AI replies from the spawned background task.
    ai_reply_tx: mpsc::Sender<AiReply>,
    ai_reply_rx: mpsc::Receiver<AiReply>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (ai_reply_tx, ai_reply_rx) = mpsc::channel(8);

        // Build the LLM provider from config if an API key is available.
        let chat_provider = build_provider(&config);

        // Always create a session — it works with any provider (or none, for offline testing).
        let chat_session = Some(ChatSession::new(
            &config.k7s.ai.token_budget,
            &config.k7s.ai.sanitizer,
        ));

        Self {
            config,
            connection: ConnectionState::Disconnected,
            registry: Registry::with_builtins(),
            mode: Mode::Browse,
            prompt: Prompt::new(),
            should_quit: false,
            status: None,
            status_expiry: None,
            browser: None,
            factory: None,
            namespace: None,
            chat: ChatWidget::new(),
            chat_session,
            chat_provider,
            ai_reply_tx,
            ai_reply_rx,
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

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
            self.flash(format!("Unknown resource: {alias}"), Duration::from_secs(3));
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

        // Drain pending AI replies.
        while let Ok(reply) = self.ai_reply_rx.try_recv() {
            self.chat.loading = false;
            match reply {
                AiReply::Ok(text) => {
                    self.chat.push_message(Role::Assistant, text.clone());
                    // Persist in session history.
                    if let Some(session) = &mut self.chat_session {
                        session.history_push_assistant(text);
                    }
                }
                AiReply::Err(e) => {
                    self.chat.push_message(Role::System, format!("Error: {e}"));
                }
            }
        }
    }
}

// ─── Provider construction ────────────────────────────────────────────────────

fn build_provider(config: &Config) -> Option<Arc<dyn Provider>> {
    let ai = &config.k7s.ai;
    // Use the API key from config, then fall back to the environment variable.
    let api_key: String = ai
        .api_key
        .clone()
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("K7S_LLM_API_KEY").ok())
        .unwrap_or_default();

    if api_key.is_empty() {
        tracing::info!("No LLM API key configured — AI chat will be available in demo mode");
        return None;
    }

    let cfg = ApiKeyProviderConfig {
        endpoint: ai
            .endpoint
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1/chat/completions".to_owned()),
        api_key,
        model: "gpt-4o-mini".to_owned(),
        max_tokens: 2048,
        temperature: 0.3,
    };

    Some(Arc::new(ApiKeyProvider::new(cfg)))
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

        if app.should_quit() {
            break;
        }
    }

    Ok(())
}

// ─── Input handling ────────────────────────────────────────────────────────────

fn handle_key_event(app: &mut App, key: crossterm::event::KeyEvent) {
    // ── Chat mode: forward all keys to the chat widget ────────────────────────
    if app.mode == Mode::Chat {
        let action = app.chat.handle_key(&key);
        match action {
            ChatAction::Close => {
                app.mode = Mode::Browse;
            }
            ChatAction::Submit(text) => {
                submit_chat_message(app, text);
            }
            ChatAction::None => {}
        }
        return;
    }

    // ── Command mode ─────────────────────────────────────────────────────────
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

    // ── Browse mode ───────────────────────────────────────────────────────────
    let action = key::resolve(&key);
    match action {
        Action::Quit => app.quit(),
        Action::CommandPrompt => {
            app.mode = Mode::Command;
            app.prompt.activate();
        }
        Action::Up => {
            if let Some(b) = &mut app.browser {
                b.up();
            }
        }
        Action::Down => {
            if let Some(b) = &mut app.browser {
                b.down();
            }
        }
        Action::PageUp => {
            if let Some(b) = &mut app.browser {
                b.page_up();
            }
        }
        Action::PageDown => {
            if let Some(b) = &mut app.browser {
                b.page_down();
            }
        }
        Action::Top => {
            if let Some(b) = &mut app.browser {
                b.top();
            }
        }
        Action::Bottom => {
            if let Some(b) = &mut app.browser {
                b.bottom();
            }
        }
        Action::Filter => {
            app.mode = Mode::Command;
            app.prompt.activate();
        }
        Action::Chat => {
            app.mode = Mode::Chat;
            if app.chat.messages.is_empty() {
                let provider_hint = if app.chat_provider.is_some() {
                    "Connected to LLM provider."
                } else {
                    "No API key configured — set K7S_LLM_API_KEY or add ai.apiKey to config."
                };
                app.chat.push_message(
                    Role::System,
                    format!(
                        "k7s AI chat — ask questions about your cluster.\n{}\nPress Esc to close.",
                        provider_hint
                    ),
                );
            }
        }
        Action::Help => app.flash("Help view (Phase 6)", Duration::from_secs(3)),
        Action::Describe => {
            if let Some(name) = app.browser.as_ref().and_then(|b| b.selected_name()) {
                app.flash(
                    format!("Describe: {name} (Phase 6)"),
                    Duration::from_secs(3),
                );
            }
        }
        _ => {}
    }
}

/// Submit a user message to the LLM, update the widget, and spawn the async call.
fn submit_chat_message(app: &mut App, text: String) {
    // Show the user message immediately.
    app.chat.push_message(Role::User, text.clone());
    app.chat.loading = true;

    let Some(provider) = app.chat_provider.clone() else {
        // No provider — echo a demo reply.
        app.chat.loading = false;
        app.chat.push_message(
            Role::Assistant,
            "No LLM provider configured. Set K7S_LLM_API_KEY to enable AI responses.".to_owned(),
        );
        return;
    };

    // Build the message list from session state.
    let messages = if let Some(session) = &app.chat_session {
        session.messages_for_send(&text)
    } else {
        vec![crate::ai::provider::Message::user(text.clone())]
    };

    // Persist user turn in session history.
    if let Some(session) = &mut app.chat_session {
        session.history_push_user(text);
    }

    // Spawn async AI call — reply comes back via the channel.
    let tx = app.ai_reply_tx.clone();
    tokio::spawn(async move {
        let result = provider.complete(&messages).await;
        let reply = match result {
            Ok(text) => AiReply::Ok(text),
            Err(e) => AiReply::Err(e.to_string()),
        };
        let _ = tx.send(reply).await;
    });
}

// ─── Rendering ────────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(0),    // main (browser + optional chat)
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

    let resource_label = app
        .browser
        .as_ref()
        .map(|b| b.title.as_str())
        .unwrap_or("pods");

    let ns_label = app.namespace.as_deref().unwrap_or("(all)");

    let chat_indicator = if app.mode == Mode::Chat {
        "  [chat]"
    } else {
        ""
    };

    let title = Paragraph::new(format!(
        " k7s  {}  › {}  ns:{}{}",
        env!("CARGO_PKG_VERSION"),
        resource_label,
        ns_label,
        chat_indicator,
    ))
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(title, chunks[0]);

    let conn = Paragraph::new(format!(" {}", app.connection.label()))
        .style(Style::default().fg(app.connection.color()))
        .block(Block::default().borders(Borders::BOTTOM))
        .alignment(Alignment::Right);
    frame.render_widget(conn, chunks[1]);
}

fn render_main(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.mode == Mode::Chat {
        // Split: browser (left 55%) | chat (right 45%)
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        // Browser still visible on the left.
        if let Some(browser) = &mut app.browser {
            browser.render(frame, split[0]);
        } else {
            render_placeholder(frame, split[0]);
        }

        // Chat window on the right.
        app.chat.render(frame, split[1]);
        return;
    }

    // Normal browse mode — full-width browser.
    if let Some(browser) = &mut app.browser {
        browser.render(frame, area);
    } else {
        render_placeholder(frame, area);
    }
}

fn render_placeholder(frame: &mut Frame, area: Rect) {
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

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(prompt_text) = app.prompt.display() {
        frame.render_widget(
            Paragraph::new(prompt_text).style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            area,
        );
        return;
    }

    if let Some(status) = &app.status {
        frame.render_widget(
            Paragraph::new(format!(" {status}")).style(Style::default().fg(Color::Yellow)),
            area,
        );
        return;
    }

    frame.render_widget(
        Paragraph::new(format_hints(LIST_HINTS)).style(Style::default().fg(Color::DarkGray)),
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
        handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Char(':'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        );
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
    fn space_opens_chat_mode() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = App::new(Config::default());
        handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        );
        assert_eq!(app.mode, Mode::Chat);
    }

    #[test]
    fn esc_in_chat_returns_to_browse() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = App::new(Config::default());
        app.mode = Mode::Chat;
        handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        );
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn chat_session_is_created_on_startup() {
        let app = App::new(Config::default());
        assert!(app.chat_session.is_some());
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
