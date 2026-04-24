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
                        return Some(ExpertAlert::new(
                            AlertKind::PodFailure,
                            &name,
                            &ns,
                            format!("CrashLoopBackOff (restarts: {restarts})"),
                        ));
                    }
                    "OOMKilled" | "Error" => {
                        return Some(ExpertAlert::new(
                            AlertKind::PodFailure,
                            &name,
                            &ns,
                            format!("Container terminated: {reason}"),
                        ));
                    }
                    "ImagePullBackOff" | "ErrImagePull" => {
                        return Some(ExpertAlert::new(
                            AlertKind::PodFailure,
                            &name,
                            &ns,
                            format!("Image pull failure: {reason}"),
                        ));
                    }
                    _ => {}
                }

                // OOMKilled appears in last state terminated.
                let last_reason = cs
                    .pointer("/lastState/terminated/reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if last_reason == "OOMKilled" {
                    return Some(ExpertAlert::new(
                        AlertKind::PodFailure,
                        &name,
                        &ns,
                        "OOMKilled (out of memory)".to_string(),
                    ));
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
            return Some(ExpertAlert::new(
                AlertKind::PodFailure,
                &name,
                &ns,
                format!("Pod Failed: {reason}"),
            ));
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
}

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
}

impl ExpertPanel {
    pub fn new() -> Self {
        Self {
            alerts: VecDeque::new(),
            list_state: ListState::default(),
            show_detail: false,
            detail_scroll: 0,
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
            KeyCode::Esc | KeyCode::Char('q') if !self.show_detail => {
                return ExpertAction::Close;
            }
            KeyCode::Esc if self.show_detail => {
                self.show_detail = false;
                self.detail_scroll = 0;
                return ExpertAction::Noop;
            }
            KeyCode::Enter | KeyCode::Char(' ') if !self.show_detail => {
                if let Some(idx) = self.list_state.selected() {
                    self.show_detail = true;
                    self.detail_scroll = 0;
                    return ExpertAction::SelectAlert(idx);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(idx) = self.list_state.selected() {
                    self.alerts.remove(idx);
                    if self.alerts.is_empty() {
                        self.list_state.select(None);
                    } else {
                        let new_idx = idx.min(self.alerts.len().saturating_sub(1));
                        self.list_state.select(Some(new_idx));
                    }
                    self.show_detail = false;
                    return ExpertAction::Dismiss;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.show_detail {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                } else {
                    self.move_selection(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.show_detail {
                    self.detail_scroll += 1;
                } else {
                    self.move_selection(1);
                }
            }
            KeyCode::Char('r') => {
                return ExpertAction::Rescan;
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

        frame.render_widget(para, inner);
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
}
