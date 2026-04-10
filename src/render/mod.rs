//! Render layer — converts raw Kubernetes JSON objects into display-ready table rows.
//!
//! # Design
//!
//! Every resource type has a dedicated `Renderer` impl.  Renderers take a raw
//! `serde_json::Value` (from the DAO or watcher store) and produce a `RenderedRow`
//! with display cells plus sort metadata.
//!
//! Using JSON values (rather than typed k8s-openapi structs) means:
//! - CRDs and unknown resources work with `GenericRenderer` out of the box.
//! - No additional deserialization round-trip for typed resources.
//! - Renderer logic is testable with plain `serde_json::json!()` literals.

pub mod config_map;
pub mod cron_job;
pub mod daemon_set;
pub mod deployment;
pub mod event;
pub mod generic;
pub mod helm;
pub mod job;
pub mod namespace;
pub mod node;
pub mod pod;
pub mod pv;
pub mod rbac;
pub mod replica_set;
pub mod secret;
pub mod service;
pub mod stateful_set;

pub use config_map::ConfigMapRenderer;
pub use cron_job::CronJobRenderer;
pub use daemon_set::DaemonSetRenderer;
pub use deployment::DeploymentRenderer;
pub use event::EventRenderer;
pub use generic::GenericRenderer;
pub use job::JobRenderer;
pub use namespace::NamespaceRenderer;
pub use node::NodeRenderer;
pub use pod::PodRenderer;
pub use pv::{PvRenderer, PvcRenderer};
pub use rbac::{
    ClusterRoleBindingRenderer, ClusterRoleRenderer, RoleBindingRenderer, RoleRenderer,
};
pub use replica_set::ReplicaSetRenderer;
pub use secret::SecretRenderer;
pub use service::ServiceRenderer;
pub use stateful_set::StatefulSetRenderer;

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;

/// A single column definition in a resource table.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: &'static str,
    pub width: Constraint,
}

impl ColumnDef {
    pub const fn new(name: &'static str, width: Constraint) -> Self {
        Self { name, width }
    }
}

/// A fully rendered table row.
#[derive(Debug, Clone, Default)]
pub struct RenderedRow {
    /// Display strings for each column — must match the renderer's column count.
    pub cells: Vec<String>,
    /// Age in seconds since `creationTimestamp`. Used for age-based sorting.
    pub age_secs: u64,
}

/// Converts a Kubernetes resource JSON value into a displayable table row.
///
/// Implementors are stateless and cheap to construct. Store one per resource
/// type in a registry keyed by GVR.
pub trait Renderer: Send + Sync {
    fn gvr(&self) -> &Gvr;

    /// Column definitions. The table header is derived from these.
    fn columns(&self) -> &[ColumnDef];

    /// Render a single resource into a table row.
    ///
    /// The `obj` value is the full DynamicObject JSON (metadata + spec + status).
    fn render(&self, obj: &Value) -> RenderedRow;
}

// ─── Shared helpers used by all renderers ────────────────────────────────────

/// Format a `creationTimestamp` string into a human-readable age.
///
/// Returns "-" if the timestamp is absent or unparseable.
pub fn age_from_obj(obj: &Value) -> (String, u64) {
    let ts = obj
        .pointer("/metadata/creationTimestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if ts.is_empty() {
        return ("-".to_owned(), 0);
    }

    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => {
            let now = chrono::Utc::now();
            let age = now.signed_duration_since(dt.with_timezone(&chrono::Utc));
            let secs = age.num_seconds().max(0) as u64;
            (format_duration_secs(secs), secs)
        }
        Err(_) => ("-".to_owned(), 0),
    }
}

/// Format a duration in seconds as a compact human-readable string.
///
/// `k9s`-style: `14s`, `5m`, `3h`, `2d`, `1w`.
pub fn format_duration_secs(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else if secs < 7 * 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs < 365 * 86_400 {
        format!("{}w", secs / (7 * 86_400))
    } else {
        format!("{}y", secs / (365 * 86_400))
    }
}

/// Extract `metadata.name` from an object.
pub fn meta_name(obj: &Value) -> &str {
    obj.pointer("/metadata/name")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>")
}

/// Extract `metadata.namespace` from an object.
pub fn meta_namespace(obj: &Value) -> &str {
    obj.pointer("/metadata/namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// Extract a label value.
pub fn label<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    obj.pointer(&format!("/metadata/labels/{key}"))
        .and_then(|v| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_boundaries() {
        assert_eq!(format_duration_secs(0), "0s");
        assert_eq!(format_duration_secs(59), "59s");
        assert_eq!(format_duration_secs(60), "1m");
        assert_eq!(format_duration_secs(3599), "59m");
        assert_eq!(format_duration_secs(3600), "1h");
        assert_eq!(format_duration_secs(86399), "23h");
        assert_eq!(format_duration_secs(86400), "1d");
        assert_eq!(format_duration_secs(6 * 86400), "6d");
        assert_eq!(format_duration_secs(7 * 86400), "1w");
        assert_eq!(format_duration_secs(365 * 86400), "1y");
    }

    #[test]
    fn meta_name_present() {
        let obj = serde_json::json!({"metadata": {"name": "my-pod"}});
        assert_eq!(meta_name(&obj), "my-pod");
    }

    #[test]
    fn meta_name_absent() {
        let obj = serde_json::json!({});
        assert_eq!(meta_name(&obj), "<unknown>");
    }

    #[test]
    fn age_from_obj_missing_timestamp() {
        let obj = serde_json::json!({"metadata": {}});
        let (age, secs) = age_from_obj(&obj);
        assert_eq!(age, "-");
        assert_eq!(secs, 0);
    }
}
