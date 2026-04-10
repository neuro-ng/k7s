//! Browser view — the main resource list screen.
//!
//! Connects three pieces:
//!   1. A `Store<DynamicObject>` (live data from the watcher factory)
//!   2. A `Renderer` (converts raw JSON → table cells)
//!   3. A `TableWidget` (handles display and user navigation)
//!
//! The view is deliberately stateless w.r.t. networking — it reads from the
//! store on every render tick, diffing against the previous snapshot to mark
//! Added / Modified / Deleted rows.

use std::collections::HashMap;
use std::sync::Arc;

use kube::api::DynamicObject;
use kube::runtime::reflector::Store;
use ratatui::layout::{Constraint, Rect};
use ratatui::Frame;

use crate::render::{ColumnDef, Renderer};
use crate::ui::table::{RowDelta, TableRow, TableWidget};

/// The browser view for a single resource type.
pub struct BrowserView {
    /// The resource type being browsed (display name).
    pub title: String,
    /// The renderer for this resource type.
    renderer: Box<dyn Renderer>,
    /// The table widget that handles display and selection.
    pub table: TableWidget,
    /// Previous snapshot of object UIDs → resource version for delta tracking.
    prev_versions: HashMap<String, String>,
}

impl BrowserView {
    pub fn new(title: String, renderer: Box<dyn Renderer>) -> Self {
        let columns = renderer
            .columns()
            .iter()
            .map(|c| crate::ui::table::Column {
                header: c.name,
                width: c.width,
            })
            .collect();

        Self {
            title,
            renderer,
            table: TableWidget::new(columns),
            prev_versions: HashMap::new(),
        }
    }

    /// Refresh the table from a live watcher store.
    ///
    /// This is called on every render tick. It:
    /// 1. Reads all objects from the store snapshot.
    /// 2. Converts each to a rendered row via the renderer.
    /// 3. Marks rows as Added / Modified / Deleted compared to the previous tick.
    /// 4. Updates the table widget.
    pub fn refresh_from_store(&mut self, store: &Store<DynamicObject>) {
        let objects = store.state();

        let mut new_versions: HashMap<String, String> = HashMap::new();
        let mut rows: Vec<TableRow> = Vec::with_capacity(objects.len());

        for obj_arc in &objects {
            let uid = obj_arc
                .metadata
                .uid
                .clone()
                .unwrap_or_else(|| obj_arc.metadata.name.clone().unwrap_or_default());

            let rv = obj_arc
                .metadata
                .resource_version
                .clone()
                .unwrap_or_default();

            new_versions.insert(uid.clone(), rv.clone());

            // Serialize to JSON for the renderer.
            let value = match serde_json::to_value(obj_arc.as_ref()) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to serialize DynamicObject for rendering");
                    continue;
                }
            };

            let rendered = self.renderer.render(&value);

            let delta = match self.prev_versions.get(&uid) {
                None => RowDelta::Added,
                Some(prev_rv) if prev_rv != &rv => RowDelta::Modified,
                _ => RowDelta::Unchanged,
            };

            rows.push(TableRow {
                cells: rendered.cells,
                delta,
                age_secs: rendered.age_secs,
            });
        }

        // Mark UIDs that disappeared as Deleted (briefly shown, then removed).
        // For now we just remove them; a future enhancement can keep them for one tick.

        self.prev_versions = new_versions;
        self.table.set_rows(rows);
    }

    /// Refresh from a static list (used when no watcher is running, e.g. tests).
    pub fn refresh_from_values(&mut self, values: &[serde_json::Value]) {
        let rows = values
            .iter()
            .map(|v| {
                let rendered = self.renderer.render(v);
                TableRow {
                    cells: rendered.cells,
                    delta: RowDelta::Unchanged,
                    age_secs: rendered.age_secs,
                }
            })
            .collect();
        self.table.set_rows(rows);
    }

    /// Render the browser into the frame area.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.table.render(frame, area, &self.title.clone());
    }

    /// Forward cursor movement to the table.
    pub fn up(&mut self) { self.table.up(); }
    pub fn down(&mut self) { self.table.down(); }
    pub fn page_up(&mut self) { self.table.page_up(20); }
    pub fn page_down(&mut self) { self.table.page_down(20); }
    pub fn top(&mut self) { self.table.top(); }
    pub fn bottom(&mut self) { self.table.bottom(); }

    pub fn set_filter(&mut self, f: String) { self.table.set_filter(f); }
    pub fn clear_filter(&mut self) { self.table.set_filter(String::new()); }

    /// The name of the currently selected resource, if any.
    pub fn selected_name(&self) -> Option<String> {
        self.table.selected_row().and_then(|r| r.cells.first().cloned())
    }
}

/// Build a `BrowserView` for a named resource type.
///
/// Returns `None` if the resource type is not recognised.
pub fn browser_for_resource(alias: &str, registry: &crate::dao::Registry) -> Option<BrowserView> {
    use crate::render::{
        CronJobRenderer, DaemonSetRenderer, DeploymentRenderer, GenericRenderer, JobRenderer,
        NamespaceRenderer, NodeRenderer, PodRenderer, ReplicaSetRenderer, ServiceRenderer,
        StatefulSetRenderer,
    };
    use crate::client::gvr::well_known;

    let meta = registry.get_by_alias(alias)?;
    let gvr = meta.gvr.clone();
    let title = meta.display_name.clone();
    let namespaced = meta.namespaced;

    let renderer: Box<dyn Renderer> = match gvr {
        g if g == well_known::pods()         => Box::new(PodRenderer::new()),
        g if g == well_known::deployments()  => Box::new(DeploymentRenderer::new()),
        g if g == well_known::stateful_sets()=> Box::new(StatefulSetRenderer::new()),
        g if g == well_known::daemon_sets()  => Box::new(DaemonSetRenderer::new()),
        g if g == well_known::replica_sets() => Box::new(ReplicaSetRenderer::new()),
        g if g == well_known::jobs()         => Box::new(JobRenderer::new()),
        g if g == well_known::cron_jobs()    => Box::new(CronJobRenderer::new()),
        g if g == well_known::namespaces()   => Box::new(NamespaceRenderer::new()),
        g if g == well_known::nodes()        => Box::new(NodeRenderer::new()),
        g if g == well_known::services()     => Box::new(ServiceRenderer::new()),
        g => Box::new(GenericRenderer::new(g, namespaced)),
    };

    Some(BrowserView::new(title, renderer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dao::Registry;
    use serde_json::json;

    #[test]
    fn browser_for_pods_alias() {
        let reg = Registry::with_builtins();
        let browser = browser_for_resource("po", &reg);
        assert!(browser.is_some(), "po alias should resolve");
    }

    #[test]
    fn browser_for_unknown_returns_none() {
        let reg = Registry::with_builtins();
        assert!(browser_for_resource("nonexistent", &reg).is_none());
    }

    #[test]
    fn refresh_from_values_populates_table() {
        let reg = Registry::with_builtins();
        let mut browser = browser_for_resource("pods", &reg).unwrap();

        let pods = vec![
            json!({
                "metadata": {"name": "pod-a"},
                "status": {"phase": "Running", "containerStatuses": [{"ready": true, "restartCount": 0, "state": {"running": {}}}]}
            }),
            json!({
                "metadata": {"name": "pod-b"},
                "status": {"phase": "Pending"}
            }),
        ];

        browser.refresh_from_values(&pods);
        assert_eq!(browser.table.row_count(), 2);
    }

    #[test]
    fn selected_name_returns_first_cell() {
        let reg = Registry::with_builtins();
        let mut browser = browser_for_resource("pods", &reg).unwrap();
        browser.refresh_from_values(&[json!({"metadata": {"name": "my-pod"}})]);
        assert_eq!(browser.selected_name(), Some("my-pod".to_owned()));
    }
}
