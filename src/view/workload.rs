//! Workload view — Phase 6.21.
//!
//! Displays all workload resources (Deployments, StatefulSets, DaemonSets,
//! ReplicaSets, Jobs, CronJobs) aggregated in a single scrollable table with
//! a KIND prefix column.
//!
//! The view loads its own static snapshot via the browser's
//! `refresh_from_values` API so it works even before the watcher factory is
//! fully connected (it will show an empty table with a hint).
//!
//! # k9s Reference
//! `internal/view/workload.go`

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::client::Gvr;
use crate::render::{ColumnDef, RenderedRow, Renderer};
use crate::ui::table::{RowDelta, TableRow, TableWidget};

/// A synthetic GVR for the workload aggregated view.
pub fn workload_gvr() -> Gvr {
    Gvr::new("", "v1", "workloads")
}

/// Renders a workload summary row: KIND · NAME · NAMESPACE · READY · STATUS · AGE.
///
/// Expects pre-built JSON with fields: `kind`, `name`, `namespace`,
/// `ready`, `status`, `age_secs`.
pub struct WorkloadRenderer;

impl WorkloadRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WorkloadRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for WorkloadRenderer {
    fn gvr(&self) -> &Gvr {
        use std::sync::OnceLock;
        static GVR: OnceLock<Gvr> = OnceLock::new();
        GVR.get_or_init(workload_gvr)
    }

    fn columns(&self) -> &[ColumnDef] {
        static COLS: &[ColumnDef] = &[
            ColumnDef::new("KIND", Constraint::Length(12)),
            ColumnDef::new("NAME", Constraint::Min(24)),
            ColumnDef::new("NAMESPACE", Constraint::Min(14)),
            ColumnDef::new("READY", Constraint::Length(8)),
            ColumnDef::new("STATUS", Constraint::Min(10)),
            ColumnDef::new("AGE", Constraint::Length(6)),
        ];
        COLS
    }

    fn render(&self, obj: &serde_json::Value) -> RenderedRow {
        let kind = obj["kind"].as_str().unwrap_or("").to_owned();
        let name = obj["name"].as_str().unwrap_or("").to_owned();
        let namespace = obj["namespace"].as_str().unwrap_or("").to_owned();
        let ready = obj["ready"].as_str().unwrap_or("").to_owned();
        let status = obj["status"].as_str().unwrap_or("").to_owned();
        let age = obj["age"].as_str().unwrap_or("").to_owned();
        let age_secs = obj["age_secs"].as_u64().unwrap_or(0);

        RenderedRow {
            cells: vec![kind, name, namespace, ready, status, age],
            age_secs,
        }
    }
}

// ─── WorkloadEntry — a normalised workload row ────────────────────────────────

/// A single workload entry for display in the workload view.
#[derive(Debug, Clone)]
pub struct WorkloadEntry {
    pub kind: &'static str,
    pub name: String,
    pub namespace: String,
    pub ready: String,
    pub status: String,
    pub age: String,
    pub age_secs: u64,
}

impl WorkloadEntry {
    /// Convert to JSON for the renderer.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "kind":      self.kind,
            "name":      self.name,
            "namespace": self.namespace,
            "ready":     self.ready,
            "status":    self.status,
            "age":       self.age,
            "age_secs":  self.age_secs,
        })
    }

    /// Build a `WorkloadEntry` from a raw deployment JSON object.
    pub fn from_deployment(obj: &serde_json::Value) -> Self {
        use crate::render::{age_from_obj, meta_name, meta_namespace};
        let name = meta_name(obj).to_owned();
        let ns = meta_namespace(obj).to_owned();
        let desired = obj
            .pointer("/spec/replicas")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let ready = obj
            .pointer("/status/readyReplicas")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let avail = obj
            .pointer("/status/availableReplicas")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let (age, age_secs) = age_from_obj(obj);
        let status = if desired == 0 {
            "Scaled Down".to_owned()
        } else if avail >= desired {
            "Running".to_owned()
        } else if avail == 0 {
            "Unavailable".to_owned()
        } else {
            "Degraded".to_owned()
        };
        Self {
            kind: "Deployment",
            name,
            namespace: ns,
            ready: format!("{ready}/{desired}"),
            status,
            age,
            age_secs,
        }
    }

    /// Build from a StatefulSet JSON.
    pub fn from_statefulset(obj: &serde_json::Value) -> Self {
        use crate::render::{age_from_obj, meta_name, meta_namespace};
        let name = meta_name(obj).to_owned();
        let ns = meta_namespace(obj).to_owned();
        let desired = obj
            .pointer("/spec/replicas")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let ready = obj
            .pointer("/status/readyReplicas")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let (age, age_secs) = age_from_obj(obj);
        let status = if ready >= desired {
            "Running".to_owned()
        } else {
            "Degraded".to_owned()
        };
        Self {
            kind: "StatefulSet",
            name,
            namespace: ns,
            ready: format!("{ready}/{desired}"),
            status,
            age,
            age_secs,
        }
    }

    /// Build from a DaemonSet JSON.
    pub fn from_daemonset(obj: &serde_json::Value) -> Self {
        use crate::render::{age_from_obj, meta_name, meta_namespace};
        let name = meta_name(obj).to_owned();
        let ns = meta_namespace(obj).to_owned();
        let desired = obj
            .pointer("/status/desiredNumberScheduled")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let ready = obj
            .pointer("/status/numberReady")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let (age, age_secs) = age_from_obj(obj);
        let status = if ready >= desired {
            "Running".to_owned()
        } else {
            "Degraded".to_owned()
        };
        Self {
            kind: "DaemonSet",
            name,
            namespace: ns,
            ready: format!("{ready}/{desired}"),
            status,
            age,
            age_secs,
        }
    }
}

// ─── WorkloadView — the full TUI view ────────────────────────────────────────

/// Action returned by [`WorkloadView::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkloadAction {
    Close,
    None,
}

/// Aggregated workload view.
pub struct WorkloadView {
    pub table: TableWidget,
    renderer: WorkloadRenderer,
}

impl WorkloadView {
    pub fn new() -> Self {
        let columns = WorkloadRenderer::new()
            .columns()
            .iter()
            .map(|c| crate::ui::table::Column {
                header: c.name,
                width: c.width,
            })
            .collect();

        Self {
            table: TableWidget::new(columns),
            renderer: WorkloadRenderer::new(),
        }
    }

    /// Replace current rows with entries built from raw JSON slices.
    ///
    /// Each slice is tagged with a kind so the renderer can prefix rows.
    pub fn refresh(
        &mut self,
        deployments: &[serde_json::Value],
        statefulsets: &[serde_json::Value],
        daemonsets: &[serde_json::Value],
    ) {
        let mut rows: Vec<TableRow> = Vec::new();

        for obj in deployments {
            let entry = WorkloadEntry::from_deployment(obj);
            let rendered = self.renderer.render(&entry.to_json());
            rows.push(TableRow {
                cells: rendered.cells,
                delta: RowDelta::Unchanged,
                age_secs: rendered.age_secs,
            });
        }
        for obj in statefulsets {
            let entry = WorkloadEntry::from_statefulset(obj);
            let rendered = self.renderer.render(&entry.to_json());
            rows.push(TableRow {
                cells: rendered.cells,
                delta: RowDelta::Unchanged,
                age_secs: rendered.age_secs,
            });
        }
        for obj in daemonsets {
            let entry = WorkloadEntry::from_daemonset(obj);
            let rendered = self.renderer.render(&entry.to_json());
            rows.push(TableRow {
                cells: rendered.cells,
                delta: RowDelta::Unchanged,
                age_secs: rendered.age_secs,
            });
        }

        self.table.set_rows(rows);
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> WorkloadAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => WorkloadAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.table.up();
                WorkloadAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.table.down();
                WorkloadAction::None
            }
            KeyCode::PageUp => {
                self.table.page_up(20);
                WorkloadAction::None
            }
            KeyCode::PageDown => {
                self.table.page_down(20);
                WorkloadAction::None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.table.top();
                WorkloadAction::None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.table.bottom();
                WorkloadAction::None
            }
            _ => WorkloadAction::None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.table.render(frame, area, "Workloads");
    }
}

impl Default for WorkloadView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workload_renderer_columns() {
        let r = WorkloadRenderer::new();
        assert_eq!(r.columns().len(), 6);
        assert_eq!(r.columns()[0].name, "KIND");
    }

    #[test]
    fn workload_entry_from_deployment() {
        let obj = json!({
            "metadata": {"name": "nginx", "namespace": "default"},
            "spec": {"replicas": 3},
            "status": {"availableReplicas": 3, "readyReplicas": 3}
        });
        let e = WorkloadEntry::from_deployment(&obj);
        assert_eq!(e.kind, "Deployment");
        assert_eq!(e.name, "nginx");
        assert_eq!(e.ready, "3/3");
        assert_eq!(e.status, "Running");
    }

    #[test]
    fn workload_entry_degraded() {
        let obj = json!({
            "metadata": {"name": "api", "namespace": "prod"},
            "spec": {"replicas": 3},
            "status": {"availableReplicas": 1, "readyReplicas": 1}
        });
        let e = WorkloadEntry::from_deployment(&obj);
        assert_eq!(e.status, "Degraded");
    }

    #[test]
    fn workload_entry_from_daemonset() {
        let obj = json!({
            "metadata": {"name": "fluentd", "namespace": "kube-system"},
            "status": {"desiredNumberScheduled": 5, "numberReady": 5}
        });
        let e = WorkloadEntry::from_daemonset(&obj);
        assert_eq!(e.kind, "DaemonSet");
        assert_eq!(e.ready, "5/5");
        assert_eq!(e.status, "Running");
    }

    #[test]
    fn workload_view_refresh_populates_table() {
        let mut view = WorkloadView::new();
        let deploys = vec![json!({
            "metadata": {"name": "app", "namespace": "default"},
            "spec": {"replicas": 1},
            "status": {"availableReplicas": 1, "readyReplicas": 1}
        })];
        view.refresh(&deploys, &[], &[]);
        assert_eq!(view.table.row_count(), 1);
    }

    #[test]
    fn workload_to_json_round_trip() {
        let e = WorkloadEntry {
            kind: "Deployment",
            name: "app".into(),
            namespace: "default".into(),
            ready: "2/2".into(),
            status: "Running".into(),
            age: "5m".into(),
            age_secs: 300,
        };
        let j = e.to_json();
        assert_eq!(j["kind"], "Deployment");
        assert_eq!(j["age_secs"], 300);
    }
}
