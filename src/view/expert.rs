//! Expert Mode TUI — Phase 21.
//!
//! Provides an active monitoring overlay that automatically detects pod
//! failures, performance issues, and log problems, then queries the LLM for
//! analysis and recommendations — all within the sanitizer security boundary.
//!
//! # Activation
//!
//! Expert mode is toggled with `X` inside the TUI or with `--expert` on the
//! CLI.  When active, a status-bar badge shows `[EXPERT]` in yellow.
//!
//! # Alert pipeline
//!
//! ```text
//! Watcher events
//!      │
//!      ▼
//! FailureDetector  ──→  PodAlert / PerformanceAlert / LogAlert
//!      │
//!      ▼ (sanitized)
//! LLM query (async)
//!      │
//!      ▼
//! ExpertPanel.alerts  ←  displayed in TUI overlay
//! ```
//!
//! # k9s Reference
//!
//! k9s has no equivalent — this is a k7s-unique feature.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

// ─── Alert types ─────────────────────────────────────────────────────────────

/// Category of a detected cluster problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertKind {
    /// Pod in CrashLoopBackOff / OOMKilled / ImagePullBackOff / Evicted.
    PodFailure,
    /// High CPU or memory utilization, or throttling events detected.
    Performance,
    /// Repeated error patterns found in application logs.
    LogSpam,
}

impl AlertKind {
    fn label(&self) -> &'static str {
        match self {
            Self::PodFailure => "POD",
            Self::Performance => "PERF",
            Self::LogSpam => "LOG",
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::PodFailure => Color::Red,
            Self::Performance => Color::Yellow,
            Self::LogSpam => Color::Cyan,
        }
    }
}

/// A single detected cluster problem with optional LLM recommendation.
#[derive(Debug, Clone)]
pub struct ExpertAlert {
    pub kind: AlertKind,
    pub resource: String,
    pub namespace: String,
    pub summary: String,
    pub recommendation: Option<String>,
    pub detected_at: DateTime<Utc>,
    /// Whether the LLM is still generating the recommendation.
    pub pending: bool,
    /// Name of the owning controller (Deployment, StatefulSet, DaemonSet) if known.
    pub owner_name: Option<String>,
    /// Kind of the owning controller (e.g. "Deployment", "StatefulSet").
    pub owner_kind: Option<String>,
}

impl ExpertAlert {
    pub fn new(
        kind: AlertKind,
        resource: impl Into<String>,
        namespace: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            resource: resource.into(),
            namespace: namespace.into(),
            summary: summary.into(),
            recommendation: None,
            detected_at: Utc::now(),
            pending: true,
            owner_name: None,
            owner_kind: None,
        }
    }
}

// ─── Failure detector ─────────────────────────────────────────────────────────

/// Stateless rules for detecting problems from raw pod / event JSON.
///
/// The detector never sees secret values — it only inspects status fields
/// that are on the sanitizer allowlist.
pub struct FailureDetector;

impl FailureDetector {
    /// Inspect a pod's JSON (already field-filtered) and return an alert if
    /// the pod is in a failure state worth reporting.
    pub fn check_pod(pod_json: &serde_json::Value) -> Option<ExpertAlert> {
        let name = pod_json
            .pointer("/metadata/name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let ns = pod_json
            .pointer("/metadata/namespace")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Check container statuses for failure reasons.
        let container_statuses = pod_json
            .pointer("/status/containerStatuses")
            .and_then(|v| v.as_array());

        if let Some(statuses) = container_statuses {
            for cs in statuses {
                let reason = cs
                    .pointer("/state/waiting/reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                match reason {
                    "CrashLoopBackOff" => {
                        let restarts = cs
                            .pointer("/restartCount")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let mut alert = ExpertAlert::new(
                            AlertKind::PodFailure,
                            &name,
                            &ns,
                            format!("CrashLoopBackOff (restarts: {restarts})"),
                        );
                        enrich_owner(&mut alert, pod_json);
                        return Some(alert);
                    }
                    "OOMKilled" | "Error" => {
                        let mut alert = ExpertAlert::new(
                            AlertKind::PodFailure,
                            &name,
                            &ns,
                            format!("Container terminated: {reason}"),
                        );
                        enrich_owner(&mut alert, pod_json);
                        return Some(alert);
                    }
                    "ImagePullBackOff" | "ErrImagePull" => {
                        let mut alert = ExpertAlert::new(
                            AlertKind::PodFailure,
                            &name,
                            &ns,
                            format!("Image pull failure: {reason}"),
                        );
                        enrich_owner(&mut alert, pod_json);
                        return Some(alert);
                    }
                    _ => {}
                }

                // OOMKilled appears in last state terminated.
                let last_reason = cs
                    .pointer("/lastState/terminated/reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if last_reason == "OOMKilled" {
                    let mut alert = ExpertAlert::new(
                        AlertKind::PodFailure,
                        &name,
                        &ns,
                        "OOMKilled (out of memory)".to_string(),
                    );
                    enrich_owner(&mut alert, pod_json);
                    return Some(alert);
                }
            }
        }

        // Check pod phase.
        let phase = pod_json
            .pointer("/status/phase")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if phase == "Failed" {
            let reason = pod_json
                .pointer("/status/reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let mut alert = ExpertAlert::new(
                AlertKind::PodFailure,
                &name,
                &ns,
                format!("Pod Failed: {reason}"),
            );
            enrich_owner(&mut alert, pod_json);
            return Some(alert);
        }

        None
    }

    /// Inspect an event JSON and return a performance alert if throttling or
    /// resource pressure is detected.
    pub fn check_event(event_json: &serde_json::Value) -> Option<ExpertAlert> {
        let reason = event_json
            .pointer("/reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let message = event_json
            .pointer("/message")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ns = event_json
            .pointer("/metadata/namespace")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let involved = event_json
            .pointer("/involvedObject/name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        match reason {
            "Evicted" | "Killing" | "BackOff" => {
                return Some(ExpertAlert::new(
                    AlertKind::PodFailure,
                    &involved,
                    &ns,
                    format!("{reason}: {}", &message[..message.len().min(120)]),
                ));
            }
            "SystemOOM" | "OOMKilling" => {
                return Some(ExpertAlert::new(
                    AlertKind::Performance,
                    &involved,
                    &ns,
                    format!("Node OOM pressure: {}", &message[..message.len().min(120)]),
                ));
            }
            _ => {}
        }

        // Throttling keywords in the message.
        let lower = message.to_lowercase();
        if lower.contains("throttl") || lower.contains("cpu limit") || lower.contains("oom") {
            return Some(ExpertAlert::new(
                AlertKind::Performance,
                &involved,
                &ns,
                format!(
                    "Throttling detected: {}",
                    &message[..message.len().min(100)]
                ),
            ));
        }

        None
    }

    /// Check compressed log output for repeated error patterns.
    ///
    /// `log_text` is the output of the log compressor, never raw logs.
    pub fn check_logs(pod_name: &str, namespace: &str, log_text: &str) -> Option<ExpertAlert> {
        let error_lines: Vec<&str> = log_text
            .lines()
            .filter(|l| {
                let lower = l.to_lowercase();
                lower.contains("error")
                    || lower.contains("exception")
                    || lower.contains("panic")
                    || lower.contains("fatal")
            })
            .take(5)
            .collect();

        if error_lines.len() >= 2 {
            let sample = error_lines.join("; ");
            let truncated = if sample.len() > 200 {
                format!("{}…", &sample[..200])
            } else {
                sample
            };
            return Some(ExpertAlert::new(
                AlertKind::LogSpam,
                pod_name,
                namespace,
                format!("Repeated errors in logs: {truncated}"),
            ));
        }

        None
    }
}

/// Populate `alert.owner_name` / `alert.owner_kind` from the pod's
/// `metadata.ownerReferences` array.  Only the first owner ref is used; only
/// controller-owned refs are considered (e.g. ReplicaSet → points at Deployment).
fn enrich_owner(alert: &mut ExpertAlert, pod_json: &serde_json::Value) {
    let Some(refs) = pod_json
        .pointer("/metadata/ownerReferences")
        .and_then(|v| v.as_array())
    else {
        return;
    };
    for r in refs {
        let is_controller = r
            .pointer("/controller")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !is_controller {
            continue;
        }
        let kind = r
            .pointer("/kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = r
            .pointer("/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !kind.is_empty() && !name.is_empty() {
            alert.owner_kind = Some(kind);
            alert.owner_name = Some(name);
        }
        break;
    }
}

// ─── Remediation types ────────────────────────────────────────────────────────

/// A concrete action the user can apply directly from the expert panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemediationKind {
    /// Delete the failing pod — its controller will recreate it.
    DeletePod { name: String, namespace: String },
    /// Run `kubectl rollout restart <kind>/<owner> -n <namespace>`.
    RolloutRestart {
        owner: String,
        owner_kind: String,
        namespace: String,
    },
    /// Open the log viewer for the pod.
    ViewLogs { name: String, namespace: String },
}

/// A suggested remediation action surfaced in the expert detail pane.
#[derive(Debug, Clone)]
pub struct RemediationSuggestion {
    pub kind: RemediationKind,
    pub label: &'static str,
    pub description: &'static str,
}

/// Build the list of remediation actions applicable to `alert`.
///
/// The list is ordered by safety: read-only actions first, destructive last.
pub fn suggestions_for_alert(alert: &ExpertAlert) -> Vec<RemediationSuggestion> {
    let name = alert.resource.clone();
    let ns = alert.namespace.clone();
    let mut out = Vec::new();

    // View logs is always available for pod-related alerts.
    out.push(RemediationSuggestion {
        kind: RemediationKind::ViewLogs {
            name: name.clone(),
            namespace: ns.clone(),
        },
        label: "View Logs",
        description: "Open the log viewer for this pod (read-only)",
    });

    // Delete pod — applicable for pod failures and log spam.
    if matches!(alert.kind, AlertKind::PodFailure | AlertKind::LogSpam) {
        out.push(RemediationSuggestion {
            kind: RemediationKind::DeletePod {
                name: name.clone(),
                namespace: ns.clone(),
            },
            label: "Delete Pod",
            description: "Delete the pod — its controller will recreate it",
        });
    }

    // Rollout restart — only when the owning controller is known.
    if let (Some(owner), Some(owner_kind)) = (&alert.owner_name, &alert.owner_kind) {
        out.push(RemediationSuggestion {
            kind: RemediationKind::RolloutRestart {
                owner: owner.clone(),
                owner_kind: owner_kind.clone(),
                namespace: ns.clone(),
            },
            label: "Rollout Restart",
            description: "Rolling restart of the owning controller (zero-downtime)",
        });
    }

    out
}

// ─── ExpertPanel (TUI widget) ─────────────────────────────────────────────────

/// Maximum alerts kept in the rolling buffer.
const MAX_ALERTS: usize = 50;

/// Actions returned from key handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpertAction {
    /// No action / key consumed.
    Noop,
    /// User closed the expert overlay.
    Close,
    /// User pressed Enter on an alert — open detail view.
    SelectAlert(usize),
    /// User dismissed the selected alert.
    Dismiss,
    /// User pressed `r` — caller should trigger an immediate cluster rescan.
    Rescan,
    /// User selected a remediation action from the detail pane.
    Remediate(RemediationSuggestion),
}

impl PartialEq for RemediationSuggestion {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for RemediationSuggestion {}

/// The expert mode TUI overlay.
///
/// Renders as a right-side panel with a scrollable list of alerts and a
/// detail pane showing the LLM recommendation for the selected alert.
pub struct ExpertPanel {
    /// Rolling buffer of detected alerts, newest first.
    alerts: VecDeque<ExpertAlert>,
    list_state: ListState,
    /// Whether to show the full-detail pane for the selected alert.
    show_detail: bool,
    /// Scroll offset inside the detail pane.
    detail_scroll: u16,
    /// Whether the remediation action sub-menu is open inside the detail pane.
    remediation_mode: bool,
    /// Index of the highlighted remediation action in the sub-menu.
    remediation_selected: usize,
}

impl ExpertPanel {
    pub fn new() -> Self {
        Self {
            alerts: VecDeque::new(),
            list_state: ListState::default(),
            show_detail: false,
            detail_scroll: 0,
            remediation_mode: false,
            remediation_selected: 0,
        }
    }

    pub fn alert_count(&self) -> usize {
        self.alerts.len()
    }

    pub fn pending_count(&self) -> usize {
        self.alerts.iter().filter(|a| a.pending).count()
    }

    /// Push a newly detected alert.  Deduplicates against the most recent 10
    /// alerts by (kind, resource, namespace, summary prefix) to avoid flooding.
    pub fn push_alert(&mut self, alert: ExpertAlert) {
        let is_dup = self.alerts.iter().take(10).any(|a| {
            a.kind == alert.kind
                && a.resource == alert.resource
                && a.namespace == alert.namespace
                && a.summary
                    .chars()
                    .take(40)
                    .eq(alert.summary.chars().take(40))
        });
        if is_dup {
            return;
        }

        self.alerts.push_front(alert);
        while self.alerts.len() > MAX_ALERTS {
            self.alerts.pop_back();
        }

        // Select first item if nothing selected.
        if self.list_state.selected().is_none() && !self.alerts.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    /// Update the recommendation text for an alert identified by (resource, namespace, summary).
    pub fn set_recommendation(
        &mut self,
        resource: &str,
        namespace: &str,
        summary_prefix: &str,
        recommendation: String,
    ) {
        for alert in &mut self.alerts {
            if alert.resource == resource
                && alert.namespace == namespace
                && alert.summary.starts_with(summary_prefix)
            {
                alert.recommendation = Some(recommendation);
                alert.pending = false;
                return;
            }
        }
    }

    /// Handle a key event inside the expert panel.
    pub fn handle_key(&mut self, key: &KeyEvent) -> ExpertAction {
        match key.code {
            // ── Global ───────────────────────────────────────────────────────
            KeyCode::Char('r') => return ExpertAction::Rescan,

            // ── Esc: unwind one layer at a time ──────────────────────────────
            KeyCode::Esc => {
                if self.remediation_mode {
                    self.remediation_mode = false;
                    self.remediation_selected = 0;
                } else if self.show_detail {
                    self.show_detail = false;
                    self.detail_scroll = 0;
                } else {
                    return ExpertAction::Close;
                }
            }

            // ── Close from list view ─────────────────────────────────────────
            KeyCode::Char('q') if !self.show_detail => return ExpertAction::Close,

            // ── Enter: in remediation menu → execute; in list → open detail ──
            KeyCode::Enter | KeyCode::Char(' ') if self.remediation_mode => {
                if let Some(idx) = self.list_state.selected() {
                    if let Some(alert) = self.alerts.get(idx) {
                        let suggestions = suggestions_for_alert(alert);
                        if let Some(suggestion) =
                            suggestions.into_iter().nth(self.remediation_selected)
                        {
                            return ExpertAction::Remediate(suggestion);
                        }
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') if !self.show_detail => {
                if let Some(idx) = self.list_state.selected() {
                    self.show_detail = true;
                    self.detail_scroll = 0;
                    return ExpertAction::SelectAlert(idx);
                }
            }

            // ── 'a': toggle remediation sub-menu (only in detail view) ───────
            KeyCode::Char('a') if self.show_detail => {
                self.remediation_mode = !self.remediation_mode;
                self.remediation_selected = 0;
            }

            // ── Dismiss alert ─────────────────────────────────────────────────
            KeyCode::Char('d') | KeyCode::Delete if !self.remediation_mode => {
                if let Some(idx) = self.list_state.selected() {
                    self.alerts.remove(idx);
                    if self.alerts.is_empty() {
                        self.list_state.select(None);
                    } else {
                        let new_idx = idx.min(self.alerts.len().saturating_sub(1));
                        self.list_state.select(Some(new_idx));
                    }
                    self.show_detail = false;
                    self.remediation_mode = false;
                    return ExpertAction::Dismiss;
                }
            }

            // ── Navigation ───────────────────────────────────────────────────
            KeyCode::Up | KeyCode::Char('k') => {
                if self.remediation_mode {
                    self.remediation_selected = self.remediation_selected.saturating_sub(1);
                } else if self.show_detail {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                } else {
                    self.move_selection(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.remediation_mode {
                    if let Some(idx) = self.list_state.selected() {
                        if let Some(alert) = self.alerts.get(idx) {
                            let count = suggestions_for_alert(alert).len();
                            if count > 0 {
                                self.remediation_selected =
                                    (self.remediation_selected + 1).min(count - 1);
                            }
                        }
                    }
                } else if self.show_detail {
                    self.detail_scroll += 1;
                } else {
                    self.move_selection(1);
                }
            }

            _ => {}
        }
        ExpertAction::Noop
    }

    fn move_selection(&mut self, delta: i32) {
        if self.alerts.is_empty() {
            return;
        }
        let count = self.alerts.len() as i32;
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, count - 1) as usize;
        self.list_state.select(Some(next));
    }

    /// Render the expert panel.
    ///
    /// Draws a full-width overlay if `fullscreen`, otherwise a right-side
    /// panel occupying ~40% of the given `area`.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, fullscreen: bool) {
        let panel_area = if fullscreen {
            area
        } else {
            // Right 40% panel
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(area);
            chunks[1]
        };

        // Clear background so the panel draws cleanly over other widgets.
        frame.render_widget(Clear, panel_area);

        if self.show_detail {
            self.render_detail(frame, panel_area);
        } else {
            self.render_list(frame, panel_area);
        }
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let header_area = Rect { height: 1, ..area };
        let list_area = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(2),
            ..area
        };
        let footer_area = Rect {
            y: area.y + area.height.saturating_sub(1),
            height: 1,
            ..area
        };

        // Header
        let pending = self.pending_count();
        let header_text = if pending > 0 {
            format!(
                " ⚡ EXPERT MODE — {} alert(s)  ({} analyzing…)",
                self.alerts.len(),
                pending
            )
        } else {
            format!(" ⚡ EXPERT MODE — {} alert(s)", self.alerts.len())
        };
        let header = Paragraph::new(header_text).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(header, header_area);

        // Alert list
        let items: Vec<ListItem> = self
            .alerts
            .iter()
            .map(|a| {
                let badge = Span::styled(
                    format!("[{}]", a.kind.label()),
                    Style::default()
                        .fg(a.kind.color())
                        .add_modifier(Modifier::BOLD),
                );
                let ns_res = Span::styled(
                    format!(" {}/{} ", a.namespace, a.resource),
                    Style::default().fg(Color::White),
                );
                let status = if a.pending {
                    Span::styled("…", Style::default().fg(Color::DarkGray))
                } else {
                    Span::styled("✓", Style::default().fg(Color::Green))
                };
                let summary = Span::styled(
                    format!(" {}", &a.summary[..a.summary.len().min(50)]),
                    Style::default().fg(Color::Gray),
                );

                let line = Line::from(vec![badge, ns_res, status, summary]);
                ListItem::new(line)
            })
            .collect();

        let list_block = Block::default()
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::Yellow));

        let list = List::new(items)
            .block(list_block)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(list, list_area, &mut self.list_state);

        // Footer hints
        let footer = Paragraph::new(" Enter: detail  d: dismiss  ↑↓: navigate  q/Esc: close")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(footer, footer_area);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        let idx = self.list_state.selected().unwrap_or(0);
        let Some(alert) = self.alerts.get(idx) else {
            return;
        };

        let block = Block::default()
            .title(format!(
                " {} — {}/{} ",
                alert.kind.label(),
                alert.namespace,
                alert.resource
            ))
            .title_style(
                Style::default()
                    .fg(alert.kind.color())
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.remediation_mode {
            // Split: top ~60% recommendation, bottom ~40% remediation actions.
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                .split(inner);
            self.render_recommendation_para(frame, alert, chunks[0]);
            self.render_remediation_submenu(frame, alert, chunks[1]);
        } else {
            // Full area for recommendation; one-line footer with hint.
            let content_area = Rect {
                height: inner.height.saturating_sub(1),
                ..inner
            };
            let footer_area = Rect {
                y: inner.y + inner.height.saturating_sub(1),
                height: 1,
                ..inner
            };
            self.render_recommendation_para(frame, alert, content_area);
            let footer = Paragraph::new(" a: actions  ↑↓: scroll  d: dismiss  Esc: back")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(footer, footer_area);
        }
    }

    fn render_recommendation_para(&self, frame: &mut Frame, alert: &ExpertAlert, area: Rect) {
        let content = if alert.pending {
            format!(
                "Detected: {}\n\nSummary:\n{}\n\n⏳ Analyzing with AI…",
                alert.detected_at.format("%H:%M:%S UTC"),
                alert.summary,
            )
        } else {
            let rec = alert
                .recommendation
                .as_deref()
                .unwrap_or("No recommendation available.");
            format!(
                "Detected: {}\n\nSummary:\n{}\n\n💡 Recommendation:\n{}",
                alert.detected_at.format("%H:%M:%S UTC"),
                alert.summary,
                rec,
            )
        };

        let para = Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .scroll((self.detail_scroll, 0))
            .style(Style::default().fg(Color::White));
        frame.render_widget(para, area);
    }

    fn render_remediation_submenu(&self, frame: &mut Frame, alert: &ExpertAlert, area: Rect) {
        if area.height < 3 {
            return;
        }
        let header_area = Rect { height: 1, ..area };
        let list_area = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(2),
            ..area
        };
        let footer_area = Rect {
            y: area.y + area.height.saturating_sub(1),
            height: 1,
            ..area
        };

        let header = Paragraph::new(" ⚡ Remediation Actions").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(header, header_area);

        let suggestions = suggestions_for_alert(alert);
        let items: Vec<ListItem> = suggestions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let selected = i == self.remediation_selected;
                let prefix = if selected { "▶ " } else { "  " };
                let line = Line::from(vec![
                    Span::styled(
                        format!("{}{}", prefix, s.label),
                        if selected {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                    Span::styled(
                        format!("  — {}", s.description),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items);
        frame.render_widget(list, list_area);

        let footer = Paragraph::new(" Enter: apply  ↑↓: navigate  Esc: cancel")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(footer, footer_area);
    }
}

impl Default for ExpertPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ─── LLM prompt builder for expert mode ──────────────────────────────────────

/// Build a terse, token-efficient LLM prompt for an alert.
///
/// The prompt is intentionally compact — expert mode may generate many
/// queries in a session, so each one must stay well within the per-query
/// token budget.
pub fn build_expert_prompt(alert: &ExpertAlert) -> String {
    let kind = alert.kind.label();
    let resource = &alert.resource;
    let ns = &alert.namespace;
    let summary = &alert.summary;

    format!(
        "Kubernetes {kind} alert for {ns}/{resource}:\n\
         {summary}\n\n\
         Provide a concise (3-5 sentences) root cause analysis and the \
         top 1-2 remediation steps. Focus on actionable kubectl commands \
         where applicable. No markdown headers."
    )
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detect_crashloop() {
        let pod = json!({
            "metadata": { "name": "web-abc", "namespace": "default" },
            "status": {
                "containerStatuses": [{
                    "restartCount": 5,
                    "state": { "waiting": { "reason": "CrashLoopBackOff" } }
                }]
            }
        });
        let alert = FailureDetector::check_pod(&pod).unwrap();
        assert_eq!(alert.kind, AlertKind::PodFailure);
        assert!(alert.summary.contains("CrashLoopBackOff"));
        assert!(alert.summary.contains("5"));
    }

    #[test]
    fn detect_oomkilled_last_state() {
        let pod = json!({
            "metadata": { "name": "oom-pod", "namespace": "prod" },
            "status": {
                "containerStatuses": [{
                    "restartCount": 1,
                    "state": { "running": {} },
                    "lastState": { "terminated": { "reason": "OOMKilled" } }
                }]
            }
        });
        let alert = FailureDetector::check_pod(&pod).unwrap();
        assert_eq!(alert.kind, AlertKind::PodFailure);
        assert!(alert.summary.contains("OOMKilled"));
    }

    #[test]
    fn detect_image_pull_failure() {
        let pod = json!({
            "metadata": { "name": "bad-image", "namespace": "dev" },
            "status": {
                "containerStatuses": [{
                    "restartCount": 0,
                    "state": { "waiting": { "reason": "ImagePullBackOff" } }
                }]
            }
        });
        let alert = FailureDetector::check_pod(&pod).unwrap();
        assert!(alert.summary.contains("ImagePullBackOff"));
    }

    #[test]
    fn no_alert_for_healthy_pod() {
        let pod = json!({
            "metadata": { "name": "healthy", "namespace": "default" },
            "status": {
                "phase": "Running",
                "containerStatuses": [{
                    "restartCount": 0,
                    "state": { "running": {} }
                }]
            }
        });
        assert!(FailureDetector::check_pod(&pod).is_none());
    }

    #[test]
    fn detect_evicted_event() {
        let event = json!({
            "metadata": { "namespace": "prod" },
            "reason": "Evicted",
            "message": "The node was low on resource: memory.",
            "involvedObject": { "name": "worker-123" }
        });
        let alert = FailureDetector::check_event(&event).unwrap();
        assert_eq!(alert.kind, AlertKind::PodFailure);
    }

    #[test]
    fn detect_throttle_in_event_message() {
        let event = json!({
            "metadata": { "namespace": "staging" },
            "reason": "Info",
            "message": "Container is being throttled due to CPU limit.",
            "involvedObject": { "name": "api-pod" }
        });
        let alert = FailureDetector::check_event(&event).unwrap();
        assert_eq!(alert.kind, AlertKind::Performance);
    }

    #[test]
    fn detect_log_error_spam() {
        let logs = "2024-01-01 ERROR connection refused\n\
                    2024-01-01 ERROR connection refused\n\
                    2024-01-01 INFO heartbeat ok\n\
                    2024-01-01 ERROR timeout exceeded\n";
        let alert = FailureDetector::check_logs("api", "default", logs).unwrap();
        assert_eq!(alert.kind, AlertKind::LogSpam);
    }

    #[test]
    fn no_alert_for_clean_logs() {
        let logs = "2024-01-01 INFO request handled\n2024-01-01 INFO request handled\n";
        assert!(FailureDetector::check_logs("api", "default", logs).is_none());
    }

    #[test]
    fn panel_deduplicates() {
        let mut panel = ExpertPanel::new();
        let alert = ExpertAlert::new(AlertKind::PodFailure, "web", "default", "CrashLoopBackOff");
        panel.push_alert(alert.clone());
        panel.push_alert(alert);
        assert_eq!(panel.alert_count(), 1);
    }

    #[test]
    fn panel_dismiss_removes_alert() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(
            AlertKind::PodFailure,
            "web",
            "default",
            "crash",
        ));
        panel.push_alert(ExpertAlert::new(AlertKind::LogSpam, "api", "ns", "errors"));
        panel.list_state.select(Some(0));

        let key = KeyEvent::from(KeyCode::Char('d'));
        panel.handle_key(&key);
        assert_eq!(panel.alert_count(), 1);
    }

    #[test]
    fn expert_prompt_contains_kind_and_resource() {
        let alert = ExpertAlert::new(AlertKind::PodFailure, "web-abc", "prod", "CrashLoopBackOff");
        let prompt = build_expert_prompt(&alert);
        assert!(prompt.contains("POD"));
        assert!(prompt.contains("web-abc"));
        assert!(prompt.contains("prod"));
        assert!(prompt.contains("CrashLoopBackOff"));
    }

    #[test]
    fn set_recommendation_marks_not_pending() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(
            AlertKind::PodFailure,
            "pod-x",
            "ns",
            "CrashLoop",
        ));
        panel.set_recommendation("pod-x", "ns", "CrashLoop", "Restart the deployment.".into());
        let alert = &panel.alerts[0];
        assert!(!alert.pending);
        assert!(alert.recommendation.is_some());
    }

    #[test]
    fn r_key_returns_rescan() {
        let mut panel = ExpertPanel::new();
        let key = KeyEvent::from(KeyCode::Char('r'));
        assert_eq!(panel.handle_key(&key), ExpertAction::Rescan);
    }

    #[test]
    fn r_key_in_detail_view_returns_rescan() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(
            AlertKind::PodFailure,
            "pod",
            "ns",
            "crash",
        ));
        panel.list_state.select(Some(0));
        // Open the detail pane first.
        let enter = KeyEvent::from(KeyCode::Enter);
        panel.handle_key(&enter);
        // r should still return Rescan even when detail is open.
        let key = KeyEvent::from(KeyCode::Char('r'));
        assert_eq!(panel.handle_key(&key), ExpertAction::Rescan);
    }

    #[test]
    fn q_key_closes_when_not_in_detail() {
        let mut panel = ExpertPanel::new();
        let key = KeyEvent::from(KeyCode::Char('q'));
        assert_eq!(panel.handle_key(&key), ExpertAction::Close);
    }

    #[test]
    fn esc_exits_detail_not_panel() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(AlertKind::LogSpam, "api", "ns", "errors"));
        panel.list_state.select(Some(0));
        let enter = KeyEvent::from(KeyCode::Enter);
        panel.handle_key(&enter); // open detail
        let esc = KeyEvent::from(KeyCode::Esc);
        // First Esc closes detail, second closes panel.
        assert_eq!(panel.handle_key(&esc), ExpertAction::Noop);
        assert_eq!(panel.handle_key(&esc), ExpertAction::Close);
    }

    // ── Phase 23: owner ref parsing ───────────────────────────────────────────

    #[test]
    fn enrich_owner_sets_deployment_fields() {
        let pod = json!({
            "metadata": {
                "name": "web-abc-xyz",
                "namespace": "prod",
                "ownerReferences": [{
                    "kind": "ReplicaSet",
                    "name": "web-abc",
                    "controller": true
                }]
            },
            "status": {
                "containerStatuses": [{
                    "restartCount": 3,
                    "state": { "waiting": { "reason": "CrashLoopBackOff" } }
                }]
            }
        });
        let alert = FailureDetector::check_pod(&pod).unwrap();
        assert_eq!(alert.owner_kind.as_deref(), Some("ReplicaSet"));
        assert_eq!(alert.owner_name.as_deref(), Some("web-abc"));
    }

    #[test]
    fn enrich_owner_ignores_non_controller_refs() {
        let pod = json!({
            "metadata": {
                "name": "standalone",
                "namespace": "dev",
                "ownerReferences": [{
                    "kind": "Deployment",
                    "name": "web",
                    "controller": false
                }]
            },
            "status": {
                "containerStatuses": [{
                    "restartCount": 1,
                    "state": { "waiting": { "reason": "CrashLoopBackOff" } }
                }]
            }
        });
        let alert = FailureDetector::check_pod(&pod).unwrap();
        assert!(alert.owner_name.is_none());
        assert!(alert.owner_kind.is_none());
    }

    #[test]
    fn enrich_owner_absent_when_no_refs() {
        let pod = json!({
            "metadata": { "name": "orphan", "namespace": "default" },
            "status": {
                "containerStatuses": [{
                    "restartCount": 0,
                    "state": { "waiting": { "reason": "ImagePullBackOff" } }
                }]
            }
        });
        let alert = FailureDetector::check_pod(&pod).unwrap();
        assert!(alert.owner_name.is_none());
    }

    // ── Phase 23: suggestions_for_alert ──────────────────────────────────────

    #[test]
    fn suggestions_always_include_view_logs() {
        let alert = ExpertAlert::new(AlertKind::PodFailure, "pod", "ns", "crash");
        let s = suggestions_for_alert(&alert);
        assert!(s.iter().any(|x| x.label == "View Logs"));
    }

    #[test]
    fn suggestions_include_delete_for_pod_failure() {
        let alert = ExpertAlert::new(AlertKind::PodFailure, "pod", "ns", "crash");
        let s = suggestions_for_alert(&alert);
        assert!(s.iter().any(|x| x.label == "Delete Pod"));
    }

    #[test]
    fn suggestions_include_delete_for_log_spam() {
        let alert = ExpertAlert::new(AlertKind::LogSpam, "pod", "ns", "errors");
        let s = suggestions_for_alert(&alert);
        assert!(s.iter().any(|x| x.label == "Delete Pod"));
    }

    #[test]
    fn suggestions_no_delete_for_performance_alert() {
        let alert = ExpertAlert::new(AlertKind::Performance, "node-1", "default", "OOM");
        let s = suggestions_for_alert(&alert);
        assert!(!s.iter().any(|x| x.label == "Delete Pod"));
    }

    #[test]
    fn suggestions_include_rollout_restart_when_owner_known() {
        let mut alert = ExpertAlert::new(AlertKind::PodFailure, "pod", "ns", "crash");
        alert.owner_name = Some("web".into());
        alert.owner_kind = Some("Deployment".into());
        let s = suggestions_for_alert(&alert);
        assert!(s.iter().any(|x| x.label == "Rollout Restart"));
    }

    #[test]
    fn suggestions_no_rollout_restart_without_owner() {
        let alert = ExpertAlert::new(AlertKind::PodFailure, "orphan", "ns", "crash");
        let s = suggestions_for_alert(&alert);
        assert!(!s.iter().any(|x| x.label == "Rollout Restart"));
    }

    // ── Phase 23: remediation key handling ───────────────────────────────────

    #[test]
    fn a_key_opens_remediation_mode_in_detail() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(
            AlertKind::PodFailure,
            "pod",
            "ns",
            "crash",
        ));
        panel.list_state.select(Some(0));
        // Open detail view first.
        panel.handle_key(&KeyEvent::from(KeyCode::Enter));
        assert!(!panel.remediation_mode);
        // Press 'a' to open remediation sub-menu.
        panel.handle_key(&KeyEvent::from(KeyCode::Char('a')));
        assert!(panel.remediation_mode);
    }

    #[test]
    fn a_key_toggles_remediation_mode() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(
            AlertKind::PodFailure,
            "pod",
            "ns",
            "crash",
        ));
        panel.list_state.select(Some(0));
        panel.handle_key(&KeyEvent::from(KeyCode::Enter));
        panel.handle_key(&KeyEvent::from(KeyCode::Char('a')));
        panel.handle_key(&KeyEvent::from(KeyCode::Char('a')));
        assert!(!panel.remediation_mode);
    }

    #[test]
    fn esc_closes_remediation_before_detail() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(
            AlertKind::PodFailure,
            "pod",
            "ns",
            "crash",
        ));
        panel.list_state.select(Some(0));
        panel.handle_key(&KeyEvent::from(KeyCode::Enter)); // open detail
        panel.handle_key(&KeyEvent::from(KeyCode::Char('a'))); // open remediation
                                                               // First Esc: closes remediation sub-menu (stays in detail).
        assert_eq!(
            panel.handle_key(&KeyEvent::from(KeyCode::Esc)),
            ExpertAction::Noop
        );
        assert!(!panel.remediation_mode);
        assert!(panel.show_detail);
        // Second Esc: closes detail.
        assert_eq!(
            panel.handle_key(&KeyEvent::from(KeyCode::Esc)),
            ExpertAction::Noop
        );
        assert!(!panel.show_detail);
        // Third Esc: closes panel.
        assert_eq!(
            panel.handle_key(&KeyEvent::from(KeyCode::Esc)),
            ExpertAction::Close
        );
    }

    #[test]
    fn enter_in_remediation_mode_returns_remediate() {
        let mut panel = ExpertPanel::new();
        panel.push_alert(ExpertAlert::new(
            AlertKind::PodFailure,
            "pod",
            "ns",
            "crash",
        ));
        panel.list_state.select(Some(0));
        panel.handle_key(&KeyEvent::from(KeyCode::Enter)); // open detail
        panel.handle_key(&KeyEvent::from(KeyCode::Char('a'))); // open remediation
                                                               // remediation_selected = 0 → ViewLogs (first suggestion)
        let action = panel.handle_key(&KeyEvent::from(KeyCode::Enter));
        match action {
            ExpertAction::Remediate(s) => {
                assert_eq!(s.label, "View Logs");
            }
            other => panic!("expected Remediate, got {other:?}"),
        }
    }

    #[test]
    fn down_key_navigates_remediation_list() {
        let mut panel = ExpertPanel::new();
        let mut alert = ExpertAlert::new(AlertKind::PodFailure, "pod", "ns", "crash");
        alert.owner_name = Some("web".into());
        alert.owner_kind = Some("Deployment".into());
        panel.push_alert(alert);
        panel.list_state.select(Some(0));
        panel.handle_key(&KeyEvent::from(KeyCode::Enter));
        panel.handle_key(&KeyEvent::from(KeyCode::Char('a')));
        // Navigate down once — should move from ViewLogs (0) to DeletePod (1).
        panel.handle_key(&KeyEvent::from(KeyCode::Down));
        assert_eq!(panel.remediation_selected, 1);
    }

    #[test]
    fn remediation_kind_view_logs_carries_pod_name() {
        let alert = ExpertAlert::new(AlertKind::PodFailure, "crash-pod", "staging", "crash");
        let s = suggestions_for_alert(&alert);
        let logs = s.iter().find(|x| x.label == "View Logs").unwrap();
        assert_eq!(
            logs.kind,
            RemediationKind::ViewLogs {
                name: "crash-pod".into(),
                namespace: "staging".into()
            }
        );
    }
}
