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
use crate::config::{Config, ConfigDirs};
use crate::dao::Registry;
use crate::health::ClusterSummary;
use crate::model::NavHistory;
use crate::ui::chat::{ChatAction, ChatWidget};
use crate::ui::dialog::{
    ConfirmAction, ConfirmDialog, ImageUpdateAction, ImageUpdateDialog, PortForwardAction,
    PortForwardDialog, ScaleAction, ScaleDialog,
};
use crate::ui::key::{self, format_hints, Action, LIST_HINTS};
use crate::ui::prompt::{Prompt, PromptSubmit};
use crate::view::BrowserView;
use crate::view::{
    HelpAction, HelpView, LogAction, LogView, PulseAction, PulseView, WorkloadAction, WorkloadView,
};
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
    /// Help overlay is open.
    Help,
    /// Chat window is open and capturing key input.
    Chat,
    /// Pulse dashboard is fullscreen.
    Pulse,
    /// Workload aggregated view is fullscreen.
    Workload,
    /// Fullscreen log viewer.
    Log,
    /// Delete-confirmation overlay is open.
    Confirm,
    /// Scale-replica-count overlay is open.
    Scale,
    /// Port-forward setup dialog is open.
    PortForward,
    /// Image update dialog is open.
    ImageUpdate,
}

/// Result of a background cluster operation (delete / scale / restart).
#[derive(Debug)]
enum OpResult {
    Ok(String),
    Err(String),
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
    /// Live Kubernetes client — `Some` once successfully connected to a cluster.
    pub kube_client: Option<kube::Client>,

    /// Navigation history — tracks visited resource aliases for `[`/`]`/`-`.
    history: NavHistory,

    /// Help overlay widget (lazy-init on first `?` press).
    help: HelpView,
    /// Pulse cluster-dashboard view.
    pulse: PulseView,
    /// Workload aggregated view.
    workload: WorkloadView,
    /// Log viewer.
    log: LogView,

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

    // ── Dialogs ───────────────────────────────────────────────────────────────
    /// Active delete-confirmation dialog.  `Some` only when `mode == Mode::Confirm`.
    confirm_dialog: Option<ConfirmDialog>,
    /// Pending delete target: (gvr resource string, namespace, name).
    pending_delete: Option<(String, Option<String>, String)>,
    /// Active scale dialog.  `Some` only when `mode == Mode::Scale`.
    scale_dialog: Option<ScaleDialog>,
    /// Pending scale target: (gvr, namespace, name).
    pending_scale: Option<(crate::client::Gvr, String, String)>,
    /// Active port-forward dialog.  `Some` only when `mode == Mode::PortForward`.
    pf_dialog: Option<PortForwardDialog>,
    /// Pending port-forward target: (namespace, pod_name).
    pending_pf: Option<(String, String)>,
    /// Active image-update dialog.  `Some` only when `mode == Mode::ImageUpdate`.
    image_dialog: Option<ImageUpdateDialog>,
    /// Pending image-update target: (resource_kind, namespace, name).
    pending_image: Option<(String, String, String)>,
    /// Port-forward manager — owns all active kubectl subprocesses.
    pf_manager: crate::portforward::PortForwardManager,

    // ── Async operation channel ───────────────────────────────────────────────
    /// Receives the outcome of background delete/scale/restart tasks.
    op_result_tx: mpsc::Sender<OpResult>,
    op_result_rx: mpsc::Receiver<OpResult>,

    // ── Config live-reload ────────────────────────────────────────────────────
    /// Receives `()` whenever the config file changes on disk.
    config_reload_rx: Option<mpsc::Receiver<()>>,
    /// Keeps the `notify` watcher alive for the duration of the app.
    _config_watcher: Option<crate::config::watcher::ConfigWatcher>,
    /// Resolved path to the config file (used for reloading).
    config_path: Option<std::path::PathBuf>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (ai_reply_tx, ai_reply_rx) = mpsc::channel(8);
        let (op_result_tx, op_result_rx) = mpsc::channel(8);

        // Build the LLM provider from config if an API key is available.
        let chat_provider = build_provider(&config);

        // Always create a session — it works with any provider (or none, for offline testing).
        let chat_session = Some(ChatSession::new(
            &config.k7s.ai.token_budget,
            &config.k7s.ai.sanitizer,
        ));

        let registry = Registry::with_builtins();

        // Pre-populate the prompt with all known resource aliases + built-in commands.
        let mut prompt = Prompt::new();
        let mut candidates: Vec<String> = registry
            .all_sorted()
            .into_iter()
            .flat_map(|m| m.aliases.iter().cloned())
            .collect();
        // Add built-in command verbs.
        for cmd in &[
            "alias",
            "aliases",
            "ctx",
            "context",
            "ns",
            "namespace",
            "help",
            "pulse",
            "workload",
            "wl",
        ] {
            if !candidates.iter().any(|c| c == cmd) {
                candidates.push(cmd.to_string());
            }
        }
        candidates.sort();
        candidates.dedup();
        prompt.set_candidates(candidates);

        // Set up config live-reload watcher if the config path is resolvable.
        let (config_watcher, config_reload_rx, config_path) =
            match ConfigDirs::resolve().map(|d| d.config_file()) {
                Ok(path) => match crate::config::watcher::ConfigWatcher::new(&path) {
                    Ok((w, rx)) => (Some(w), Some(rx), Some(path)),
                    Err(e) => {
                        tracing::debug!(error = %e, "config watcher could not start");
                        (None, None, Some(path))
                    }
                },
                Err(_) => (None, None, None),
            };

        Self {
            config,
            connection: ConnectionState::Disconnected,
            registry,
            mode: Mode::Browse,
            prompt,
            should_quit: false,
            status: None,
            status_expiry: None,
            browser: None,
            factory: None,
            namespace: None,
            kube_client: None,
            history: NavHistory::new(),
            help: HelpView::new(),
            pulse: PulseView::new(),
            workload: WorkloadView::new(),
            chat: ChatWidget::new(),
            chat_session,
            chat_provider,
            ai_reply_tx,
            ai_reply_rx,
            log: LogView::new("", vec![]),
            confirm_dialog: None,
            pending_delete: None,
            scale_dialog: None,
            pending_scale: None,
            pf_dialog: None,
            pending_pf: None,
            image_dialog: None,
            pending_image: None,
            pf_manager: crate::portforward::PortForwardManager::new(),
            op_result_tx,
            op_result_rx,
            config_reload_rx,
            _config_watcher: config_watcher,
            config_path,
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
    /// Swaps the current browser view, records the visit in [`NavHistory`],
    /// and starts the watcher if needed.
    ///
    /// The special alias `"ctx"` / `"context"` opens the kubeconfig context
    /// browser (no API call; reads from disk).
    pub fn navigate(&mut self, alias: &str) {
        // Special case: context view loads from kubeconfig, not the K8s API.
        if matches!(alias, "ctx" | "context") {
            self.history.push(alias);
            self.browser = Some(crate::view::context_browser());
            return;
        }

        // Special case: pulse and workload are fullscreen dedicated views.
        if matches!(alias, "pulse") {
            self.history.push(alias);
            self.pulse = PulseView::new();
            // Seed with an empty summary (real data flows via tick() once watchers are live).
            self.pulse.update(ClusterSummary::default());
            self.mode = Mode::Pulse;
            return;
        }

        if matches!(alias, "workload" | "wl" | "workloads") {
            self.history.push(alias);
            self.workload = WorkloadView::new();
            self.mode = Mode::Workload;
            return;
        }

        if matches!(alias, "alias" | "aliases") {
            self.history.push(alias);
            self.browser = Some(crate::view::alias_browser(&self.registry));
            return;
        }

        if let Some(view) = crate::view::browser_for_resource(alias, &self.registry) {
            self.history.push(alias);
            self.browser = Some(view);
        } else {
            self.flash(format!("Unknown resource: {alias}"), Duration::from_secs(3));
        }
    }

    /// Navigate using the resolved alias stored in history without re-pushing.
    fn navigate_no_push(&mut self, alias: &str) {
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

        // React to config file changes — reload and apply.
        let config_changed = self
            .config_reload_rx
            .as_mut()
            .map(|rx| rx.try_recv().is_ok())
            .unwrap_or(false);
        if config_changed {
            // Drain any further pending signals (debounce rapid successive writes).
            while self
                .config_reload_rx
                .as_mut()
                .map(|rx| rx.try_recv().is_ok())
                .unwrap_or(false)
            {}
            if let Some(path) = self.config_path.clone() {
                match crate::config::load(&path) {
                    Ok(new_cfg) => {
                        tracing::info!(path = %path.display(), "config reloaded");
                        self.config = new_cfg;
                        self.flash("Config reloaded".to_owned(), Duration::from_secs(2));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "config reload failed");
                        self.flash(format!("Config reload error: {e}"), Duration::from_secs(4));
                    }
                }
            }
        }

        // Drain pending operation results.
        while let Ok(result) = self.op_result_rx.try_recv() {
            match result {
                OpResult::Ok(msg) => self.flash(msg, Duration::from_secs(3)),
                OpResult::Err(e) => self.flash(format!("Error: {e}"), Duration::from_secs(5)),
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
    // ── Help overlay: consumes all keys ──────────────────────────────────────
    if app.mode == Mode::Help {
        let action = app.help.handle_key(&key);
        if action == HelpAction::Close {
            app.mode = Mode::Browse;
        }
        return;
    }

    // ── Pulse view ────────────────────────────────────────────────────────────
    if app.mode == Mode::Pulse {
        let action = app.pulse.handle_key(&key);
        if action == PulseAction::Close {
            app.mode = Mode::Browse;
        }
        return;
    }

    // ── Workload view ─────────────────────────────────────────────────────────
    if app.mode == Mode::Workload {
        let action = app.workload.handle_key(&key);
        if action == WorkloadAction::Close {
            app.mode = Mode::Browse;
        }
        return;
    }

    // ── Log view ─────────────────────────────────────────────────────────────
    if app.mode == Mode::Log {
        let action = app.log.handle_key(key);
        match action {
            LogAction::Close => app.mode = Mode::Browse,
            LogAction::SwitchContainer(name) => {
                app.log.pod_name = app.log.pod_name.clone();
                app.flash(
                    format!("Switching to container: {name}"),
                    Duration::from_secs(2),
                );
                // Future: re-stream logs for `name` when cluster is connected.
            }
            LogAction::None => {}
        }
        return;
    }

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
    // ── Port-forward dialog ───────────────────────────────────────────────────
    if app.mode == Mode::PortForward {
        if let Some(dlg) = &mut app.pf_dialog {
            match dlg.handle_key(key.code) {
                PortForwardAction::Confirm(pod_port, local_port) => {
                    let target = app.pending_pf.take();
                    app.pf_dialog = None;
                    app.mode = Mode::Browse;
                    if let Some((ns, pod)) = target {
                        match app.pf_manager.add(&ns, &pod, pod_port, local_port) {
                            Ok(id) => app.flash(
                                format!("Port-forward started: {id}  127.0.0.1:{local_port} → {pod}:{pod_port}"),
                                Duration::from_secs(4),
                            ),
                            Err(e) => app.flash(
                                format!("Port-forward failed: {e}"),
                                Duration::from_secs(5),
                            ),
                        }
                    }
                }
                PortForwardAction::Cancel => {
                    app.pf_dialog = None;
                    app.pending_pf = None;
                    app.mode = Mode::Browse;
                }
                PortForwardAction::None => {}
            }
        }
        return;
    }

    // ── Image update dialog ───────────────────────────────────────────────────
    if app.mode == Mode::ImageUpdate {
        if let Some(dlg) = &mut app.image_dialog {
            match dlg.handle_key(key.code) {
                ImageUpdateAction::Confirm(container, new_image) => {
                    let target = app.pending_image.take();
                    app.image_dialog = None;
                    app.mode = Mode::Browse;
                    if let Some((resource, ns, name)) = target {
                        let update = crate::exec::ImageUpdate::new(
                            &resource, &name, &ns, &container, &new_image,
                        );
                        let result = update.run();
                        if result.exit_code == Some(0) {
                            app.flash(
                                format!("Updated {resource}/{name} {container}={new_image}"),
                                Duration::from_secs(3),
                            );
                        } else {
                            app.flash(
                                format!("Image update failed (exit {:?})", result.exit_code),
                                Duration::from_secs(4),
                            );
                        }
                    }
                }
                ImageUpdateAction::Cancel => {
                    app.image_dialog = None;
                    app.pending_image = None;
                    app.mode = Mode::Browse;
                }
                ImageUpdateAction::None => {}
            }
        }
        return;
    }

    // ── Confirm dialog ────────────────────────────────────────────────────────
    if app.mode == Mode::Confirm {
        if let Some(dlg) = &app.confirm_dialog {
            match dlg.handle_key(key.code) {
                ConfirmAction::Yes => {
                    let target = app.pending_delete.take();
                    app.confirm_dialog = None;
                    app.mode = Mode::Browse;
                    if let Some((resource, ns, name)) = target {
                        if let Some(client) = app.kube_client.clone() {
                            let tx = app.op_result_tx.clone();
                            let gvr = crate::client::Gvr {
                                group: String::new(),
                                version: "v1".to_owned(),
                                resource: resource.clone(),
                            };
                            let name2 = name.clone();
                            tokio::spawn(async move {
                                let result = crate::dao::ops::delete_resource(
                                    client,
                                    &gvr,
                                    ns.as_deref(),
                                    &name2,
                                )
                                .await;
                                let _ = tx
                                    .send(match result {
                                        Ok(_) => {
                                            OpResult::Ok(format!("Deleted {resource}/{name2}"))
                                        }
                                        Err(e) => OpResult::Err(e.to_string()),
                                    })
                                    .await;
                            });
                        } else {
                            app.flash(
                                format!("Delete {resource}/{name}: no cluster connection"),
                                Duration::from_secs(3),
                            );
                        }
                    }
                }
                ConfirmAction::No => {
                    app.confirm_dialog = None;
                    app.pending_delete = None;
                    app.mode = Mode::Browse;
                }
                ConfirmAction::None => {}
            }
        }
        return;
    }

    // ── Scale dialog ──────────────────────────────────────────────────────────
    if app.mode == Mode::Scale {
        if let Some(dlg) = &mut app.scale_dialog {
            match dlg.handle_key(key.code) {
                ScaleAction::Confirm(replicas) => {
                    let target = app.pending_scale.take();
                    app.scale_dialog = None;
                    app.mode = Mode::Browse;
                    if let Some((gvr, ns, name)) = target {
                        if let Some(client) = app.kube_client.clone() {
                            let tx = app.op_result_tx.clone();
                            let name2 = name.clone();
                            let res = gvr.resource.clone();
                            tokio::spawn(async move {
                                let result = crate::dao::ops::scale_resource(
                                    client,
                                    &gvr,
                                    &ns,
                                    &name2,
                                    replicas as i32,
                                )
                                .await;
                                let _ = tx
                                    .send(match result {
                                        Ok(_) => OpResult::Ok(format!(
                                            "Scaled {res}/{name2} to {replicas}"
                                        )),
                                        Err(e) => OpResult::Err(e.to_string()),
                                    })
                                    .await;
                            });
                        } else {
                            app.flash(
                                format!("Scale {}/{name}: no cluster connection", gvr.resource),
                                Duration::from_secs(3),
                            );
                        }
                    }
                }
                ScaleAction::Cancel => {
                    app.scale_dialog = None;
                    app.pending_scale = None;
                    app.mode = Mode::Browse;
                }
                ScaleAction::None => {}
            }
        }
        return;
    }

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
                    // `:ctx <name>` — record intent and show flash.
                    // Actual reconnection happens in a future phase when the
                    // K8s client layer is wired in; for now we surface the
                    // selection so the user can see which context was chosen.
                    app.flash(
                        format!("Context selected: {ctx} (reconnect on next tick)"),
                        Duration::from_secs(3),
                    );
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
        Action::Help => {
            app.help = HelpView::new(); // reset scroll
            app.mode = Mode::Help;
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
        // ── History navigation ────────────────────────────────────────────────
        Action::HistoryBack => {
            let alias = app.history.back().map(|s| s.to_owned());
            if let Some(a) = alias {
                app.navigate_no_push(&a);
            } else {
                app.flash("Already at earliest history entry", Duration::from_secs(2));
            }
        }
        Action::HistoryForward => {
            let alias = app.history.forward().map(|s| s.to_owned());
            if let Some(a) = alias {
                app.navigate_no_push(&a);
            } else {
                app.flash(
                    "Already at most recent history entry",
                    Duration::from_secs(2),
                );
            }
        }
        Action::HistoryLast => {
            let alias = app.history.last().map(|s| s.to_owned());
            if let Some(a) = alias {
                app.navigate_no_push(&a);
            } else {
                app.flash("No previous resource to return to", Duration::from_secs(2));
            }
        }
        // ── Chat ──────────────────────────────────────────────────────────────
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
        Action::Enter => {
            // Drill into sub-resource: Enter on a pod row opens the container view.
            use crate::client::gvr::well_known;
            if let Some(browser) = &app.browser {
                let is_pods = browser.resource_gvr.as_ref() == Some(&well_known::pods());
                if is_pods {
                    if let Some(pod_value) = browser.selected_value() {
                        let container_view = crate::view::container_browser(&pod_value);
                        app.history.push("containers");
                        app.browser = Some(container_view);
                    }
                }
            }
        }
        Action::Logs => {
            // Open the log view for the selected pod.
            if let Some(browser) = &app.browser {
                use crate::client::gvr::well_known;
                let is_pods = browser.resource_gvr.as_ref() == Some(&well_known::pods());
                if is_pods {
                    if let Some(pod_value) = browser.selected_value() {
                        // Extract container names from pod spec.
                        let containers: Vec<String> = pod_value
                            .pointer("/spec/containers")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                                    .map(|s| s.to_owned())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let pod_name = pod_value
                            .pointer("/metadata/name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("pod")
                            .to_owned();
                        app.log = LogView::new(pod_name, containers);
                        app.mode = Mode::Log;
                    } else {
                        app.flash(
                            "Select a pod to view logs".to_owned(),
                            Duration::from_secs(2),
                        );
                    }
                } else {
                    app.flash(
                        "Logs only available for pods".to_owned(),
                        Duration::from_secs(2),
                    );
                }
            }
        }
        Action::Describe => {
            if let Some(name) = app.browser.as_ref().and_then(|b| b.selected_name()) {
                app.flash(
                    format!("Describe: {name} (Phase 6)"),
                    Duration::from_secs(3),
                );
            }
        }
        Action::Delete => {
            if let Some(browser) = &app.browser {
                if let Some(name) = browser.selected_name() {
                    let ns = browser
                        .selected_namespace()
                        .or_else(|| app.namespace.clone());
                    let resource = browser
                        .resource_gvr
                        .as_ref()
                        .map(|g| g.resource.clone())
                        .unwrap_or_else(|| "resource".to_owned());
                    let msg = if let Some(ref n) = ns {
                        format!("Delete {resource}/{name} from namespace {n}?")
                    } else {
                        format!("Delete {resource}/{name}?")
                    };
                    app.confirm_dialog = Some(ConfirmDialog::new("Confirm Delete", msg));
                    app.pending_delete = Some((resource, ns, name));
                    app.mode = Mode::Confirm;
                } else {
                    app.flash(
                        "Select a resource to delete".to_owned(),
                        Duration::from_secs(2),
                    );
                }
            }
        }
        Action::Scale => {
            if let Some(browser) = &app.browser {
                if let Some(name) = browser.selected_name() {
                    let gvr = browser.resource_gvr.clone();
                    let ns = browser
                        .selected_namespace()
                        .or_else(|| app.namespace.clone())
                        .unwrap_or_default();
                    // Pre-fill current replica count if available in the row.
                    let current = browser
                        .selected_value()
                        .and_then(|v| v.pointer("/spec/replicas").and_then(|r| r.as_u64()))
                        .unwrap_or(1) as u32;
                    app.scale_dialog = Some(ScaleDialog::new(&name, current));
                    app.pending_scale = gvr.map(|g| (g, ns, name));
                    app.mode = Mode::Scale;
                } else {
                    app.flash(
                        "Select a workload to scale".to_owned(),
                        Duration::from_secs(2),
                    );
                }
            }
        }
        Action::SetImage => {
            if let Some(browser) = &app.browser {
                if let Some(name) = browser.selected_name() {
                    let ns = browser
                        .selected_namespace()
                        .or_else(|| app.namespace.clone())
                        .unwrap_or_else(|| "default".to_owned());
                    let resource = browser
                        .resource_gvr
                        .as_ref()
                        .map(|g| g.resource.clone())
                        .unwrap_or_else(|| "deployment".to_owned());
                    // Best-effort: read current image from selected value.
                    let (container, current_image) = browser
                        .selected_value()
                        .and_then(|v| {
                            let c = v.pointer("/spec/containers/0")?;
                            let cname = c.get("name")?.as_str()?.to_owned();
                            let img = c.get("image")?.as_str()?.to_owned();
                            Some((cname, img))
                        })
                        .unwrap_or_else(|| ("app".to_owned(), String::new()));
                    // For workloads the template containers are nested.
                    let (container, current_image) =
                        if container == "app" && current_image.is_empty() {
                            browser
                                .selected_value()
                                .and_then(|v| {
                                    let c = v.pointer("/spec/template/spec/containers/0")?;
                                    let cname = c.get("name")?.as_str()?.to_owned();
                                    let img = c.get("image")?.as_str()?.to_owned();
                                    Some((cname, img))
                                })
                                .unwrap_or((container, current_image))
                        } else {
                            (container, current_image)
                        };
                    app.image_dialog =
                        Some(ImageUpdateDialog::new(&name, container, current_image));
                    app.pending_image = Some((resource, ns, name));
                    app.mode = Mode::ImageUpdate;
                } else {
                    app.flash(
                        "Select a workload to update its image".to_owned(),
                        Duration::from_secs(2),
                    );
                }
            }
        }
        Action::PortForward => {
            if let Some(browser) = &app.browser {
                if let Some(name) = browser.selected_name() {
                    let ns = browser
                        .selected_namespace()
                        .or_else(|| app.namespace.clone())
                        .unwrap_or_else(|| "default".to_owned());
                    // Pick the first container port from the pod spec, default 8080.
                    let pod_port = browser
                        .selected_value()
                        .and_then(|v| {
                            v.pointer("/spec/containers/0/ports/0/containerPort")
                                .and_then(|p| p.as_u64())
                        })
                        .unwrap_or(8080) as u16;
                    app.pf_dialog = Some(PortForwardDialog::new(&name, pod_port));
                    app.pending_pf = Some((ns, name));
                    app.mode = Mode::PortForward;
                } else {
                    app.flash(
                        "Select a pod to port-forward".to_owned(),
                        Duration::from_secs(2),
                    );
                }
            }
        }
        Action::Restart => {
            if let Some(browser) = &app.browser {
                if let Some(name) = browser.selected_name() {
                    let gvr = browser.resource_gvr.clone();
                    let ns = browser
                        .selected_namespace()
                        .or_else(|| app.namespace.clone())
                        .unwrap_or_default();
                    if let (Some(gvr), Some(client)) = (gvr, app.kube_client.clone()) {
                        let tx = app.op_result_tx.clone();
                        let name2 = name.clone();
                        let res = gvr.resource.clone();
                        tokio::spawn(async move {
                            let result =
                                crate::dao::ops::restart_resource(client, &gvr, &ns, &name2).await;
                            let _ = tx
                                .send(match result {
                                    Ok(_) => OpResult::Ok(format!("Restarted {res}/{name2}")),
                                    Err(e) => OpResult::Err(e.to_string()),
                                })
                                .await;
                        });
                        app.flash(format!("Restarting {name}…"), Duration::from_secs(2));
                    } else {
                        app.flash(
                            "Restart requires a cluster connection".to_owned(),
                            Duration::from_secs(3),
                        );
                    }
                } else {
                    app.flash(
                        "Select a workload to restart".to_owned(),
                        Duration::from_secs(2),
                    );
                }
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

    // Help overlay draws on top of everything else.
    if app.mode == Mode::Help {
        app.help.render(frame, area);
    }

    // Dialog overlays draw on top of the browser.
    if app.mode == Mode::Confirm {
        if let Some(dlg) = &app.confirm_dialog {
            dlg.render(frame, area);
        }
    }
    if app.mode == Mode::Scale {
        if let Some(dlg) = &app.scale_dialog {
            dlg.render(frame, area);
        }
    }
    if app.mode == Mode::PortForward {
        if let Some(dlg) = &app.pf_dialog {
            dlg.render(frame, area);
        }
    }
    if app.mode == Mode::ImageUpdate {
        if let Some(dlg) = &app.image_dialog {
            dlg.render(frame, area);
        }
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(45)])
        .split(area);

    // Build breadcrumb trail from navigation history (last 4 entries).
    let trail = app.history.trail(4);
    let breadcrumbs = if trail.len() <= 1 {
        app.browser
            .as_ref()
            .map(|b| b.title.clone())
            .unwrap_or_else(|| "pods".to_owned())
    } else {
        trail.join(" › ")
    };

    let ns_label = app.namespace.as_deref().unwrap_or("(all)");

    let mode_tag = match &app.mode {
        Mode::Chat => "  [chat]",
        Mode::Help => "  [help]",
        Mode::Command => "  [cmd]",
        Mode::Pulse => "  [pulse]",
        Mode::Workload => "  [workload]",
        Mode::Log => "  [logs]",
        Mode::Confirm => "  [delete?]",
        Mode::Scale => "  [scale]",
        Mode::PortForward => "  [port-forward]",
        Mode::ImageUpdate => "  [set-image]",
        Mode::Browse => "",
    };

    // History navigation indicators.
    let back_indicator = if app.history.can_go_back() {
        "‹"
    } else {
        " "
    };
    let fwd_indicator = if app.history.can_go_forward() {
        "›"
    } else {
        " "
    };

    let title = Paragraph::new(format!(
        " k7s  {}  {back_indicator} {breadcrumbs} {fwd_indicator}  ns:{ns_label}{mode_tag}",
        env!("CARGO_PKG_VERSION"),
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
    match app.mode {
        Mode::Chat => {
            // Split: browser (left 55%) | chat (right 45%)
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(area);
            if let Some(browser) = &mut app.browser {
                browser.render(frame, split[0]);
            } else {
                crate::ui::splash::render_splash(frame, split[0], env!("CARGO_PKG_VERSION"));
            }
            app.chat.render(frame, split[1]);
        }
        Mode::Pulse => {
            app.pulse.render(frame, area);
        }
        Mode::Workload => {
            app.workload.render(frame, area);
        }
        Mode::Log => {
            app.log.render(frame, area);
        }
        _ => {
            if let Some(browser) = &mut app.browser {
                browser.render(frame, area);
            } else {
                crate::ui::splash::render_splash(frame, area, env!("CARGO_PKG_VERSION"));
            }
        }
    }
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if app.mode == Mode::Help {
        frame.render_widget(
            Paragraph::new("  ↑↓/jk scroll  g top  q/Esc close")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if app.mode == Mode::Pulse || app.mode == Mode::Workload {
        frame.render_widget(
            Paragraph::new("  q/Esc close").style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if app.mode == Mode::Log {
        frame.render_widget(
            Paragraph::new("  ↑↓ scroll  / filter  t timestamps  c containers  q close")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if app.mode == Mode::Confirm {
        frame.render_widget(
            Paragraph::new("  y/Enter confirm   n/Esc cancel")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if app.mode == Mode::Scale {
        frame.render_widget(
            Paragraph::new("  0-9 set replicas   Enter confirm   Esc cancel")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if app.mode == Mode::PortForward {
        frame.render_widget(
            Paragraph::new("  Tab switch field   0-9 type port   Enter start   Esc cancel")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if app.mode == Mode::ImageUpdate {
        frame.render_widget(
            Paragraph::new("  Type image ref   ←→ move cursor   Enter confirm   Esc cancel")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if let Some(prompt_text) = app.prompt.display() {
        // Split footer: prompt on left, suggestions on right.
        let chunks = ratatui::layout::Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(0)])
            .split(area);

        frame.render_widget(
            Paragraph::new(prompt_text).style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            chunks[0],
        );

        let hint = app.prompt.suggestion_hint(8);
        if !hint.is_empty() {
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
                chunks[1],
            );
        }
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
    fn navigate_pushes_to_history() {
        let mut app = App::new(Config::default());
        app.navigate("pods");
        app.navigate("nodes");
        assert!(app.history.can_go_back());
    }

    #[test]
    fn history_back_navigates() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = App::new(Config::default());
        app.navigate("pods");
        app.navigate("nodes");
        // Press [ to go back.
        handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Char('['),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        );
        // Browser should now show pods.
        assert_eq!(app.browser.as_ref().unwrap().title, "Pods");
    }

    #[test]
    fn question_mark_opens_help() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = App::new(Config::default());
        handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Char('?'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        );
        assert_eq!(app.mode, Mode::Help);
    }

    #[test]
    fn esc_in_help_returns_to_browse() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = App::new(Config::default());
        app.mode = Mode::Help;
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
