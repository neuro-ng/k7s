//! KubeVela Application Platform TUI view — Phase 36.
//!
//! Multi-pane view for KubeVela (OAM) resources:
//!
//! | Sub-view   | Trigger         | Description                              |
//! |------------|-----------------|------------------------------------------|
//! | `Apps`     | `:vela` / `:va` | All Application CRs                      |
//! | `Components` | `Enter` on app | OAM components + trait health            |
//! | `Workflow` | `w` on app      | Workflow step phases                     |
//! | `Revisions`| `h` on app      | ApplicationRevision history              |
//! | `Defs`     | `:veladefs`/`:vd` | Capability definitions (component/trait) |
//!
//! Data is loaded asynchronously via `VelaAction` returns; the caller (`App`)
//! spawns the kube tasks and feeds the results back via `set_*` methods.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState,
};
use ratatui::Frame;

use crate::render::vela as rvela;
use crate::vela::{
    parse_components, parse_policies, parse_workflow_steps, VelaApplication, VelaComponent,
    VelaDefinition, VelaPolicy, VelaRevision, VelaWorkflowStep,
};

// ─── Sub-view ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum SubView {
    Apps,
    Components,
    Workflow,
    Revisions,
    Defs,
    /// OAM policies for the selected application.
    Policy,
    /// Full scrollable YAML pane for a selected definition.
    DefDetail,
}

// ─── Actions ──────────────────────────────────────────────────────────────────

/// What the calling `App` should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum VelaAction {
    /// User closed the top-level Apps or Defs view — exit Vela mode.
    Close,
    /// Reload the Application list.
    Refresh,
    /// Load ApplicationRevisions for `(app_name, namespace)`.
    LoadRevisions { name: String, namespace: String },
    /// Load capability definitions (component, trait, etc.).
    LoadDefs { def_type: String },
    /// Open AI analysis for the selected application.
    AiAnalyse { name: String, namespace: String },
    /// Show confirm dialog: delete the named application.
    ConfirmDelete { name: String, namespace: String },
    /// Show confirm dialog: workflow restart the named application.
    ConfirmRestart { name: String, namespace: String },
    /// Show confirm dialog: workflow resume the named application.
    ConfirmResume { name: String, namespace: String },
    /// Show confirm dialog: rollback to the selected revision.
    ConfirmRollback {
        name: String,
        namespace: String,
        revision: Option<u64>,
    },
    /// No action needed from the caller.
    None,
}

// ─── VelaView ─────────────────────────────────────────────────────────────────

/// KubeVela multi-pane TUI view.
pub struct VelaView {
    sub: SubView,

    // ── Apps ──────────────────────────────────────────────────────────────────
    apps: Vec<VelaApplication>,
    app_table: TableState,

    // ── Components ────────────────────────────────────────────────────────────
    components: Vec<VelaComponent>,
    comp_table: TableState,

    // ── Workflow ──────────────────────────────────────────────────────────────
    workflow_steps: Vec<VelaWorkflowStep>,
    wf_table: TableState,

    // ── Revisions ─────────────────────────────────────────────────────────────
    revisions: Vec<VelaRevision>,
    rev_table: TableState,

    // ── Defs ──────────────────────────────────────────────────────────────────
    defs: Vec<VelaDefinition>,
    def_table: TableState,
    def_type_filter: String,

    // ── Policy ────────────────────────────────────────────────────────────────
    policies: Vec<VelaPolicy>,
    policy_table: TableState,

    // ── DefDetail (scrollable YAML pane) ──────────────────────────────────────
    def_detail_lines: Vec<String>,
    def_detail_title: String,
    def_detail_scroll: u16,

    // ── Context ───────────────────────────────────────────────────────────────
    /// Name of the currently expanded application.
    selected_app_name: String,
    selected_app_ns: String,

    /// Transient status / error line shown at the bottom of the view.
    pub status: Option<String>,
}

impl VelaView {
    pub fn new() -> Self {
        Self {
            sub: SubView::Apps,
            apps: Vec::new(),
            app_table: TableState::default(),
            components: Vec::new(),
            comp_table: TableState::default(),
            workflow_steps: Vec::new(),
            wf_table: TableState::default(),
            revisions: Vec::new(),
            rev_table: TableState::default(),
            defs: Vec::new(),
            def_table: TableState::default(),
            def_type_filter: "component".to_string(),
            policies: Vec::new(),
            policy_table: TableState::default(),
            def_detail_lines: Vec::new(),
            def_detail_title: String::new(),
            def_detail_scroll: 0,
            selected_app_name: String::new(),
            selected_app_ns: String::new(),
            status: None,
        }
    }

    // ── Data setters (called from App tick) ───────────────────────────────────

    pub fn set_apps(&mut self, apps: Vec<VelaApplication>) {
        self.apps = apps;
        if !self.apps.is_empty() && self.app_table.selected().is_none() {
            self.app_table.select(Some(0));
        }
        self.status = None;
    }

    pub fn set_revisions(&mut self, revisions: Vec<VelaRevision>) {
        self.revisions = revisions;
        if !self.revisions.is_empty() {
            self.rev_table.select(Some(self.revisions.len() - 1));
        }
        self.sub = SubView::Revisions;
        self.status = None;
    }

    pub fn set_defs(&mut self, defs: Vec<VelaDefinition>, def_type: &str) {
        self.defs = defs;
        self.def_type_filter = def_type.to_string();
        if !self.defs.is_empty() && self.def_table.selected().is_none() {
            self.def_table.select(Some(0));
        }
        self.sub = SubView::Defs;
        self.status = None;
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
    }

    /// Load policies for the selected application and switch to Policy sub-view.
    pub fn set_policies(&mut self, policies: Vec<VelaPolicy>) {
        self.policies = policies;
        self.policy_table = TableState::default();
        if !self.policies.is_empty() {
            self.policy_table.select(Some(0));
        }
        self.sub = SubView::Policy;
        self.status = None;
    }

    // ── Key dispatch ──────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, event: &KeyEvent) -> VelaAction {
        match self.sub {
            SubView::Apps => self.handle_apps_key(event),
            SubView::Components => self.handle_components_key(event),
            SubView::Workflow => self.handle_workflow_key(event),
            SubView::Revisions => self.handle_revisions_key(event),
            SubView::Defs => self.handle_defs_key(event),
            SubView::Policy => self.handle_policy_key(event),
            SubView::DefDetail => self.handle_def_detail_key(event),
        }
    }

    // ── Apps key handler ──────────────────────────────────────────────────────

    fn handle_apps_key(&mut self, event: &KeyEvent) -> VelaAction {
        match event.code {
            KeyCode::Char('q') | KeyCode::Esc => return VelaAction::Close,

            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.app_table.selected().unwrap_or(0);
                if i > 0 {
                    self.app_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.app_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.apps.len() {
                    self.app_table.select(Some(next));
                }
            }
            KeyCode::Home | KeyCode::Char('g') if !self.apps.is_empty() => {
                self.app_table.select(Some(0));
            }
            KeyCode::End | KeyCode::Char('G') if !self.apps.is_empty() => {
                self.app_table.select(Some(self.apps.len() - 1));
            }

            // Enter — open component tree for the selected app.
            KeyCode::Enter => {
                if let Some(app) = self.selected_app() {
                    let name = app.name.clone();
                    let ns = app.namespace.clone();
                    let raw = app.raw.clone();
                    self.selected_app_name = name;
                    self.selected_app_ns = ns;
                    self.components = parse_components(&raw);
                    self.comp_table = TableState::default();
                    if !self.components.is_empty() {
                        self.comp_table.select(Some(0));
                    }
                    self.sub = SubView::Components;
                }
            }

            // w — workflow steps for the selected app.
            KeyCode::Char('w') => {
                if let Some(app) = self.selected_app() {
                    let name = app.name.clone();
                    let ns = app.namespace.clone();
                    let raw = app.raw.clone();
                    self.selected_app_name = name;
                    self.selected_app_ns = ns;
                    self.workflow_steps = parse_workflow_steps(&raw);
                    self.wf_table = TableState::default();
                    if !self.workflow_steps.is_empty() {
                        self.wf_table.select(Some(0));
                    }
                    self.sub = SubView::Workflow;
                }
            }

            // h — revision history (async — needs kube API call).
            KeyCode::Char('h') => {
                if let Some(app) = self.selected_app() {
                    let name = app.name.clone();
                    let ns = app.namespace.clone();
                    self.selected_app_name = name.clone();
                    self.selected_app_ns = ns.clone();
                    self.status = Some(format!("Loading revisions for {name}…"));
                    return VelaAction::LoadRevisions {
                        name,
                        namespace: ns,
                    };
                }
            }

            // p — policies for the selected app.
            KeyCode::Char('p') => {
                if let Some(app) = self.selected_app() {
                    let name = app.name.clone();
                    let ns = app.namespace.clone();
                    let raw = app.raw.clone();
                    let _ = app;
                    self.selected_app_name = name;
                    self.selected_app_ns = ns;
                    let policies = parse_policies(&raw);
                    self.set_policies(policies);
                }
            }

            // D — confirm delete application.
            KeyCode::Char('D') => {
                if let Some(app) = self.selected_app() {
                    return VelaAction::ConfirmDelete {
                        name: app.name.clone(),
                        namespace: app.namespace.clone(),
                    };
                }
            }

            // Ctrl+R — confirm workflow restart.
            KeyCode::Char('r') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(app) = self.selected_app() {
                    return VelaAction::ConfirmRestart {
                        name: app.name.clone(),
                        namespace: app.namespace.clone(),
                    };
                }
            }

            // Ctrl+S — confirm workflow resume.
            KeyCode::Char('s') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(app) = self.selected_app() {
                    return VelaAction::ConfirmResume {
                        name: app.name.clone(),
                        namespace: app.namespace.clone(),
                    };
                }
            }

            // r / F5 — refresh app list (no Ctrl modifier).
            KeyCode::Char('r') | KeyCode::F(5) => return VelaAction::Refresh,

            // A — AI analyse.
            KeyCode::Char('A') => {
                if let Some(app) = self.selected_app() {
                    return VelaAction::AiAnalyse {
                        name: app.name.clone(),
                        namespace: app.namespace.clone(),
                    };
                }
            }

            _ => {}
        }
        VelaAction::None
    }

    // ── Components key handler ────────────────────────────────────────────────

    fn handle_components_key(&mut self, event: &KeyEvent) -> VelaAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::Apps;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.comp_table.selected().unwrap_or(0);
                if i > 0 {
                    self.comp_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.comp_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.components.len() {
                    self.comp_table.select(Some(next));
                }
            }
            _ => {}
        }
        VelaAction::None
    }

    // ── Workflow key handler ──────────────────────────────────────────────────

    fn handle_workflow_key(&mut self, event: &KeyEvent) -> VelaAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::Apps;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.wf_table.selected().unwrap_or(0);
                if i > 0 {
                    self.wf_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.wf_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.workflow_steps.len() {
                    self.wf_table.select(Some(next));
                }
            }
            _ => {}
        }
        VelaAction::None
    }

    // ── Revisions key handler ─────────────────────────────────────────────────

    fn handle_revisions_key(&mut self, event: &KeyEvent) -> VelaAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::Apps;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.rev_table.selected().unwrap_or(0);
                if i > 0 {
                    self.rev_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.rev_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.revisions.len() {
                    self.rev_table.select(Some(next));
                }
            }
            // r — confirm rollback to selected revision.
            KeyCode::Char('r') => {
                let revision = self
                    .rev_table
                    .selected()
                    .and_then(|i| self.revisions.get(i))
                    .map(|r| r.revision);
                return VelaAction::ConfirmRollback {
                    name: self.selected_app_name.clone(),
                    namespace: self.selected_app_ns.clone(),
                    revision,
                };
            }
            _ => {}
        }
        VelaAction::None
    }

    // ── Defs key handler ──────────────────────────────────────────────────────

    fn handle_defs_key(&mut self, event: &KeyEvent) -> VelaAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => return VelaAction::Close,

            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.def_table.selected().unwrap_or(0);
                if i > 0 {
                    self.def_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.def_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.defs.len() {
                    self.def_table.select(Some(next));
                }
            }

            // Tab — cycle through definition types.
            KeyCode::Tab => {
                let next = match self.def_type_filter.as_str() {
                    "component" => "trait",
                    "trait" => "workflowstep",
                    "workflowstep" => "policy",
                    _ => "component",
                };
                let t = next.to_string();
                self.def_table = TableState::default();
                self.status = Some(format!("Loading {t} definitions…"));
                return VelaAction::LoadDefs { def_type: t };
            }

            // d — open definition detail (scrollable YAML pane).
            KeyCode::Char('d') => {
                if let Some(def) = self.def_table.selected().and_then(|i| self.defs.get(i)) {
                    let title = format!(" {} — {} ", def.name, def.def_type);
                    let yaml = serde_yaml::to_string(&def.raw).unwrap_or_else(|_| {
                        format!(
                            "name: {}\ntype: {}\ndescription: {}",
                            def.name, def.def_type, def.description
                        )
                    });
                    self.def_detail_title = title;
                    self.def_detail_lines = yaml.lines().map(str::to_owned).collect();
                    self.def_detail_scroll = 0;
                    self.sub = SubView::DefDetail;
                }
            }

            // r / F5 — refresh current definition type.
            KeyCode::Char('r') | KeyCode::F(5) => {
                let t = self.def_type_filter.clone();
                return VelaAction::LoadDefs { def_type: t };
            }

            _ => {}
        }
        VelaAction::None
    }

    // ── Policy key handler ────────────────────────────────────────────────────

    fn handle_policy_key(&mut self, event: &KeyEvent) -> VelaAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::Apps;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.policy_table.selected().unwrap_or(0);
                if i > 0 {
                    self.policy_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.policy_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.policies.len() {
                    self.policy_table.select(Some(next));
                }
            }
            _ => {}
        }
        VelaAction::None
    }

    // ── DefDetail key handler ─────────────────────────────────────────────────

    fn handle_def_detail_key(&mut self, event: &KeyEvent) -> VelaAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::Defs;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.def_detail_scroll = self.def_detail_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.def_detail_lines.len().saturating_sub(1) as u16;
                if self.def_detail_scroll < max {
                    self.def_detail_scroll += 1;
                }
            }
            KeyCode::PageUp => {
                self.def_detail_scroll = self.def_detail_scroll.saturating_sub(20);
            }
            KeyCode::PageDown => {
                let max = self.def_detail_lines.len().saturating_sub(1) as u16;
                self.def_detail_scroll = (self.def_detail_scroll + 20).min(max);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.def_detail_scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.def_detail_scroll = self.def_detail_lines.len().saturating_sub(1) as u16;
            }
            _ => {}
        }
        VelaAction::None
    }

    // ── Render ────────────────────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        match &self.sub {
            SubView::Apps => self.render_apps(frame, area),
            SubView::Components => self.render_components(frame, area),
            SubView::Workflow => self.render_workflow(frame, area),
            SubView::Revisions => self.render_revisions(frame, area),
            SubView::Defs => self.render_defs(frame, area),
            SubView::Policy => self.render_policy(frame, area),
            SubView::DefDetail => self.render_def_detail(frame, area),
        }
    }

    fn render_apps(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = rvela::app_headers();
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .apps
            .iter()
            .map(|app| {
                let status_style = rvela::status_color(&app.status);
                Row::new(vec![
                    Cell::from(app.name.clone()),
                    Cell::from(app.namespace.clone()),
                    Cell::from(app.status.clone()).style(status_style),
                    Cell::from(app.workflow_status.clone()),
                    Cell::from(app.component_count.to_string()),
                    Cell::from(app.age_label()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(25),
            Constraint::Percentage(18),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
            Constraint::Length(12),
            Constraint::Percentage(9),
        ];

        let title = if self.apps.is_empty() {
            " KubeVela Applications (none — is vela installed?) ".to_string()
        } else {
            format!(" KubeVela Applications ({}) ", self.apps.len())
        };

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.app_table);
        self.render_status(frame, area);
    }

    fn render_components(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = rvela::component_headers();
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .components
            .iter()
            .map(|comp| {
                let healthy_style = if comp.healthy {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                };
                let traits = comp
                    .traits
                    .iter()
                    .map(|t| t.trait_type.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                Row::new(vec![
                    Cell::from(comp.name.clone()),
                    Cell::from(comp.workload_type.clone()),
                    Cell::from(if comp.healthy { "true" } else { "false" }).style(healthy_style),
                    Cell::from(traits),
                    Cell::from(comp.message.clone()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(22),
            Constraint::Percentage(18),
            Constraint::Length(8),
            Constraint::Percentage(20),
            Constraint::Percentage(34),
        ];

        let title = format!(
            " {} — Components ({}) ",
            self.selected_app_name,
            self.components.len()
        );

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.comp_table);
    }

    fn render_workflow(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = rvela::workflow_headers();
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .workflow_steps
            .iter()
            .map(|step| {
                let phase_style = rvela::phase_color(&step.phase);
                Row::new(vec![
                    Cell::from(step.name.clone()),
                    Cell::from(step.step_type.clone()),
                    Cell::from(step.phase.clone()).style(phase_style),
                    Cell::from(step.message.clone()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(22),
            Constraint::Percentage(18),
            Constraint::Percentage(14),
            Constraint::Percentage(44),
        ];

        let title = format!(
            " {} — Workflow Steps ({}) ",
            self.selected_app_name,
            self.workflow_steps.len()
        );

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.wf_table);
    }

    fn render_revisions(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = rvela::revision_headers();
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .revisions
            .iter()
            .map(|rev| {
                let status_style = if rev.status == "succeeded" {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                Row::new(vec![
                    Cell::from(rev.revision.to_string()),
                    Cell::from(rev.name.clone()),
                    Cell::from(rev.deploy_time.clone()),
                    Cell::from(rev.status.clone()).style(status_style),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(10),
            Constraint::Percentage(35),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ];

        let title = format!(
            " {} — Revisions ({}) ",
            self.selected_app_name,
            self.revisions.len()
        );

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.rev_table);
        self.render_status(frame, area);
    }

    fn render_defs(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = rvela::def_headers();
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .defs
            .iter()
            .map(|def| {
                Row::new(vec![
                    Cell::from(def.name.clone()),
                    Cell::from(def.def_type.clone()),
                    Cell::from(def.description.clone()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(28),
            Constraint::Percentage(16),
            Constraint::Percentage(54),
        ];

        let title = format!(
            " KubeVela {} Definitions ({})  Tab=next type  r=refresh ",
            capitalize(&self.def_type_filter),
            self.defs.len()
        );

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.def_table);
        self.render_status(frame, area);
    }

    fn render_policy(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = ["NAME", "TYPE", "PROPERTIES"];
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .policies
            .iter()
            .map(|p| {
                let props = if p.properties.is_null() {
                    "-".to_string()
                } else {
                    serde_json::to_string(&p.properties).unwrap_or_else(|_| "-".into())
                };
                Row::new(vec![
                    Cell::from(p.name.clone()),
                    Cell::from(p.policy_type.clone()),
                    Cell::from(props),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(53),
        ];

        let title = if self.policies.is_empty() {
            format!(" {} — Policies (none) ", self.selected_app_name)
        } else {
            format!(
                " {} — Policies ({}) ",
                self.selected_app_name,
                self.policies.len()
            )
        };

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.policy_table);
        self.render_status(frame, area);
    }

    fn render_def_detail(&mut self, frame: &mut Frame, area: Rect) {
        let total = self.def_detail_lines.len();
        let content = self.def_detail_lines.join("\n");
        let scroll = self.def_detail_scroll;

        let block = Block::default()
            .borders(Borders::ALL)
            .title(self.def_detail_title.clone())
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        let para = Paragraph::new(content)
            .block(block)
            .style(Style::default().fg(Color::White))
            .scroll((scroll, 0));

        frame.render_widget(para, area);

        // Scrollbar on the right edge.
        if total > area.height as usize {
            let mut sb_state = ScrollbarState::new(total.saturating_sub(area.height as usize))
                .position(scroll as usize);
            let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            frame.render_stateful_widget(sb, area, &mut sb_state);
        }
    }

    /// Draw the status line (error / loading message) at the bottom of `area`.
    fn render_status(&self, frame: &mut Frame, area: Rect) {
        if let Some(msg) = &self.status {
            let hint_area = Rect {
                x: area.x + 1,
                y: area.y + area.height.saturating_sub(1),
                width: area.width.saturating_sub(2),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Yellow)),
                hint_area,
            );
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn selected_app(&self) -> Option<&VelaApplication> {
        self.app_table.selected().and_then(|i| self.apps.get(i))
    }

    /// Whether the current sub-view is the top-level Defs view.
    pub fn is_defs_mode(&self) -> bool {
        self.sub == SubView::Defs
    }

    /// Switch directly into Defs sub-view (used when `:veladefs` is navigated).
    pub fn enter_defs_mode(&mut self) {
        self.sub = SubView::Defs;
        self.defs.clear();
        self.def_table = TableState::default();
    }
}

impl Default for VelaView {
    fn default() -> Self {
        Self::new()
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use serde_json::json;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_app(name: &str, status: &str) -> VelaApplication {
        VelaApplication {
            name: name.into(),
            namespace: "default".into(),
            status: status.into(),
            component_count: 1,
            workflow_status: "succeeded".into(),
            age_secs: 100,
            raw: json!({
                "spec": {
                    "components": [{"name": "web", "type": "webservice"}]
                },
                "status": {
                    "services": [{"name": "web", "healthy": true, "message": "", "traits": []}],
                    "workflow": {
                        "steps": [
                            {"name": "deploy", "type": "deploy", "phase": "succeeded", "message": ""}
                        ]
                    }
                }
            }),
        }
    }

    #[test]
    fn new_starts_in_apps_mode() {
        let view = VelaView::new();
        assert!(matches!(view.sub, SubView::Apps));
    }

    #[test]
    fn q_in_apps_returns_close() {
        let mut view = VelaView::new();
        let action = view.handle_key(&press(KeyCode::Char('q')));
        assert_eq!(action, VelaAction::Close);
    }

    #[test]
    fn esc_in_components_returns_to_apps() {
        let mut view = VelaView::new();
        view.sub = SubView::Components;
        view.handle_key(&press(KeyCode::Esc));
        assert!(matches!(view.sub, SubView::Apps));
    }

    #[test]
    fn enter_on_app_opens_components() {
        let mut view = VelaView::new();
        view.set_apps(vec![make_app("my-app", "running")]);
        view.handle_key(&press(KeyCode::Enter));
        assert!(matches!(view.sub, SubView::Components));
        assert_eq!(view.components.len(), 1);
        assert_eq!(view.components[0].name, "web");
    }

    #[test]
    fn w_on_app_opens_workflow() {
        let mut view = VelaView::new();
        view.set_apps(vec![make_app("my-app", "running")]);
        view.handle_key(&press(KeyCode::Char('w')));
        assert!(matches!(view.sub, SubView::Workflow));
        assert_eq!(view.workflow_steps.len(), 1);
        assert_eq!(view.workflow_steps[0].name, "deploy");
    }

    #[test]
    fn h_on_app_returns_load_revisions() {
        let mut view = VelaView::new();
        view.set_apps(vec![make_app("my-app", "running")]);
        let action = view.handle_key(&press(KeyCode::Char('h')));
        assert!(matches!(action, VelaAction::LoadRevisions { .. }));
    }

    #[test]
    fn set_revisions_switches_to_revisions_sub_view() {
        let mut view = VelaView::new();
        view.set_revisions(vec![VelaRevision {
            name: "my-app-v1".into(),
            revision: 1,
            deploy_time: "2026-05-01".into(),
            status: "succeeded".into(),
        }]);
        assert!(matches!(view.sub, SubView::Revisions));
        assert_eq!(view.revisions.len(), 1);
    }

    #[test]
    fn set_defs_switches_to_defs_sub_view() {
        let mut view = VelaView::new();
        view.set_defs(
            vec![VelaDefinition {
                name: "webservice".into(),
                def_type: "Component".into(),
                description: "webservice".into(),
                raw: serde_json::Value::Null,
            }],
            "component",
        );
        assert!(matches!(view.sub, SubView::Defs));
        assert_eq!(view.defs.len(), 1);
    }

    #[test]
    fn down_navigation_wraps_at_end() {
        let mut view = VelaView::new();
        view.set_apps(vec![make_app("a", "running"), make_app("b", "running")]);
        view.handle_key(&press(KeyCode::Down));
        assert_eq!(view.app_table.selected(), Some(1));
        // Should not go past end.
        view.handle_key(&press(KeyCode::Down));
        assert_eq!(view.app_table.selected(), Some(1));
    }

    #[test]
    fn refresh_returns_action() {
        let mut view = VelaView::new();
        let action = view.handle_key(&press(KeyCode::Char('r')));
        assert_eq!(action, VelaAction::Refresh);
    }

    #[test]
    fn ai_analyse_returns_action() {
        let mut view = VelaView::new();
        view.set_apps(vec![make_app("my-app", "running")]);
        let action = view.handle_key(&press(KeyCode::Char('A')));
        assert!(matches!(action, VelaAction::AiAnalyse { .. }));
    }

    #[test]
    fn capitalize_works() {
        assert_eq!(capitalize("component"), "Component");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("workflowstep"), "Workflowstep");
    }

    #[test]
    fn q_in_defs_returns_close() {
        let mut view = VelaView::new();
        view.sub = SubView::Defs;
        let action = view.handle_key(&press(KeyCode::Char('q')));
        assert_eq!(action, VelaAction::Close);
    }

    #[test]
    fn tab_in_defs_cycles_type() {
        let mut view = VelaView::new();
        view.sub = SubView::Defs;
        view.def_type_filter = "component".into();
        let action = view.handle_key(&press(KeyCode::Tab));
        assert!(matches!(action, VelaAction::LoadDefs { def_type } if def_type == "trait"));
    }

    #[test]
    fn enter_defs_mode_clears_defs() {
        let mut view = VelaView::new();
        view.defs = vec![VelaDefinition {
            name: "old".into(),
            def_type: "Component".into(),
            description: "".into(),
            raw: serde_json::Value::Null,
        }];
        view.enter_defs_mode();
        assert!(view.defs.is_empty());
        assert!(matches!(view.sub, SubView::Defs));
    }

    #[test]
    fn p_on_app_opens_policy_view() {
        let mut view = VelaView::new();
        view.set_apps(vec![make_app("my-app", "running")]);
        view.handle_key(&press(KeyCode::Char('p')));
        assert!(matches!(view.sub, SubView::Policy));
    }

    #[test]
    fn d_on_def_opens_def_detail() {
        let mut view = VelaView::new();
        view.set_defs(
            vec![VelaDefinition {
                name: "webservice".into(),
                def_type: "Component".into(),
                description: "web".into(),
                raw: serde_json::Value::Null,
            }],
            "component",
        );
        view.handle_key(&press(KeyCode::Char('d')));
        assert!(matches!(view.sub, SubView::DefDetail));
    }

    #[test]
    fn big_d_on_app_returns_confirm_delete() {
        let mut view = VelaView::new();
        view.set_apps(vec![make_app("my-app", "running")]);
        let action = view.handle_key(&press(KeyCode::Char('D')));
        assert!(matches!(action, VelaAction::ConfirmDelete { .. }));
    }

    #[test]
    fn r_in_revisions_returns_confirm_rollback() {
        let mut view = VelaView::new();
        view.selected_app_name = "my-app".into();
        view.selected_app_ns = "default".into();
        view.set_revisions(vec![VelaRevision {
            name: "my-app-v2".into(),
            revision: 2,
            deploy_time: "2026-05-01".into(),
            status: "succeeded".into(),
        }]);
        let action = view.handle_key(&press(KeyCode::Char('r')));
        assert!(matches!(
            action,
            VelaAction::ConfirmRollback {
                revision: Some(2),
                ..
            }
        ));
    }

    #[test]
    fn esc_in_def_detail_returns_to_defs() {
        let mut view = VelaView::new();
        view.sub = SubView::DefDetail;
        view.handle_key(&press(KeyCode::Esc));
        assert!(matches!(view.sub, SubView::Defs));
    }

    #[test]
    fn esc_in_policy_returns_to_apps() {
        let mut view = VelaView::new();
        view.sub = SubView::Policy;
        view.handle_key(&press(KeyCode::Esc));
        assert!(matches!(view.sub, SubView::Apps));
    }
}
