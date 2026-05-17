//! KubeVela / OAM data structures and kube API client — Phase 36.
//!
//! All read operations query the Kubernetes API directly (no `vela` binary).
//! Mutating operations (restart, resume, rollback, delete) are handled by the
//! caller shelling out to the `vela` CLI via a subprocess.

pub mod client;

// ─── Application ─────────────────────────────────────────────────────────────

/// A KubeVela Application resource (`core.oam.dev/v1beta1/applications`).
#[derive(Debug, Clone)]
pub struct VelaApplication {
    /// Application name.
    pub name: String,
    /// Namespace.
    pub namespace: String,
    /// Overall application status (e.g. `"running"`, `"workflowFailed"`).
    pub status: String,
    /// Number of components in the application spec.
    pub component_count: usize,
    /// Workflow phase (e.g. `"succeeded"`, `"running"`, `"failed"`), or empty.
    pub workflow_status: String,
    /// Age in seconds since `creationTimestamp`.
    pub age_secs: u64,
    /// Full raw Application CR JSON (for detail views).
    pub raw: serde_json::Value,
}

impl VelaApplication {
    /// Human-readable age label.
    pub fn age_label(&self) -> String {
        let s = self.age_secs;
        if s < 120 {
            format!("{s}s")
        } else if s < 7200 {
            format!("{}m", s / 60)
        } else if s < 172_800 {
            format!("{}h", s / 3600)
        } else {
            format!("{}d", s / 86400)
        }
    }
}

// ─── Component ───────────────────────────────────────────────────────────────

/// A single OAM component within an Application.
#[derive(Debug, Clone)]
pub struct VelaComponent {
    pub name: String,
    pub workload_type: String,
    /// `true` if `status.services[].healthy == true`.
    pub healthy: bool,
    pub message: String,
    pub traits: Vec<VelaTrait>,
}

// ─── Trait ────────────────────────────────────────────────────────────────────

/// An OAM trait attached to a component.
#[derive(Debug, Clone)]
pub struct VelaTrait {
    pub trait_type: String,
    pub healthy: bool,
    pub message: String,
}

// ─── Workflow step ────────────────────────────────────────────────────────────

/// A single step in a KubeVela workflow execution.
#[derive(Debug, Clone)]
pub struct VelaWorkflowStep {
    pub name: String,
    pub step_type: String,
    /// Phase: `"running"`, `"succeeded"`, `"failed"`, `"skipped"`, `"stopped"`.
    pub phase: String,
    pub message: String,
}

// ─── Revision ─────────────────────────────────────────────────────────────────

/// An `ApplicationRevision` resource — one entry in the rollout history.
#[derive(Debug, Clone)]
pub struct VelaRevision {
    /// Kubernetes resource name (e.g. `"my-app-v3"`).
    pub name: String,
    /// Numeric revision extracted from the resource name or label.
    pub revision: u64,
    pub deploy_time: String,
    /// `"succeeded"` / `"failed"` / `"unknown"`.
    pub status: String,
}

// ─── Definition ───────────────────────────────────────────────────────────────

/// A KubeVela capability definition (ComponentDefinition, TraitDefinition, etc.)
#[derive(Debug, Clone)]
pub struct VelaDefinition {
    pub name: String,
    /// `"Component"`, `"Trait"`, `"WorkflowStep"`, or `"Policy"`.
    pub def_type: String,
    /// Short description from `annotations["definition.oam.dev/description"]`.
    pub description: String,
}

// ─── Parser helpers ───────────────────────────────────────────────────────────

/// Extract `Vec<VelaComponent>` from a raw Application CR JSON.
///
/// Merges spec component types with `status.services[]` health data.
pub fn parse_components(raw: &serde_json::Value) -> Vec<VelaComponent> {
    // Build a lookup from component name → health/traits from status.services.
    let mut health_map: std::collections::HashMap<String, (bool, String, Vec<VelaTrait>)> =
        std::collections::HashMap::new();

    if let Some(services) = raw.pointer("/status/services").and_then(|v| v.as_array()) {
        for svc in services {
            let name = str_or(svc, "name");
            let healthy = svc
                .get("healthy")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let message = str_or(svc, "message");
            let traits = svc
                .get("traits")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|t| VelaTrait {
                            trait_type: str_or(t, "type"),
                            healthy: t.get("healthy").and_then(|v| v.as_bool()).unwrap_or(false),
                            message: str_or(t, "message"),
                        })
                        .collect()
                })
                .unwrap_or_default();
            health_map.insert(name, (healthy, message, traits));
        }
    }

    // Walk spec.components for type info.
    let empty = vec![];
    let spec_components = raw
        .pointer("/spec/components")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);

    spec_components
        .iter()
        .map(|c| {
            let name = str_or(c, "name");
            let workload_type = str_or(c, "type");
            let (healthy, message, traits) =
                health_map
                    .remove(&name)
                    .unwrap_or((false, String::new(), vec![]));
            VelaComponent {
                name,
                workload_type,
                healthy,
                message,
                traits,
            }
        })
        .collect()
}

/// Extract `Vec<VelaWorkflowStep>` from a raw Application CR JSON.
pub fn parse_workflow_steps(raw: &serde_json::Value) -> Vec<VelaWorkflowStep> {
    let empty = vec![];
    let steps = raw
        .pointer("/status/workflow/steps")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);

    steps
        .iter()
        .map(|s| VelaWorkflowStep {
            name: str_or(s, "name"),
            step_type: str_or(s, "type"),
            phase: str_or(s, "phase"),
            message: str_or(s, "message"),
        })
        .collect()
}

fn str_or(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_app() -> serde_json::Value {
        json!({
            "spec": {
                "components": [
                    {"name": "webservice", "type": "webservice"},
                    {"name": "worker", "type": "worker"}
                ]
            },
            "status": {
                "status": "running",
                "services": [
                    {
                        "name": "webservice",
                        "healthy": true,
                        "message": "",
                        "traits": [
                            {"type": "scaler", "healthy": true, "message": ""}
                        ]
                    },
                    {
                        "name": "worker",
                        "healthy": false,
                        "message": "OOMKill",
                        "traits": []
                    }
                ],
                "workflow": {
                    "status": "succeeded",
                    "steps": [
                        {"name": "deploy", "type": "deploy", "phase": "succeeded", "message": ""},
                        {"name": "notify", "type": "notification", "phase": "succeeded", "message": ""}
                    ]
                }
            }
        })
    }

    #[test]
    fn parse_components_count() {
        let components = parse_components(&sample_app());
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn parse_components_health() {
        let components = parse_components(&sample_app());
        let ws = components.iter().find(|c| c.name == "webservice").unwrap();
        assert!(ws.healthy);
        let worker = components.iter().find(|c| c.name == "worker").unwrap();
        assert!(!worker.healthy);
        assert_eq!(worker.message, "OOMKill");
    }

    #[test]
    fn parse_components_traits() {
        let components = parse_components(&sample_app());
        let ws = components.iter().find(|c| c.name == "webservice").unwrap();
        assert_eq!(ws.traits.len(), 1);
        assert_eq!(ws.traits[0].trait_type, "scaler");
        assert!(ws.traits[0].healthy);
    }

    #[test]
    fn parse_workflow_steps_count() {
        let steps = parse_workflow_steps(&sample_app());
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn parse_workflow_steps_phase() {
        let steps = parse_workflow_steps(&sample_app());
        assert_eq!(steps[0].name, "deploy");
        assert_eq!(steps[0].phase, "succeeded");
    }

    #[test]
    fn age_label_seconds() {
        let app = VelaApplication {
            name: "x".into(),
            namespace: "default".into(),
            status: "running".into(),
            component_count: 1,
            workflow_status: "succeeded".into(),
            age_secs: 45,
            raw: json!({}),
        };
        assert_eq!(app.age_label(), "45s");
    }

    #[test]
    fn age_label_hours() {
        let app = VelaApplication {
            name: "x".into(),
            namespace: "default".into(),
            status: "running".into(),
            component_count: 1,
            workflow_status: String::new(),
            age_secs: 7200,
            raw: json!({}),
        };
        assert_eq!(app.age_label(), "2h");
    }

    #[test]
    fn age_label_days() {
        let app = VelaApplication {
            name: "x".into(),
            namespace: "default".into(),
            status: "running".into(),
            component_count: 1,
            workflow_status: String::new(),
            age_secs: 86_400 * 3,
            raw: json!({}),
        };
        assert_eq!(app.age_label(), "3d");
    }

    #[test]
    fn parse_empty_app_returns_empty_components() {
        let raw = json!({"spec": {}, "status": {}});
        assert!(parse_components(&raw).is_empty());
        assert!(parse_workflow_steps(&raw).is_empty());
    }
}
