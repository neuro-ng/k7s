//! Modal dialog widgets — Phase 10.7 & 10.8.
//!
//! Provides self-contained TUI dialog widgets that overlay the main view:
//!
//! * [`ScaleDialog`]  — numeric input for scaling a workload's replica count.
//! * [`DrainDialog`]  — checkbox options for draining a Kubernetes node.
//! * [`ConfirmDialog`] — generic yes/no confirmation (used by delete actions).
//!
//! All dialogs:
//! - Centre themselves in the available area.
//! - Draw a bordered box with a title.
//! - Return an `Action` from `handle_key()` so the caller can decide what to do.
//!
//! # k9s Reference: `internal/view/scale_extender.go`, `internal/view/drain_dialog.go`

use crossterm::event::KeyCode;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

// ─── ScaleDialog ─────────────────────────────────────────────────────────────

/// Action returned by [`ScaleDialog::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaleAction {
    /// User confirmed with Enter; contains the new replica count.
    Confirm(u32),
    /// User cancelled (Esc / q).
    Cancel,
    /// Key consumed but no terminal action yet.
    None,
}

/// Modal dialog for scaling a workload's replica count.
///
/// # Usage
/// ```ignore
/// let mut dlg = ScaleDialog::new("my-deploy", 3);
/// match dlg.handle_key(key) {
///     ScaleAction::Confirm(n) => dao.scale(name, ns, n).await?,
///     ScaleAction::Cancel     => { /* close dialog */ }
///     ScaleAction::None       => {}
/// }
/// dlg.render(frame, frame.area());
/// ```
pub struct ScaleDialog {
    title: String,
    input: String,
    error: Option<String>,
}

impl ScaleDialog {
    /// Create a new dialog pre-filled with the current replica count.
    pub fn new(resource_name: &str, current: u32) -> Self {
        Self {
            title: format!("Scale · {resource_name}"),
            input: current.to_string(),
            error: None,
        }
    }

    /// Handle a key event.
    pub fn handle_key(&mut self, key: KeyCode) -> ScaleAction {
        match key {
            KeyCode::Enter => match self.input.trim().parse::<u32>() {
                Ok(n) => ScaleAction::Confirm(n),
                Err(_) => {
                    self.error = Some("Enter a valid non-negative integer".into());
                    ScaleAction::None
                }
            },
            KeyCode::Esc | KeyCode::Char('q') => ScaleAction::Cancel,
            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.input.push(c);
                self.error = None;
                ScaleAction::None
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.error = None;
                ScaleAction::None
            }
            _ => ScaleAction::None,
        }
    }

    /// Render the dialog centred inside `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(40, 7, area);
        frame.render_widget(Clear, popup);

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" {} ", self.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));

        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let label = Line::from(vec![
            Span::raw("Replicas: "),
            Span::styled(
                format!("{}_", self.input),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let hint = Line::from(Span::styled(
            "  Enter=confirm   Esc=cancel",
            Style::default().fg(Color::DarkGray),
        ));
        let error_line = self
            .error
            .as_deref()
            .map(|e| Line::from(Span::styled(e, Style::default().fg(Color::Red))));

        let mut lines = vec![Line::raw(""), label, Line::raw(""), hint];
        if let Some(el) = error_line {
            lines.push(el);
        }

        let para = Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true });
        frame.render_widget(para, inner);
    }
}

// ─── DrainDialog ─────────────────────────────────────────────────────────────

/// Options gathered by [`DrainDialog`].
#[derive(Debug, Clone)]
pub struct DrainOptions {
    /// `--ignore-daemonsets`
    pub ignore_daemonsets: bool,
    /// `--delete-emptydir-data`
    pub delete_emptydir_data: bool,
    /// `--force`
    pub force: bool,
    /// `--grace-period` in seconds; `None` = use pod default.
    pub grace_period: Option<u32>,
}

impl Default for DrainOptions {
    fn default() -> Self {
        Self {
            ignore_daemonsets: true,
            delete_emptydir_data: false,
            force: false,
            grace_period: None,
        }
    }
}

/// Action returned by [`DrainDialog::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainAction {
    /// User confirmed; caller should run `kubectl drain` with these options.
    Confirm(DrainConfirm),
    /// User cancelled.
    Cancel,
    /// Key consumed but no terminal action.
    None,
}

/// Confirmed drain request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainConfirm {
    pub ignore_daemonsets: bool,
    pub delete_emptydir_data: bool,
    pub force: bool,
    pub grace_period: Option<u32>,
}

/// Index of the focused checkbox row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrainFocus {
    IgnoreDaemonsets,
    DeleteEmptydirData,
    Force,
}

/// Modal dialog for configuring `kubectl drain` options.
pub struct DrainDialog {
    title: String,
    opts: DrainOptions,
    focused: DrainFocus,
    /// When `Some`, we're collecting a grace-period integer.
    grace_input: Option<String>,
    error: Option<String>,
}

impl DrainDialog {
    pub fn new(node_name: &str) -> Self {
        Self {
            title: format!("Drain · {node_name}"),
            opts: DrainOptions::default(),
            focused: DrainFocus::IgnoreDaemonsets,
            grace_input: None,
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> DrainAction {
        // Grace-period input mode.
        if let Some(ref mut input) = self.grace_input {
            match key {
                KeyCode::Enter => {
                    match input.trim().parse::<u32>() {
                        Ok(n) => {
                            self.opts.grace_period = Some(n);
                            self.grace_input = None;
                            self.error = None;
                        }
                        Err(_) => {
                            self.error = Some("Enter a valid number of seconds".into());
                        }
                    }
                    return DrainAction::None;
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    input.push(c);
                    self.error = None;
                    return DrainAction::None;
                }
                KeyCode::Backspace => {
                    input.pop();
                    return DrainAction::None;
                }
                KeyCode::Esc => {
                    self.grace_input = None;
                    return DrainAction::None;
                }
                _ => return DrainAction::None,
            }
        }

        match key {
            KeyCode::Enter => DrainAction::Confirm(DrainConfirm {
                ignore_daemonsets: self.opts.ignore_daemonsets,
                delete_emptydir_data: self.opts.delete_emptydir_data,
                force: self.opts.force,
                grace_period: self.opts.grace_period,
            }),
            KeyCode::Esc | KeyCode::Char('q') => DrainAction::Cancel,
            KeyCode::Char(' ') => {
                self.toggle_focused();
                DrainAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.focused = match self.focused {
                    DrainFocus::IgnoreDaemonsets => DrainFocus::Force,
                    DrainFocus::DeleteEmptydirData => DrainFocus::IgnoreDaemonsets,
                    DrainFocus::Force => DrainFocus::DeleteEmptydirData,
                };
                DrainAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.focused = match self.focused {
                    DrainFocus::IgnoreDaemonsets => DrainFocus::DeleteEmptydirData,
                    DrainFocus::DeleteEmptydirData => DrainFocus::Force,
                    DrainFocus::Force => DrainFocus::IgnoreDaemonsets,
                };
                DrainAction::None
            }
            KeyCode::Char('g') => {
                // 'g' opens the grace-period input field.
                self.grace_input = Some(
                    self.opts
                        .grace_period
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                );
                DrainAction::None
            }
            _ => DrainAction::None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(50, 12, area);
        frame.render_widget(Clear, popup);

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" {} ", self.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let focused_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let normal_style = Style::default();

        let checkbox = |checked: bool| if checked { "[x]" } else { "[ ]" };
        let row_style = |focus: DrainFocus| {
            if self.focused == focus {
                focused_style
            } else {
                normal_style
            }
        };

        let grace_text = match &self.grace_input {
            Some(s) => format!("Grace period (s): {s}_"),
            None => match self.opts.grace_period {
                Some(n) => format!("Grace period (s): {n}  [g]=edit"),
                None => "Grace period (s): default  [g]=set".into(),
            },
        };

        let mut lines = vec![
            Line::raw(""),
            Line::from(Span::styled(
                format!(
                    "  {} --ignore-daemonsets",
                    checkbox(self.opts.ignore_daemonsets)
                ),
                row_style(DrainFocus::IgnoreDaemonsets),
            )),
            Line::from(Span::styled(
                format!(
                    "  {} --delete-emptydir-data",
                    checkbox(self.opts.delete_emptydir_data)
                ),
                row_style(DrainFocus::DeleteEmptydirData),
            )),
            Line::from(Span::styled(
                format!("  {} --force", checkbox(self.opts.force)),
                row_style(DrainFocus::Force),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                format!("  {grace_text}"),
                Style::default().fg(Color::Cyan),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                "  Space=toggle  Enter=drain  Esc=cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        if let Some(e) = &self.error {
            lines.push(Line::from(Span::styled(
                e.as_str(),
                Style::default().fg(Color::Red),
            )));
        }

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    fn toggle_focused(&mut self) {
        match self.focused {
            DrainFocus::IgnoreDaemonsets => {
                self.opts.ignore_daemonsets = !self.opts.ignore_daemonsets
            }
            DrainFocus::DeleteEmptydirData => {
                self.opts.delete_emptydir_data = !self.opts.delete_emptydir_data
            }
            DrainFocus::Force => self.opts.force = !self.opts.force,
        }
    }
}

// ─── ConfirmDialog ───────────────────────────────────────────────────────────

/// Action returned by [`ConfirmDialog::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// User pressed `y` / Enter.
    Yes,
    /// User pressed `n` / Esc.
    No,
    /// Key consumed, no decision yet.
    None,
}

/// Generic yes/no confirmation overlay.
///
/// # Example message
/// `"Delete pod/nginx-abc123 from namespace default?"`
pub struct ConfirmDialog {
    title: String,
    message: String,
}

impl ConfirmDialog {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
        }
    }

    pub fn handle_key(&self, key: KeyCode) -> ConfirmAction {
        match key {
            KeyCode::Char('y') | KeyCode::Enter => ConfirmAction::Yes,
            KeyCode::Char('n') | KeyCode::Esc => ConfirmAction::No,
            _ => ConfirmAction::None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(50, 7, area);
        frame.render_widget(Clear, popup);

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" {} ", self.title),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let lines = vec![
            Line::raw(""),
            Line::from(Span::raw(self.message.as_str())),
            Line::raw(""),
            Line::from(Span::styled(
                "  y / Enter = confirm   n / Esc = cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Left)
                .wrap(Wrap { trim: true }),
            inner,
        );
    }
}

// ─── ImageUpdateDialog ────────────────────────────────────────────────────────

/// Action returned by [`ImageUpdateDialog::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageUpdateAction {
    /// User confirmed; contains `(container_name, new_image)`.
    Confirm(String, String),
    /// User cancelled.
    Cancel,
    /// Key consumed, no terminal action yet.
    None,
}

/// Modal dialog for updating a container's image reference.
///
/// Displays a single text input pre-filled with the current image.
/// The user edits the tag or full image reference in-place.
///
/// # k9s Reference: `internal/view/image_extender.go`
pub struct ImageUpdateDialog {
    title: String,
    container: String,
    image: String,
    cursor: usize,
    error: Option<String>,
}

impl ImageUpdateDialog {
    /// Create a new dialog for the given container and its current image.
    pub fn new(
        resource_name: &str,
        container: impl Into<String>,
        current_image: impl Into<String>,
    ) -> Self {
        let image = current_image.into();
        let cursor = image.len();
        Self {
            title: format!("Update Image · {resource_name}"),
            container: container.into(),
            image,
            cursor,
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> ImageUpdateAction {
        match key {
            KeyCode::Enter => {
                let img = self.image.trim().to_owned();
                if img.is_empty() {
                    self.error = Some("Image reference cannot be empty".into());
                    ImageUpdateAction::None
                } else {
                    ImageUpdateAction::Confirm(self.container.clone(), img)
                }
            }
            KeyCode::Esc => ImageUpdateAction::Cancel,
            KeyCode::Char(c) => {
                self.image.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                self.error = None;
                ImageUpdateAction::None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.image.remove(self.cursor);
                    self.error = None;
                }
                ImageUpdateAction::None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                ImageUpdateAction::None
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.image.len());
                ImageUpdateAction::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                ImageUpdateAction::None
            }
            KeyCode::End => {
                self.cursor = self.image.len();
                ImageUpdateAction::None
            }
            _ => ImageUpdateAction::None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(60, 8, area);
        frame.render_widget(Clear, popup);

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" {} ", self.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        // Insert cursor character into the image string at cursor position.
        let mut displayed = self.image.clone();
        displayed.insert(self.cursor, '▏');

        let mut lines = vec![
            Line::raw(""),
            Line::from(vec![
                Span::raw(format!("  {} : ", self.container)),
                Span::styled(
                    displayed,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "  ←→ move   Backspace delete   Enter confirm   Esc cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        if let Some(e) = &self.error {
            lines.push(Line::from(Span::styled(
                e.as_str(),
                Style::default().fg(Color::Red),
            )));
        }

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

// ─── PortForwardDialog ────────────────────────────────────────────────────────

/// Action returned by [`PortForwardDialog::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortForwardAction {
    /// User confirmed; contains `(pod_port, local_port)`.
    Confirm(u16, u16),
    /// User cancelled.
    Cancel,
    /// Key consumed but no terminal action yet.
    None,
}

/// Which field is focused inside the port-forward dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PfFocus {
    PodPort,
    LocalPort,
}

/// Modal dialog for configuring a `kubectl port-forward`.
///
/// Displays two editable fields: pod port (pre-filled from the selected
/// container spec if available) and local port (defaults to the pod port).
pub struct PortForwardDialog {
    title: String,
    pod_port: String,
    local_port: String,
    focus: PfFocus,
    error: Option<String>,
}

impl PortForwardDialog {
    /// Create a new dialog pre-filled with `pod_port`.
    ///
    /// `local_port` defaults to the same value as `pod_port`.
    pub fn new(resource_name: &str, pod_port: u16) -> Self {
        let port_str = pod_port.to_string();
        Self {
            title: format!("Port-Forward · {resource_name}"),
            local_port: port_str.clone(),
            pod_port: port_str,
            focus: PfFocus::PodPort,
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> PortForwardAction {
        match key {
            KeyCode::Enter => {
                let pp = self.pod_port.trim().parse::<u16>();
                let lp = self.local_port.trim().parse::<u16>();
                match (pp, lp) {
                    (Ok(pp), Ok(lp)) => PortForwardAction::Confirm(pp, lp),
                    _ => {
                        self.error = Some("Enter valid port numbers (1–65535)".into());
                        PortForwardAction::None
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => PortForwardAction::Cancel,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    PfFocus::PodPort => PfFocus::LocalPort,
                    PfFocus::LocalPort => PfFocus::PodPort,
                };
                PortForwardAction::None
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let field = self.active_field_mut();
                field.push(c);
                self.error = None;
                PortForwardAction::None
            }
            KeyCode::Backspace => {
                self.active_field_mut().pop();
                self.error = None;
                PortForwardAction::None
            }
            _ => PortForwardAction::None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(44, 9, area);
        frame.render_widget(Clear, popup);

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" {} ", self.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let focused_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let normal_style = Style::default().fg(Color::White);

        let pod_style = if self.focus == PfFocus::PodPort {
            focused_style
        } else {
            normal_style
        };
        let local_style = if self.focus == PfFocus::LocalPort {
            focused_style
        } else {
            normal_style
        };

        let mut lines = vec![
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Pod port   : "),
                Span::styled(format!("{}_", self.pod_port), pod_style),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Local port : "),
                Span::styled(format!("{}_", self.local_port), local_style),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "  Tab=switch   Enter=start   Esc=cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        if let Some(e) = &self.error {
            lines.push(Line::from(Span::styled(
                e.as_str(),
                Style::default().fg(Color::Red),
            )));
        }

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.focus {
            PfFocus::PodPort => &mut self.pod_port,
            PfFocus::LocalPort => &mut self.local_port,
        }
    }
}

// ─── Layout helper ───────────────────────────────────────────────────────────

/// Return a [`Rect`] centred in `area` with the given percentage width and
/// fixed-line height.
fn centred_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(
                (100u16.saturating_sub(height.min(100) * 100 / area.height.max(1))) / 2,
            ),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x.min(100)) / 2),
            Constraint::Percentage(percent_x.min(100)),
            Constraint::Percentage((100 - percent_x.min(100)) / 2),
        ])
        .split(vert[1]);

    horiz[1]
}

// ─── HelmInstallDialog ───────────────────────────────────────────────────────

/// Action returned by [`HelmInstallDialog::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelmInstallAction {
    /// User confirmed; carries all install parameters.
    Confirm {
        name: String,
        chart: String,
        namespace: String,
        values_file: Option<String>,
        set_args: Vec<String>,
        dry_run: bool,
    },
    Cancel,
    None,
}

/// A three-field modal for `helm install <name> <chart> -n <namespace>`.
///
/// Tab cycles through fields; Enter submits; Esc cancels.
pub struct HelmInstallDialog {
    release_name: String,
    chart: String,
    namespace: String,
    focused: usize, // 0=name, 1=chart, 2=namespace
    cursor: usize,
    error: Option<String>,
}

impl HelmInstallDialog {
    /// Pre-populate with release name and chart; namespace defaults to "default".
    pub fn new(name: impl Into<String>, chart: impl Into<String>) -> Self {
        let name = name.into();
        let cursor = name.len();
        Self {
            release_name: name,
            chart: chart.into(),
            namespace: "default".to_string(),
            focused: 0,
            cursor,
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> HelmInstallAction {
        match key {
            KeyCode::Enter => {
                let name = self.release_name.trim().to_owned();
                let chart = self.chart.trim().to_owned();
                let ns = self.namespace.trim().to_owned();
                if name.is_empty() {
                    self.error = Some("Release name cannot be empty".into());
                    return HelmInstallAction::None;
                }
                if chart.is_empty() {
                    self.error = Some("Chart reference cannot be empty".into());
                    return HelmInstallAction::None;
                }
                HelmInstallAction::Confirm {
                    name,
                    chart,
                    namespace: if ns.is_empty() { "default".into() } else { ns },
                    values_file: None,
                    set_args: vec![],
                    dry_run: false,
                }
            }
            KeyCode::Esc => HelmInstallAction::Cancel,
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % 3;
                self.cursor = self.active_field().len();
                HelmInstallAction::None
            }
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 { 2 } else { self.focused - 1 };
                self.cursor = self.active_field().len();
                HelmInstallAction::None
            }
            KeyCode::Char(c) => {
                let cursor = self.cursor;
                self.active_field_mut().insert(cursor, c);
                self.cursor += c.len_utf8();
                self.error = None;
                HelmInstallAction::None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let cursor = self.cursor;
                    self.active_field_mut().remove(cursor);
                    self.error = None;
                }
                HelmInstallAction::None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                HelmInstallAction::None
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.active_field().len());
                HelmInstallAction::None
            }
            _ => HelmInstallAction::None,
        }
    }

    fn active_field(&self) -> &str {
        match self.focused {
            0 => &self.release_name,
            1 => &self.chart,
            _ => &self.namespace,
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.focused {
            0 => &mut self.release_name,
            1 => &mut self.chart,
            _ => &mut self.namespace,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(60, 12, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Helm Install ", Style::default().add_modifier(Modifier::BOLD)));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let highlight = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let normal = Style::default().fg(Color::White);

        let field_style = |idx: usize| if idx == self.focused { highlight } else { normal };

        let mut name_disp = self.release_name.clone();
        let mut chart_disp = self.chart.clone();
        let mut ns_disp = self.namespace.clone();
        match self.focused {
            0 => { name_disp.insert(self.cursor, '▏'); }
            1 => { chart_disp.insert(self.cursor, '▏'); }
            _ => { ns_disp.insert(self.cursor, '▏'); }
        }

        let mut lines = vec![
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Release Name : "),
                Span::styled(name_disp, field_style(0)),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Chart        : "),
                Span::styled(chart_disp, field_style(1)),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Namespace    : "),
                Span::styled(ns_disp, field_style(2)),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "  Tab next field   Enter install   Esc cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        if let Some(e) = &self.error {
            lines.push(Line::from(Span::styled(e.as_str(), Style::default().fg(Color::Red))));
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

// ─── HelmUpgradeDialog ───────────────────────────────────────────────────────

/// Action returned by [`HelmUpgradeDialog::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelmUpgradeAction {
    Confirm {
        release: String,
        chart: String,
        namespace: String,
        values_file: Option<String>,
        set_args: Vec<String>,
        install_flag: bool,
        dry_run: bool,
    },
    Cancel,
    None,
}

/// A three-field modal for `helm upgrade <release> <chart> -n <namespace>`.
pub struct HelmUpgradeDialog {
    release: String,
    chart: String,
    namespace: String,
    focused: usize,
    cursor: usize,
    error: Option<String>,
}

impl HelmUpgradeDialog {
    pub fn new(release: impl Into<String>, chart: impl Into<String>) -> Self {
        let release = release.into();
        let cursor = release.len();
        Self {
            release,
            chart: chart.into(),
            namespace: "default".to_string(),
            focused: 0,
            cursor,
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> HelmUpgradeAction {
        match key {
            KeyCode::Enter => {
                let rel = self.release.trim().to_owned();
                let chart = self.chart.trim().to_owned();
                let ns = self.namespace.trim().to_owned();
                if rel.is_empty() {
                    self.error = Some("Release name cannot be empty".into());
                    return HelmUpgradeAction::None;
                }
                if chart.is_empty() {
                    self.error = Some("Chart reference cannot be empty".into());
                    return HelmUpgradeAction::None;
                }
                HelmUpgradeAction::Confirm {
                    release: rel,
                    chart,
                    namespace: if ns.is_empty() { "default".into() } else { ns },
                    values_file: None,
                    set_args: vec![],
                    install_flag: false,
                    dry_run: false,
                }
            }
            KeyCode::Esc => HelmUpgradeAction::Cancel,
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % 3;
                self.cursor = self.active_field().len();
                HelmUpgradeAction::None
            }
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 { 2 } else { self.focused - 1 };
                self.cursor = self.active_field().len();
                HelmUpgradeAction::None
            }
            KeyCode::Char(c) => {
                let cursor = self.cursor;
                self.active_field_mut().insert(cursor, c);
                self.cursor += c.len_utf8();
                self.error = None;
                HelmUpgradeAction::None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let cursor = self.cursor;
                    self.active_field_mut().remove(cursor);
                    self.error = None;
                }
                HelmUpgradeAction::None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                HelmUpgradeAction::None
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.active_field().len());
                HelmUpgradeAction::None
            }
            _ => HelmUpgradeAction::None,
        }
    }

    fn active_field(&self) -> &str {
        match self.focused {
            0 => &self.release,
            1 => &self.chart,
            _ => &self.namespace,
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.focused {
            0 => &mut self.release,
            1 => &mut self.chart,
            _ => &mut self.namespace,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(60, 12, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Helm Upgrade ", Style::default().add_modifier(Modifier::BOLD)));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let highlight = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let normal = Style::default().fg(Color::White);
        let field_style = |idx: usize| if idx == self.focused { highlight } else { normal };

        let mut rel_disp = self.release.clone();
        let mut chart_disp = self.chart.clone();
        let mut ns_disp = self.namespace.clone();
        match self.focused {
            0 => { rel_disp.insert(self.cursor, '▏'); }
            1 => { chart_disp.insert(self.cursor, '▏'); }
            _ => { ns_disp.insert(self.cursor, '▏'); }
        }

        let mut lines = vec![
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Release Name : "),
                Span::styled(rel_disp, field_style(0)),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Chart        : "),
                Span::styled(chart_disp, field_style(1)),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Namespace    : "),
                Span::styled(ns_disp, field_style(2)),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "  Tab next field   Enter upgrade   Esc cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        if let Some(e) = &self.error {
            lines.push(Line::from(Span::styled(e.as_str(), Style::default().fg(Color::Red))));
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ScaleDialog ──────────────────────────────────────────────────────────

    #[test]
    fn scale_confirm_valid() {
        let mut dlg = ScaleDialog::new("my-deploy", 3);
        // Clear pre-filled value and type "5".
        dlg.handle_key(KeyCode::Backspace);
        dlg.handle_key(KeyCode::Char('5'));
        assert_eq!(dlg.handle_key(KeyCode::Enter), ScaleAction::Confirm(5));
    }

    #[test]
    fn scale_prefill() {
        let dlg = ScaleDialog::new("my-deploy", 3);
        assert_eq!(dlg.input, "3");
    }

    #[test]
    fn scale_cancel() {
        let mut dlg = ScaleDialog::new("my-deploy", 3);
        assert_eq!(dlg.handle_key(KeyCode::Esc), ScaleAction::Cancel);
    }

    #[test]
    fn scale_invalid_input_shows_error() {
        let mut dlg = ScaleDialog::new("x", 1);
        dlg.input.clear();
        dlg.input.push('a'); // force a non-digit via direct field access
                             // handle_key Enter should set error since 'a' can't come from handle_key
                             // Let's clear and test the empty case.
        dlg.input.clear();
        assert_eq!(dlg.handle_key(KeyCode::Enter), ScaleAction::None);
        assert!(dlg.error.is_some());
    }

    // ── DrainDialog ──────────────────────────────────────────────────────────

    #[test]
    fn drain_default_options() {
        let dlg = DrainDialog::new("node-1");
        assert!(dlg.opts.ignore_daemonsets);
        assert!(!dlg.opts.delete_emptydir_data);
        assert!(!dlg.opts.force);
    }

    #[test]
    fn drain_toggle_focused() {
        let mut dlg = DrainDialog::new("node-1");
        // Space toggles IgnoreDaemonsets (default focused).
        dlg.handle_key(KeyCode::Char(' '));
        assert!(!dlg.opts.ignore_daemonsets);
    }

    #[test]
    fn drain_navigate_and_toggle() {
        let mut dlg = DrainDialog::new("node-1");
        dlg.handle_key(KeyCode::Down); // => DeleteEmptydirData
        dlg.handle_key(KeyCode::Char(' '));
        assert!(dlg.opts.delete_emptydir_data);
    }

    #[test]
    fn drain_confirm_returns_options() {
        let mut dlg = DrainDialog::new("node-1");
        let action = dlg.handle_key(KeyCode::Enter);
        assert_eq!(
            action,
            DrainAction::Confirm(DrainConfirm {
                ignore_daemonsets: true,
                delete_emptydir_data: false,
                force: false,
                grace_period: None,
            })
        );
    }

    #[test]
    fn drain_cancel() {
        let mut dlg = DrainDialog::new("node-1");
        assert_eq!(dlg.handle_key(KeyCode::Esc), DrainAction::Cancel);
    }

    // ── ConfirmDialog ─────────────────────────────────────────────────────────

    #[test]
    fn confirm_yes_on_y() {
        let dlg = ConfirmDialog::new("Delete", "Delete pod?");
        assert_eq!(dlg.handle_key(KeyCode::Char('y')), ConfirmAction::Yes);
        assert_eq!(dlg.handle_key(KeyCode::Enter), ConfirmAction::Yes);
    }

    #[test]
    fn confirm_no_on_esc() {
        let dlg = ConfirmDialog::new("Delete", "Delete pod?");
        assert_eq!(dlg.handle_key(KeyCode::Esc), ConfirmAction::No);
        assert_eq!(dlg.handle_key(KeyCode::Char('n')), ConfirmAction::No);
    }

    // ── PortForwardDialog ─────────────────────────────────────────────────────

    #[test]
    fn pf_prefill_from_pod_port() {
        let dlg = PortForwardDialog::new("nginx", 8080);
        assert_eq!(dlg.pod_port, "8080");
        assert_eq!(dlg.local_port, "8080");
    }

    #[test]
    fn pf_confirm_returns_ports() {
        let mut dlg = PortForwardDialog::new("nginx", 8080);
        assert_eq!(
            dlg.handle_key(KeyCode::Enter),
            PortForwardAction::Confirm(8080, 8080)
        );
    }

    #[test]
    fn pf_cancel_on_esc() {
        let mut dlg = PortForwardDialog::new("nginx", 8080);
        assert_eq!(dlg.handle_key(KeyCode::Esc), PortForwardAction::Cancel);
    }

    #[test]
    fn pf_tab_switches_focus() {
        let mut dlg = PortForwardDialog::new("svc", 3000);
        assert_eq!(dlg.focus, PfFocus::PodPort);
        dlg.handle_key(KeyCode::Tab);
        assert_eq!(dlg.focus, PfFocus::LocalPort);
        // Type a different local port.
        dlg.handle_key(KeyCode::Backspace);
        dlg.handle_key(KeyCode::Backspace);
        dlg.handle_key(KeyCode::Backspace);
        dlg.handle_key(KeyCode::Backspace);
        dlg.handle_key(KeyCode::Char('9'));
        dlg.handle_key(KeyCode::Char('0'));
        dlg.handle_key(KeyCode::Char('0'));
        dlg.handle_key(KeyCode::Char('0'));
        assert_eq!(
            dlg.handle_key(KeyCode::Enter),
            PortForwardAction::Confirm(3000, 9000)
        );
    }

    // ── ImageUpdateDialog ─────────────────────────────────────────────────────

    #[test]
    fn image_update_prefill() {
        let dlg = ImageUpdateDialog::new("my-deploy", "app", "nginx:1.25");
        assert_eq!(dlg.image, "nginx:1.25");
        assert_eq!(dlg.container, "app");
    }

    #[test]
    fn image_update_confirm() {
        let mut dlg = ImageUpdateDialog::new("dp", "app", "nginx:1.25");
        assert_eq!(
            dlg.handle_key(KeyCode::Enter),
            ImageUpdateAction::Confirm("app".to_owned(), "nginx:1.25".to_owned())
        );
    }

    #[test]
    fn image_update_cancel() {
        let mut dlg = ImageUpdateDialog::new("dp", "app", "nginx:1.25");
        assert_eq!(dlg.handle_key(KeyCode::Esc), ImageUpdateAction::Cancel);
    }

    #[test]
    fn image_update_empty_shows_error() {
        let mut dlg = ImageUpdateDialog::new("dp", "app", "");
        assert_eq!(dlg.handle_key(KeyCode::Enter), ImageUpdateAction::None);
        assert!(dlg.error.is_some());
    }

    #[test]
    fn image_update_backspace_edits() {
        let mut dlg = ImageUpdateDialog::new("dp", "app", "nginx:1.25");
        // Backspace removes last char (cursor is at end).
        dlg.handle_key(KeyCode::Backspace);
        assert_eq!(dlg.image, "nginx:1.2");
    }

    #[test]
    fn image_update_type_appends() {
        let mut dlg = ImageUpdateDialog::new("dp", "app", "nginx:1.2");
        dlg.handle_key(KeyCode::Char('6'));
        assert_eq!(dlg.image, "nginx:1.26");
    }

    #[test]
    fn pf_invalid_port_shows_error() {
        let mut dlg = PortForwardDialog::new("svc", 80);
        // Clear the pod port field and type non-numeric.
        dlg.pod_port.clear();
        dlg.pod_port.push('x'); // direct field write to bypass handle_key digit filter
        assert_eq!(dlg.handle_key(KeyCode::Enter), PortForwardAction::None);
        assert!(dlg.error.is_some());
    }

    #[test]
    fn helm_install_confirm_returns_params() {
        let mut dlg = HelmInstallDialog::new("my-release", "stable/nginx");
        assert_eq!(
            dlg.handle_key(KeyCode::Enter),
            HelmInstallAction::Confirm {
                name: "my-release".to_string(),
                chart: "stable/nginx".to_string(),
                namespace: "default".to_string(),
                values_file: None,
                set_args: vec![],
                dry_run: false,
            }
        );
    }

    #[test]
    fn helm_install_esc_cancels() {
        let mut dlg = HelmInstallDialog::new("rel", "chart");
        assert_eq!(dlg.handle_key(KeyCode::Esc), HelmInstallAction::Cancel);
    }

    #[test]
    fn helm_upgrade_confirm_returns_params() {
        let mut dlg = HelmUpgradeDialog::new("my-release", "stable/nginx");
        assert_eq!(
            dlg.handle_key(KeyCode::Enter),
            HelmUpgradeAction::Confirm {
                release: "my-release".to_string(),
                chart: "stable/nginx".to_string(),
                namespace: "default".to_string(),
                values_file: None,
                set_args: vec![],
                install_flag: false,
                dry_run: false,
            }
        );
    }

    #[test]
    fn helm_upgrade_esc_cancels() {
        let mut dlg = HelmUpgradeDialog::new("rel", "chart");
        assert_eq!(dlg.handle_key(KeyCode::Esc), HelmUpgradeAction::Cancel);
    }
}
