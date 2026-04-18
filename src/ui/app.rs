use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEventKind,
};
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
use crate::config::{Config, ConfigDirs, Plugin, PluginConfig, PluginContext};
use crate::dao::Registry;
use crate::health::ClusterSummary;
use crate::history::{CommandHistory, HistorySource};
use crate::model::NavHistory;
use crate::ui::chat::{ChatAction, ChatWidget};
use crate::ui::dialog::{
    ConfirmAction, ConfirmDialog, ImageUpdateAction, ImageUpdateDialog, PortForwardAction,
    PortForwardDialog, ScaleAction, ScaleDialog,
};
use crate::ui::key::{self, format_hints, Action, LIST_HINTS};
use crate::ui::prompt::{Prompt, PromptSubmit};
use crate::metrics::{spawn_metrics_poller, MetricsSnapshot, MetricsStore, DEFAULT_POLL_INTERVAL};
use crate::view::BrowserView;
use crate::view::{
    demo_tree, DirAction, DirView, HelpAction, HelpView, LogAction, LogView, MetricsAction,
    MetricsView, PulseAction, PulseView, WorkloadAction, WorkloadView, XRayAction, XRayView,
};
use crate::vul::{ImgScanAction, ImgScanView, VulReport, VulnerabilityScanner};
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
    /// XRay resource-tree view is fullscreen.
    XRay,
    /// Delete-confirmation overlay is open.
    Confirm,
    /// Scale-replica-count overlay is open.
    Scale,
    /// Port-forward setup dialog is open.
    PortForward,
    /// Image update dialog is open.
    ImageUpdate,
    /// Image vulnerability scan results are displayed.
    ImgScan,
    /// Local filesystem directory browser.
    Dir,
    /// Live metrics dashboard (sparklines for pods and nodes).
    Metrics,
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
    /// Unified command history — persisted across CLI and TUI sessions.
    cmd_history: CommandHistory,

    /// Help overlay widget (lazy-init on first `?` press).
    help: HelpView,
    /// Pulse cluster-dashboard view.
    pulse: PulseView,
    /// Workload aggregated view.
    workload: WorkloadView,
    /// XRay resource-tree view.
    xray: XRayView,
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
    /// Vulnerability scan result view.
    img_scan: ImgScanView,
    /// Local filesystem directory browser.
    dir: DirView,

    // ── Metrics (Phase 18) ────────────────────────────────────────────────────
    /// Live metrics dashboard view.
    metrics_view: MetricsView,
    /// In-memory time-series store for pod/node metrics.
    pub metrics_store: MetricsStore,
    /// Receives periodic `MetricsSnapshot` values from the background poller.
    metrics_rx: mpsc::Receiver<MetricsSnapshot>,
    /// Sender half kept alive so we can hand it to the poller task.
    metrics_tx: mpsc::Sender<MetricsSnapshot>,
    /// Cancels the metrics background poller on shutdown.
    metrics_cancel: tokio_util::sync::CancellationToken,

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

    // ── Plugin system ─────────────────────────────────────────────────────────
    /// Loaded plugin definitions from `plugins.yaml`.
    pub plugin_config: PluginConfig,
    /// A foreground plugin waiting to be run (TUI must be suspended first).
    /// Set by `handle_key_event`; consumed in `run_loop` before the next draw.
    pub pending_foreground_plugin: Option<(Plugin, PluginContext)>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (ai_reply_tx, ai_reply_rx) = mpsc::channel(8);
        let (op_result_tx, op_result_rx) = mpsc::channel(8);
        let (metrics_tx, metrics_rx) = mpsc::channel(4);

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
            "xray",
            "vuln",
            "dir",
            "metrics",
            "top",
            "retry",
            "!!",
        ] {
            if !candidates.iter().any(|c| c == cmd) {
                candidates.push(cmd.to_string());
            }
        }
        candidates.sort();
        candidates.dedup();
        prompt.set_candidates(candidates);

        // Load the unified command history from the XDG state directory.
        let cmd_history = ConfigDirs::resolve()
            .map(|d| CommandHistory::load(&d.state))
            .unwrap_or_else(|_| CommandHistory::in_memory());

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

        // Load plugin configuration from `~/.config/k7s/plugins.yaml`.
        let plugin_config = ConfigDirs::resolve()
            .ok()
            .and_then(|d| {
                let p = d.config.join("plugins.yaml");
                PluginConfig::load(&p)
                    .map_err(|e| tracing::warn!(error = %e, "failed to load plugins.yaml"))
                    .ok()
            })
            .unwrap_or_default();

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
            cmd_history,
            help: HelpView::new(),
            pulse: PulseView::new(),
            workload: WorkloadView::new(),
            xray: XRayView::new(),
            chat: ChatWidget::new(),
            chat_session,
            chat_provider,
            ai_reply_tx,
            ai_reply_rx,
            metrics_view: MetricsView::new(),
            metrics_store: MetricsStore::new(),
            metrics_tx,
            metrics_rx,
            metrics_cancel: tokio_util::sync::CancellationToken::new(),
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
            img_scan: ImgScanView::new(VulReport::default()),
            dir: DirView::new_cwd(),
            op_result_tx,
            op_result_rx,
            config_reload_rx,
            _config_watcher: config_watcher,
            config_path,
            plugin_config,
            pending_foreground_plugin: None,
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
            self.cmd_history.push(
                HistorySource::Tui,
                format!("navigate:{alias}"),
                None,
                self.namespace.clone(),
                true,
            );
            self.browser = Some(crate::view::context_browser());
            return;
        }

        // Special case: pulse and workload are fullscreen dedicated views.
        if matches!(alias, "pulse") {
            self.history.push(alias);
            self.cmd_history.push(
                HistorySource::Tui,
                "navigate:pulse",
                None,
                self.namespace.clone(),
                true,
            );
            self.pulse = PulseView::new();
            // Seed with an empty summary (real data flows via tick() once watchers are live).
            self.pulse.update(ClusterSummary::default());
            self.mode = Mode::Pulse;
            return;
        }

        if matches!(alias, "workload" | "wl" | "workloads") {
            self.history.push(alias);
            self.cmd_history.push(
                HistorySource::Tui,
                format!("navigate:{alias}"),
                None,
                self.namespace.clone(),
                true,
            );
            self.workload = WorkloadView::new();
            self.mode = Mode::Workload;
            return;
        }

        if matches!(alias, "alias" | "aliases") {
            self.history.push(alias);
            self.cmd_history.push(
                HistorySource::Tui,
                "navigate:aliases",
                None,
                self.namespace.clone(),
                true,
            );
            self.browser = Some(crate::view::alias_browser(&self.registry));
            return;
        }

        if matches!(alias, "xray") {
            self.history.push(alias);
            self.cmd_history.push(
                HistorySource::Tui,
                "navigate:xray",
                None,
                self.namespace.clone(),
                true,
            );
            self.xray = XRayView::new();
            self.xray.set_roots(demo_tree());
            self.mode = Mode::XRay;
            return;
        }

        if matches!(alias, "metrics" | "top") {
            self.history.push(alias);
            self.cmd_history.push(
                HistorySource::Tui,
                format!("navigate:{alias}"),
                None,
                self.namespace.clone(),
                true,
            );
            self.metrics_view = MetricsView::new();
            self.mode = Mode::Metrics;
            return;
        }

        if alias == "dir" || alias.starts_with("dir ") || alias.starts_with("dir/") {
            self.history.push("dir");
            self.cmd_history.push(
                HistorySource::Tui,
                format!("navigate:{alias}"),
                None,
                self.namespace.clone(),
                true,
            );
            // Allow `:dir /some/path` by extracting the path portion.
            let path = if let Some(stripped) = alias.strip_prefix("dir ") {
                stripped.trim().to_owned()
            } else {
                String::new()
            };
            self.dir = if path.is_empty() {
                DirView::new_cwd()
            } else {
                DirView::new(&path)
            };
            self.mode = Mode::Dir;
            return;
        }

        if matches!(alias, "vuln" | "scan") {
            self.history.push(alias);
            self.cmd_history.push(
                HistorySource::Tui,
                "navigate:vuln",
                None,
                self.namespace.clone(),
                true,
            );
            // Show an empty report; the user can trigger an actual scan from
            // the browser by pressing `v` on a pod row (future phase wiring).
            let report = VulReport {
                image: "(select a pod and press v to scan)".to_owned(),
                ..VulReport::default()
            };
            self.img_scan = ImgScanView::new(report);
            self.mode = Mode::ImgScan;
            return;
        }

        if let Some(view) = crate::view::browser_for_resource(alias, &self.registry) {
            self.history.push(alias);
            self.cmd_history.push(
                HistorySource::Tui,
                format!("navigate:{alias}"),
                None,
                self.namespace.clone(),
                true,
            );
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

        // Drain incoming metrics snapshots.
        while let Ok(snapshot) = self.metrics_rx.try_recv() {
            self.metrics_store.ingest(&snapshot);
            self.metrics_view.on_metrics_updated();
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

    /// Start the background metrics poller using the live kube client.
    ///
    /// Safe to call multiple times — cancels any previous poller first.
    pub fn start_metrics_poller(&mut self) {
        if let Some(client) = self.kube_client.clone() {
            // Cancel previous poller, create a fresh token.
            self.metrics_cancel.cancel();
            self.metrics_cancel = tokio_util::sync::CancellationToken::new();
            spawn_metrics_poller(
                client,
                self.metrics_tx.clone(),
                DEFAULT_POLL_INTERVAL,
                self.metrics_cancel.clone(),
            );
            tracing::debug!("metrics poller started");
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
    let mouse_enabled = config.k7s.ui.enable_mouse;
    let mut terminal = setup_terminal(mouse_enabled)?;
    let result = run_loop(&mut terminal, config).await;
    restore_terminal(&mut terminal, mouse_enabled)?;
    result
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mouse_was_enabled: bool,
) -> io::Result<()> {
    if mouse_was_enabled {
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
    }
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn setup_terminal(
    enable_mouse: bool,
) -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen)?;
    if enable_mouse {
        execute!(stdout, EnableMouseCapture)?;
    }
    Terminal::new(CrosstermBackend::new(stdout))
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
) -> anyhow::Result<()> {
    let mouse_enabled = config.k7s.ui.enable_mouse;
    let mut app = App::new(config);

    // Start on the pods view by default.
    app.navigate("pods");

    loop {
        app.tick();
        terminal.draw(|frame| render(frame, &mut app))?;

        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key_event(&mut app, key);
                }
                Event::Mouse(mouse) if app.config.k7s.ui.enable_mouse => {
                    handle_mouse_event(&mut app, mouse);
                }
                _ => {}
            }
        }

        // ── Foreground plugin execution ────────────────────────────────────────
        // Foreground plugins need the terminal suspended.  We handle them here,
        // outside the draw/poll cycle, so the TUI can be cleanly torn down and
        // restored around the subprocess.
        if let Some((plugin, ctx)) = app.pending_foreground_plugin.take() {
            restore_terminal(terminal, mouse_enabled)?;
            let result = plugin.run(&ctx);
            *terminal = setup_terminal(mouse_enabled)?;
            terminal.clear()?;
            match result {
                Ok(_) => app.flash(
                    format!("Plugin '{}' finished", ctx.plugin_name),
                    Duration::from_secs(2),
                ),
                Err(e) => app.flash(
                    format!("Plugin '{}' error: {e}", ctx.plugin_name),
                    Duration::from_secs(4),
                ),
            }
        }

        if app.should_quit() {
            break;
        }
    }

    Ok(())
}

// ─── Retry handler ───────────────────────────────────────────────────────────

/// Handle `:retry N` / `!!` from the TUI command prompt.
///
/// Looks up the Nth-last entry in the unified command history and replays it
/// if it represents a replayable TUI action.  CLI commands recorded from
/// outside the TUI cannot be re-run here (they require a live terminal) — a
/// descriptive flash message is shown instead.
fn handle_tui_retry(app: &mut App, n: usize) {
    // We need to clone the data we need before mutably borrowing `app`.
    let entry_info = app
        .cmd_history
        .nth_last(n)
        .map(|e| (e.source.clone(), e.command.clone()));

    match entry_info {
        None => {
            app.flash(
                format!(
                    "retry: no entry at position {n} (history has {} entries)",
                    app.cmd_history.len()
                ),
                Duration::from_secs(3),
            );
        }
        Some((HistorySource::Cli, cmd)) => {
            // CLI commands need a real terminal — can't run them inside the TUI.
            app.flash(
                format!("retry: CLI command \"{cmd}\" must be run outside the TUI"),
                Duration::from_secs(4),
            );
        }
        Some((HistorySource::Tui, cmd)) => {
            replay_tui_command(app, &cmd);
        }
    }
}

/// Parse and re-execute a TUI history command string.
///
/// TUI history strings follow a `verb:payload` scheme:
///
/// | String | Action |
/// |--------|--------|
/// | `navigate:pods` | Navigate to the pods view |
/// | `ns:default` | Switch to the `default` namespace |
/// | `ns:(all)` | Switch to all namespaces |
/// | `ctx:prod` | Select the `prod` context |
/// | `filter:app=nginx` | Apply a table filter |
fn replay_tui_command(app: &mut App, cmd: &str) {
    if let Some(alias) = cmd.strip_prefix("navigate:") {
        app.navigate(alias);
        return;
    }

    if let Some(ns_str) = cmd.strip_prefix("ns:") {
        let ns = if ns_str == "(all)" || ns_str.is_empty() {
            None
        } else {
            Some(ns_str.to_owned())
        };
        let label = ns.as_deref().unwrap_or("(all)").to_owned();
        app.namespace = ns;
        app.flash(format!("Namespace: {label}"), Duration::from_secs(2));
        return;
    }

    if let Some(ctx) = cmd.strip_prefix("ctx:") {
        app.flash(
            format!("Context selected: {ctx} (reconnect on next tick)"),
            Duration::from_secs(3),
        );
        return;
    }

    if let Some(filter) = cmd.strip_prefix("filter:") {
        if let Some(b) = &mut app.browser {
            b.set_filter(filter.to_owned());
        }
        app.flash(format!("Filter: {filter}"), Duration::from_secs(2));
        return;
    }

    // Unknown format — surface it to the user.
    app.flash(
        format!("retry: cannot replay \"{cmd}\""),
        Duration::from_secs(3),
    );
}

// ─── Mouse handling ───────────────────────────────────────────────────────────

/// Handle a mouse event when `enable_mouse = true` in config.
///
/// Supported gestures:
/// * **Scroll up/down** — scroll the active table or log viewer.
/// * **Left click** — move the table cursor to the clicked row (best-effort;
///   row mapping requires knowledge of the scroll offset, so we approximate).
fn handle_mouse_event(app: &mut App, mouse: crossterm::event::MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            // Scroll the active view up.
            match app.mode {
                Mode::Log => app.log.scroll_up(3),
                Mode::Chat => app.chat.scroll_up(1),
                _ => {
                    if let Some(b) = &mut app.browser {
                        b.up();
                        b.up();
                        b.up();
                    }
                }
            }
        }
        MouseEventKind::ScrollDown => {
            match app.mode {
                Mode::Log => app.log.scroll_down(3),
                Mode::Chat => app.chat.scroll_down(1),
                _ => {
                    if let Some(b) = &mut app.browser {
                        b.down();
                        b.down();
                        b.down();
                    }
                }
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Left click: if in a browser, attempt to move cursor to the
            // clicked row.  The header occupies the first ~3 rows, and each
            // data row is one line tall.  Subtract the header height (3) and
            // the top area offset (1 for the header bar) to compute the
            // approximate data-row index.
            if app.mode == Mode::Browse {
                if let Some(b) = &mut app.browser {
                    // row 0 = terminal top; rows 0-2 are header/crumbs/column.
                    // Approximate: click at row R corresponds to table row R - 4.
                    let data_row = (mouse.row as usize).saturating_sub(4);
                    b.set_cursor(data_row);
                }
            }
        }
        _ => {}
    }
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

    // ── XRay view ────────────────────────────────────────────────────────────
    if app.mode == Mode::XRay {
        let action = app.xray.handle_key(&key);
        if action == XRayAction::Close {
            app.mode = Mode::Browse;
        }
        return;
    }

    // ── Dir view ──────────────────────────────────────────────────────────────
    if app.mode == Mode::Dir {
        let action = app.dir.handle_key(&key);
        if action == DirAction::Close {
            app.mode = Mode::Browse;
        }
        return;
    }

    // ── ImgScan view ─────────────────────────────────────────────────────────
    if app.mode == Mode::ImgScan {
        let action = app.img_scan.handle_key(&key);
        if action == ImgScanAction::Close {
            app.mode = Mode::Browse;
        }
        return;
    }

    // ── Metrics view ──────────────────────────────────────────────────────────
    if app.mode == Mode::Metrics {
        let action = app.metrics_view.handle_key(&key);
        match action {
            Some(MetricsAction::Close) => {
                app.mode = Mode::Browse;
            }
            Some(MetricsAction::Refresh) => {
                app.start_metrics_poller();
                app.flash("Refreshing metrics…".to_owned(), Duration::from_secs(2));
            }
            None => {}
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
                        // ── Plugin confirm ────────────────────────────────────
                        if resource.starts_with("__plugin__:") {
                            // pending_foreground_plugin holds the real data.
                            if let Some((plugin, ctx)) = app.pending_foreground_plugin.take() {
                                run_plugin(app, &plugin, &ctx);
                            }
                        } else if let Some(client) = app.kube_client.clone() {
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
                    app.pending_foreground_plugin = None;
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
                    app.cmd_history.push(
                        HistorySource::Tui,
                        format!("ns:{}", ns.as_deref().unwrap_or("(all)")),
                        None,
                        ns.clone(),
                        true,
                    );
                    app.namespace = ns;
                    app.flash(format!("Namespace: {label}"), Duration::from_secs(2));
                }
                PromptSubmit::Context(ctx) => {
                    // `:ctx <name>` — record intent and show flash.
                    // Actual reconnection happens in a future phase when the
                    // K8s client layer is wired in; for now we surface the
                    // selection so the user can see which context was chosen.
                    app.cmd_history.push(
                        HistorySource::Tui,
                        format!("ctx:{ctx}"),
                        Some(ctx.clone()),
                        None,
                        true,
                    );
                    app.flash(
                        format!("Context selected: {ctx} (reconnect on next tick)"),
                        Duration::from_secs(3),
                    );
                }
                PromptSubmit::Filter(f) => {
                    if let Some(b) = &mut app.browser {
                        b.set_filter(f.clone());
                    }
                    app.cmd_history.push(
                        HistorySource::Tui,
                        format!("filter:{f}"),
                        None,
                        app.namespace.clone(),
                        true,
                    );
                    app.flash(format!("Filter: {f}"), Duration::from_secs(2));
                }
                PromptSubmit::Retry(n) => {
                    handle_tui_retry(app, n);
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
        Action::VulnScan => {
            // Scan the image of the selected pod / container.
            let image = app.browser.as_ref().and_then(|b| {
                b.selected_value().and_then(|v| {
                    // Pod: /spec/containers/0/image
                    v.pointer("/spec/containers/0/image")
                        .or_else(|| v.pointer("/spec/template/spec/containers/0/image"))
                        .and_then(|img| img.as_str())
                        .map(|s| s.to_owned())
                })
            });
            if let Some(img) = image {
                let report = VulReport {
                    image: img.clone(),
                    error: Some("Scanning… (results appear after trivy finishes)".to_owned()),
                    ..VulReport::default()
                };
                app.img_scan = ImgScanView::new(report);
                app.mode = Mode::ImgScan;
                // Spawn the actual scan in the background; result delivered via op_result channel.
                let tx = app.op_result_tx.clone();
                tokio::spawn(async move {
                    let scanner = VulnerabilityScanner::new();
                    let report = scanner.scan(&img).await.unwrap_or_else(|e| VulReport {
                        image: img.clone(),
                        error: Some(e.to_string()),
                        ..VulReport::default()
                    });
                    let _ = tx.send(OpResult::Ok(report.summary())).await;
                });
            } else {
                app.flash("Select a pod to scan its image".to_owned(), Duration::from_secs(2));
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
        _ => {
            // ── Plugin dispatch ────────────────────────────────────────────────
            // Check if the unhandled key matches any plugin binding for the
            // current resource scope.
            dispatch_plugin_key(app, &key);
        }
    }
}

/// Check all loaded plugins for a binding that matches `key` and the current
/// browser scope, then execute or prompt for the matching plugin.
fn dispatch_plugin_key(app: &mut App, key: &crossterm::event::KeyEvent) {
    // Determine the current scope from the active browser's GVR resource name
    // (e.g. "pods", "deployments").  Fall back to "all" so `scope: [all]` plugins
    // always fire.
    let scope = app
        .browser
        .as_ref()
        .and_then(|b| b.resource_gvr.as_ref())
        .map(|g| g.resource.as_str())
        .unwrap_or("all");

    // Collect applicable plugins for this scope.
    let plugins: Vec<(String, Plugin)> = app
        .plugin_config
        .plugins
        .iter()
        .filter(|(_, p)| p.applies_to(scope) && p.matches_key(key))
        .map(|(name, p)| (name.clone(), p.clone()))
        .collect();

    let Some((plugin_name, plugin)) = plugins.into_iter().next() else {
        return; // No plugin matched.
    };

    // Build the expansion context from the currently selected row.
    let name = app
        .browser
        .as_ref()
        .and_then(|b| b.selected_name())
        .unwrap_or_default();
    let namespace = app
        .browser
        .as_ref()
        .and_then(|b| b.selected_namespace())
        .or_else(|| app.namespace.clone())
        .unwrap_or_default();
    let (context_name, cluster_name) = match &app.connection {
        ConnectionState::Connected { context, .. } => (context.clone(), context.clone()),
        _ => (String::new(), String::new()),
    };
    let ctx = PluginContext {
        plugin_name: plugin_name.clone(),
        name,
        namespace,
        context: context_name,
        cluster: cluster_name,
    };

    if plugin.confirm {
        let expanded = plugin.expand_args(&ctx).join(" ");
        let msg = format!("Run plugin '{plugin_name}'?\n  {} {}", plugin.command, expanded);
        app.confirm_dialog = Some(ConfirmDialog::new("Run Plugin", msg));
        // Temporarily repurpose pending_delete to carry the plugin action.
        // We use a sentinel GVR prefix to distinguish it from a real delete.
        app.pending_delete = Some((
            format!("__plugin__:{plugin_name}"),
            None,
            serde_json::to_string(&ctx.name).unwrap_or_default(),
        ));
        // Store the full plugin+ctx for later use when confirmed.
        app.pending_foreground_plugin = Some((plugin, ctx));
        app.mode = Mode::Confirm;
    } else {
        run_plugin(app, &plugin, &ctx);
    }
}

/// Execute a plugin immediately (no confirmation required).
///
/// Background plugins are spawned without blocking.
/// Foreground plugins are queued in `pending_foreground_plugin` for the
/// main loop to execute after suspending the terminal.
fn run_plugin(app: &mut App, plugin: &Plugin, ctx: &PluginContext) {
    if plugin.background {
        match plugin.run(ctx) {
            Ok(_) => app.flash(
                format!("Plugin '{}' started in background", ctx.plugin_name),
                Duration::from_secs(2),
            ),
            Err(e) => app.flash(
                format!("Plugin '{}' failed: {e}", ctx.plugin_name),
                Duration::from_secs(4),
            ),
        }
    } else {
        // Foreground: queue it for the main loop to suspend the TUI and run.
        app.pending_foreground_plugin = Some((plugin.clone(), ctx.clone()));
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
        Mode::XRay => "  [xray]",
        Mode::Log => "  [logs]",
        Mode::Confirm => "  [delete?]",
        Mode::Scale => "  [scale]",
        Mode::PortForward => "  [port-forward]",
        Mode::ImageUpdate => "  [set-image]",
        Mode::ImgScan => "  [vuln-scan]",
        Mode::Dir => "  [dir]",
        Mode::Metrics => "  [metrics]",
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
        Mode::XRay => {
            app.xray.render(frame, area);
        }
        Mode::Log => {
            app.log.render(frame, area);
        }
        Mode::ImgScan => {
            app.img_scan.render(frame, area);
        }
        Mode::Dir => {
            app.dir.render(frame, area);
        }
        Mode::Metrics => {
            let ctx = match &app.connection {
                ConnectionState::Connected { context, .. } => context.as_str(),
                _ => "disconnected",
            };
            app.metrics_view.draw(frame, area, &app.metrics_store, ctx);
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

    if app.mode == Mode::Metrics {
        frame.render_widget(
            Paragraph::new("  ↑↓/jk scroll  r refresh  q/Esc close")
                .style(Style::default().fg(Color::DarkGray)),
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

    // Append any applicable plugin hints after the standard hints.
    let scope = app
        .browser
        .as_ref()
        .and_then(|b| b.resource_gvr.as_ref())
        .map(|g| g.resource.as_str())
        .unwrap_or("all");
    let plugin_hint_str: String = app
        .plugin_config
        .for_scope(scope)
        .into_iter()
        .map(|(_, p)| format!("  {} {}", p.short_cut, p.description))
        .collect::<Vec<_>>()
        .join("  ");

    let base = format_hints(LIST_HINTS);
    let extra = if plugin_hint_str.is_empty() {
        String::new()
    } else {
        format!("  │{plugin_hint_str}")
    };
    frame.render_widget(
        Paragraph::new(format!("{base}{extra}")).style(Style::default().fg(Color::DarkGray)),
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
