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
    title:    String,
    input:    String,
    error:    Option<String>,
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
            KeyCode::Enter => {
                match self.input.trim().parse::<u32>() {
                    Ok(n)  => ScaleAction::Confirm(n),
                    Err(_) => {
                        self.error = Some("Enter a valid non-negative integer".into());
                        ScaleAction::None
                    }
                }
            }
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

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let label = Line::from(vec![
            Span::raw("Replicas: "),
            Span::styled(
                format!("{}_", self.input),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
        ]);
        let hint = Line::from(Span::styled(
            "  Enter=confirm   Esc=cancel",
            Style::default().fg(Color::DarkGray),
        ));
        let error_line = self.error.as_deref().map(|e| {
            Line::from(Span::styled(e, Style::default().fg(Color::Red)))
        });

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
            ignore_daemonsets:    true,
            delete_emptydir_data: false,
            force:                false,
            grace_period:         None,
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
    pub ignore_daemonsets:    bool,
    pub delete_emptydir_data: bool,
    pub force:                bool,
    pub grace_period:         Option<u32>,
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
    title:   String,
    opts:    DrainOptions,
    focused: DrainFocus,
    /// When `Some`, we're collecting a grace-period integer.
    grace_input: Option<String>,
    error:   Option<String>,
}

impl DrainDialog {
    pub fn new(node_name: &str) -> Self {
        Self {
            title:       format!("Drain · {node_name}"),
            opts:        DrainOptions::default(),
            focused:     DrainFocus::IgnoreDaemonsets,
            grace_input: None,
            error:       None,
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
                ignore_daemonsets:    self.opts.ignore_daemonsets,
                delete_emptydir_data: self.opts.delete_emptydir_data,
                force:                self.opts.force,
                grace_period:         self.opts.grace_period,
            }),
            KeyCode::Esc | KeyCode::Char('q') => DrainAction::Cancel,
            KeyCode::Char(' ') => {
                self.toggle_focused();
                DrainAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.focused = match self.focused {
                    DrainFocus::IgnoreDaemonsets    => DrainFocus::Force,
                    DrainFocus::DeleteEmptydirData  => DrainFocus::IgnoreDaemonsets,
                    DrainFocus::Force               => DrainFocus::DeleteEmptydirData,
                };
                DrainAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.focused = match self.focused {
                    DrainFocus::IgnoreDaemonsets    => DrainFocus::DeleteEmptydirData,
                    DrainFocus::DeleteEmptydirData  => DrainFocus::Force,
                    DrainFocus::Force               => DrainFocus::IgnoreDaemonsets,
                };
                DrainAction::None
            }
            KeyCode::Char('g') => {
                // 'g' opens the grace-period input field.
                self.grace_input = Some(
                    self.opts.grace_period
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

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let focused_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let normal_style  = Style::default();

        let checkbox = |checked: bool| if checked { "[x]" } else { "[ ]" };
        let row_style = |focus: DrainFocus| {
            if self.focused == focus { focused_style } else { normal_style }
        };

        let grace_text = match &self.grace_input {
            Some(s) => format!("Grace period (s): {s}_"),
            None    => match self.opts.grace_period {
                Some(n) => format!("Grace period (s): {n}  [g]=edit"),
                None    => "Grace period (s): default  [g]=set".into(),
            },
        };

        let mut lines = vec![
            Line::raw(""),
            Line::from(Span::styled(
                format!("  {} --ignore-daemonsets", checkbox(self.opts.ignore_daemonsets)),
                row_style(DrainFocus::IgnoreDaemonsets),
            )),
            Line::from(Span::styled(
                format!("  {} --delete-emptydir-data", checkbox(self.opts.delete_emptydir_data)),
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
            lines.push(Line::from(Span::styled(e.as_str(), Style::default().fg(Color::Red))));
        }

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn toggle_focused(&mut self) {
        match self.focused {
            DrainFocus::IgnoreDaemonsets    => self.opts.ignore_daemonsets    = !self.opts.ignore_daemonsets,
            DrainFocus::DeleteEmptydirData  => self.opts.delete_emptydir_data = !self.opts.delete_emptydir_data,
            DrainFocus::Force               => self.opts.force                = !self.opts.force,
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
    title:   String,
    message: String,
}

impl ConfirmDialog {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title:   title.into(),
            message: message.into(),
        }
    }

    pub fn handle_key(&self, key: KeyCode) -> ConfirmAction {
        match key {
            KeyCode::Char('y') | KeyCode::Enter => ConfirmAction::Yes,
            KeyCode::Char('n') | KeyCode::Esc   => ConfirmAction::No,
            _ => ConfirmAction::None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(50, 7, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
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

// ─── Layout helper ───────────────────────────────────────────────────────────

/// Return a [`Rect`] centred in `area` with the given percentage width and
/// fixed-line height.
fn centred_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100u16.saturating_sub(height.min(100) * 100 / area.height.max(1))) / 2),
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
                ignore_daemonsets:    true,
                delete_emptydir_data: false,
                force:                false,
                grace_period:         None,
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
        assert_eq!(dlg.handle_key(KeyCode::Enter),     ConfirmAction::Yes);
    }

    #[test]
    fn confirm_no_on_esc() {
        let dlg = ConfirmDialog::new("Delete", "Delete pod?");
        assert_eq!(dlg.handle_key(KeyCode::Esc),        ConfirmAction::No);
        assert_eq!(dlg.handle_key(KeyCode::Char('n')),  ConfirmAction::No);
    }
}
