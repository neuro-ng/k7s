//! Resource reference view — "UsedBy" — Phase 10.11.
//!
//! Given a ConfigMap or Secret, scans all pods (and pod-template-bearing
//! workloads) in the namespace to find which ones reference it.
//!
//! A reference can be:
//! - `envFrom[].configMapRef.name` / `envFrom[].secretRef.name`
//! - `volumes[].configMap.name` / `volumes[].secret.secretName`
//! - `containers[].env[].valueFrom.configMapKeyRef.name`
//! - `containers[].env[].valueFrom.secretKeyRef.name`
//!
//! # Design
//!
//! `UsedByView` is populated asynchronously: the caller scans the watcher
//! store (or calls the Kubernetes API) and passes the results in.  The view
//! is purely a display component and owns no live connections.
//!
//! # k9s Reference: `internal/view/reference.go`

use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// The kind of reference found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceKind {
    /// `envFrom[].configMapRef` or `envFrom[].secretRef`
    EnvFrom,
    /// `volumes[].configMap` or `volumes[].secret`
    Volume,
    /// `containers[].env[].valueFrom.configMapKeyRef` or `.secretKeyRef`
    EnvVar { key: String },
}

impl ReferenceKind {
    fn label(&self) -> String {
        match self {
            Self::EnvFrom => "envFrom".to_owned(),
            Self::Volume => "volume".to_owned(),
            Self::EnvVar { key } => format!("env[{key}]"),
        }
    }
}

/// A single usage reference to the scanned ConfigMap or Secret.
#[derive(Debug, Clone)]
pub struct UsageRef {
    /// Referencing resource kind (e.g. `"Pod"`, `"Deployment"`).
    pub kind: String,
    /// Namespace of the referencing resource.
    pub namespace: String,
    /// Name of the referencing resource.
    pub name: String,
    /// Container within the pod/workload (for env/envFrom refs).
    pub container: Option<String>,
    /// How it references the ConfigMap/Secret.
    pub reference_kind: ReferenceKind,
}

impl UsageRef {
    /// One-line display string.
    pub fn display(&self) -> String {
        let container_part = self
            .container
            .as_deref()
            .map(|c| format!("/{c}"))
            .unwrap_or_default();
        format!(
            "{}/{}{} [{}]",
            self.kind,
            self.name,
            container_part,
            self.reference_kind.label()
        )
    }
}

// ─── Scanner ──────────────────────────────────────────────────────────────────

/// Scan a slice of raw pod JSON values for references to a named ConfigMap or
/// Secret.
///
/// Call once per workload resource type (pods, deployments, statefulsets…).
/// Pass the pod-template section for non-Pod workloads.
///
/// `target_name` is the ConfigMap or Secret name to search for.
/// `is_secret` controls whether to look for secret refs (true) or configmap refs (false).
pub fn scan_pods_for_refs(
    pods: &[serde_json::Value],
    target_name: &str,
    is_secret: bool,
    kind: &str,
) -> Vec<UsageRef> {
    let mut results = Vec::new();

    for pod in pods {
        let name = pod
            .pointer("/metadata/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let namespace = pod
            .pointer("/metadata/namespace")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_owned();

        // For workloads (Deployment, etc.) the pod template is nested under
        // /spec/template/spec; for bare Pods it's /spec.
        let spec_root = if kind == "Pod" {
            "/spec"
        } else {
            "/spec/template/spec"
        };

        scan_spec(
            pod,
            spec_root,
            target_name,
            is_secret,
            kind,
            &namespace,
            &name,
            &mut results,
        );
    }

    results
}

#[allow(clippy::too_many_arguments)]
fn scan_spec(
    obj: &serde_json::Value,
    spec_path: &str,
    target_name: &str,
    is_secret: bool,
    kind: &str,
    namespace: &str,
    name: &str,
    out: &mut Vec<UsageRef>,
) {
    // ── volumes ──────────────────────────────────────────────────────────────
    if let Some(volumes) = obj
        .pointer(&format!("{spec_path}/volumes"))
        .and_then(|v| v.as_array())
    {
        for vol in volumes {
            let matched = if is_secret {
                vol.pointer("/secret/secretName")
                    .and_then(|v| v.as_str())
                    .map(|n| n == target_name)
                    .unwrap_or(false)
            } else {
                vol.pointer("/configMap/name")
                    .and_then(|v| v.as_str())
                    .map(|n| n == target_name)
                    .unwrap_or(false)
            };
            if matched {
                out.push(UsageRef {
                    kind: kind.to_owned(),
                    namespace: namespace.to_owned(),
                    name: name.to_owned(),
                    container: None,
                    reference_kind: ReferenceKind::Volume,
                });
            }
        }
    }

    // ── containers (and initContainers) ──────────────────────────────────────
    for container_path in &[
        format!("{spec_path}/containers"),
        format!("{spec_path}/initContainers"),
    ] {
        let Some(containers) = obj.pointer(container_path).and_then(|v| v.as_array()) else {
            continue;
        };
        for container in containers {
            let container_name = container
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();

            // envFrom
            if let Some(env_from) = container.get("envFrom").and_then(|v| v.as_array()) {
                for ef in env_from {
                    let matched = if is_secret {
                        ef.pointer("/secretRef/name")
                            .and_then(|v| v.as_str())
                            .map(|n| n == target_name)
                            .unwrap_or(false)
                    } else {
                        ef.pointer("/configMapRef/name")
                            .and_then(|v| v.as_str())
                            .map(|n| n == target_name)
                            .unwrap_or(false)
                    };
                    if matched {
                        out.push(UsageRef {
                            kind: kind.to_owned(),
                            namespace: namespace.to_owned(),
                            name: name.to_owned(),
                            container: Some(container_name.clone()),
                            reference_kind: ReferenceKind::EnvFrom,
                        });
                    }
                }
            }

            // env[].valueFrom
            if let Some(env) = container.get("env").and_then(|v| v.as_array()) {
                for ev in env {
                    let env_var_name = ev
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let matched = if is_secret {
                        ev.pointer("/valueFrom/secretKeyRef/name")
                            .and_then(|v| v.as_str())
                            .map(|n| n == target_name)
                            .unwrap_or(false)
                    } else {
                        ev.pointer("/valueFrom/configMapKeyRef/name")
                            .and_then(|v| v.as_str())
                            .map(|n| n == target_name)
                            .unwrap_or(false)
                    };
                    if matched {
                        out.push(UsageRef {
                            kind: kind.to_owned(),
                            namespace: namespace.to_owned(),
                            name: name.to_owned(),
                            container: Some(container_name.clone()),
                            reference_kind: ReferenceKind::EnvVar { key: env_var_name },
                        });
                    }
                }
            }
        }
    }
}

// ─── TUI view ─────────────────────────────────────────────────────────────────

/// Fullscreen overlay showing which resources reference a ConfigMap or Secret.
pub struct UsedByView {
    /// The resource being inspected.
    pub resource_kind: String,
    pub resource_name: String,
    /// Results from the scanner.
    pub refs: Vec<UsageRef>,
    state: ListState,
}

impl UsedByView {
    pub fn new(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            resource_kind: kind.into(),
            resource_name: name.into(),
            refs: Vec::new(),
            state: ListState::default(),
        }
    }

    /// Replace the displayed references.
    pub fn set_refs(&mut self, refs: Vec<UsageRef>) {
        self.refs = refs;
        if !self.refs.is_empty() {
            self.state.select(Some(0));
        } else {
            self.state.select(None);
        }
    }

    pub fn up(&mut self) {
        let i = self
            .state
            .selected()
            .map(|s| s.saturating_sub(1))
            .unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn down(&mut self) {
        if self.refs.is_empty() {
            return;
        }
        let i = self
            .state
            .selected()
            .map(|s| (s + 1).min(self.refs.len() - 1))
            .unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" Used By — {}/{} ", self.resource_kind, self.resource_name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.refs.is_empty() {
            frame.render_widget(
                Paragraph::new("  No references found in this namespace.")
                    .style(Style::default().fg(Color::DarkGray)),
                inner,
            );
            return;
        }

        let items: Vec<ListItem> = self
            .refs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let selected = self.state.selected() == Some(i);
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(
                    format!("  {}", r.display()),
                    style,
                )))
            })
            .collect();

        let list = ratatui::widgets::List::new(items).highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        frame.render_stateful_widget(list, inner, &mut self.state);
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pod(name: &str, ns: &str, spec: serde_json::Value) -> serde_json::Value {
        json!({
            "metadata": { "name": name, "namespace": ns },
            "spec": spec
        })
    }

    #[test]
    fn detects_env_from_configmap() {
        let p = pod(
            "api",
            "default",
            json!({
                "containers": [{
                    "name": "app",
                    "envFrom": [{ "configMapRef": { "name": "app-config" } }]
                }]
            }),
        );
        let refs = scan_pods_for_refs(&[p], "app-config", false, "Pod");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "api");
        assert_eq!(refs[0].reference_kind, ReferenceKind::EnvFrom);
    }

    #[test]
    fn detects_env_from_secret() {
        let p = pod(
            "db",
            "default",
            json!({
                "containers": [{
                    "name": "app",
                    "envFrom": [{ "secretRef": { "name": "db-creds" } }]
                }]
            }),
        );
        let refs = scan_pods_for_refs(&[p], "db-creds", true, "Pod");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].reference_kind, ReferenceKind::EnvFrom);
    }

    #[test]
    fn detects_volume_configmap() {
        let p = pod(
            "proxy",
            "default",
            json!({
                "volumes": [{ "name": "cfg", "configMap": { "name": "nginx-conf" } }],
                "containers": [{ "name": "nginx" }]
            }),
        );
        let refs = scan_pods_for_refs(&[p], "nginx-conf", false, "Pod");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].reference_kind, ReferenceKind::Volume);
        assert!(refs[0].container.is_none());
    }

    #[test]
    fn detects_volume_secret() {
        let p = pod(
            "tls-pod",
            "default",
            json!({
                "volumes": [{ "name": "tls", "secret": { "secretName": "my-tls" } }],
                "containers": [{ "name": "app" }]
            }),
        );
        let refs = scan_pods_for_refs(&[p], "my-tls", true, "Pod");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].reference_kind, ReferenceKind::Volume);
    }

    #[test]
    fn detects_env_value_from_configmap_key() {
        let p = pod(
            "worker",
            "default",
            json!({
                "containers": [{
                    "name": "main",
                    "env": [{
                        "name": "LOG_LEVEL",
                        "valueFrom": { "configMapKeyRef": { "name": "app-config", "key": "logLevel" } }
                    }]
                }]
            }),
        );
        let refs = scan_pods_for_refs(&[p], "app-config", false, "Pod");
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0].reference_kind,
            ReferenceKind::EnvVar {
                key: "LOG_LEVEL".to_owned()
            }
        );
    }

    #[test]
    fn no_match_returns_empty() {
        let p = pod(
            "other",
            "default",
            json!({ "containers": [{ "name": "app" }] }),
        );
        let refs = scan_pods_for_refs(&[p], "nonexistent", false, "Pod");
        assert!(refs.is_empty());
    }

    #[test]
    fn multiple_refs_in_one_pod() {
        let p = pod(
            "multi",
            "default",
            json!({
                "volumes": [{ "name": "cfg", "configMap": { "name": "shared-cfg" } }],
                "containers": [{
                    "name": "app",
                    "envFrom": [{ "configMapRef": { "name": "shared-cfg" } }]
                }]
            }),
        );
        let refs = scan_pods_for_refs(&[p], "shared-cfg", false, "Pod");
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn usage_ref_display_with_container() {
        let r = UsageRef {
            kind: "Pod".to_owned(),
            namespace: "default".to_owned(),
            name: "my-pod".to_owned(),
            container: Some("app".to_owned()),
            reference_kind: ReferenceKind::EnvFrom,
        };
        let d = r.display();
        assert!(d.contains("Pod/my-pod/app"));
        assert!(d.contains("envFrom"));
    }

    #[test]
    fn usage_ref_display_no_container() {
        let r = UsageRef {
            kind: "Pod".to_owned(),
            namespace: "default".to_owned(),
            name: "my-pod".to_owned(),
            container: None,
            reference_kind: ReferenceKind::Volume,
        };
        let d = r.display();
        assert!(d.contains("Pod/my-pod [volume]"));
    }

    #[test]
    fn view_set_refs_selects_first() {
        let mut view = UsedByView::new("ConfigMap", "app-config");
        view.set_refs(vec![UsageRef {
            kind: "Pod".to_owned(),
            namespace: "default".to_owned(),
            name: "p1".to_owned(),
            container: None,
            reference_kind: ReferenceKind::Volume,
        }]);
        assert_eq!(view.state.selected(), Some(0));
    }

    #[test]
    fn view_empty_refs_no_selection() {
        let mut view = UsedByView::new("Secret", "db-creds");
        view.set_refs(vec![]);
        assert_eq!(view.state.selected(), None);
    }
}
