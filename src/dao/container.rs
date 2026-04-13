//! Container sub-resource DAO — Phase 3.13.
//!
//! Exposes the containers (and init containers) of a specific pod as a
//! queryable list.  This is a *sub-resource* of pods: there is no dedicated
//! K8s API endpoint; the data is extracted from the parent pod's spec and
//! status.
//!
//! # k9s Reference
//! `internal/dao/container.go`

use serde_json::Value;

use crate::client::Gvr;

/// A synthetic GVR for containers (not a real K8s resource type).
pub fn container_gvr() -> Gvr {
    Gvr::new("", "v1", "containers")
}

/// Type of container within a pod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    /// A regular workload container.
    Regular,
    /// An init container (runs before regular containers).
    Init,
    /// An ephemeral (debug) container.
    Ephemeral,
}

impl ContainerKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Regular => "C",
            Self::Init => "I",
            Self::Ephemeral => "E",
        }
    }
}

/// Information about one container within a pod, derived from spec + status.
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    pub kind: ContainerKind,
    pub ready: bool,
    pub state: String,
    pub restart_count: u32,
    pub cpu_request: String,
    pub mem_request: String,
    pub cpu_limit: String,
    pub mem_limit: String,
}

impl ContainerInfo {
    /// Synthesize a JSON object the `ContainerRenderer` can consume.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "name":          self.name,
            "image":         self.image,
            "kind":          self.kind.label(),
            "ready":         if self.ready { "true" } else { "false" },
            "state":         self.state,
            "restarts":      self.restart_count,
            "cpu_request":   self.cpu_request,
            "mem_request":   self.mem_request,
            "cpu_limit":     self.cpu_limit,
            "mem_limit":     self.mem_limit,
        })
    }
}

/// Extract all containers (regular, init, ephemeral) from a raw pod JSON.
///
/// Returns an empty list when the pod JSON is malformed or has no containers.
pub fn containers_from_pod(pod: &Value) -> Vec<ContainerInfo> {
    let mut out = Vec::new();

    // Build a fast lookup of status by container name.
    let statuses = container_statuses(pod, "/status/containerStatuses");
    let init_statuses = container_statuses(pod, "/status/initContainerStatuses");
    let ephemeral_statuses = container_statuses(pod, "/status/ephemeralContainerStatuses");

    for (kind, spec_path, status_map) in [
        (ContainerKind::Regular, "/spec/containers", &statuses),
        (ContainerKind::Init, "/spec/initContainers", &init_statuses),
        (
            ContainerKind::Ephemeral,
            "/spec/ephemeralContainers",
            &ephemeral_statuses,
        ),
    ] {
        let Some(arr) = pod.pointer(spec_path).and_then(|v| v.as_array()) else {
            continue;
        };

        for spec in arr {
            let name = spec
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let image = spec
                .get("image")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();

            let (ready, state, restarts) = if let Some(s) = status_map.get(name.as_str()) {
                let ready = s.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
                let state = container_state_label(s);
                let restarts = s.get("restartCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                (ready, state, restarts)
            } else {
                (false, "Waiting".to_owned(), 0)
            };

            let (cpu_req, mem_req, cpu_lim, mem_lim) = resource_requests(spec);

            out.push(ContainerInfo {
                name,
                image,
                kind,
                ready,
                state,
                restart_count: restarts,
                cpu_request: cpu_req,
                mem_request: mem_req,
                cpu_limit: cpu_lim,
                mem_limit: mem_lim,
            });
        }
    }

    out
}

// ─── Private helpers ─────────────────────────────────────────────────────────

/// Build a name → status object map from a pod's `*ContainerStatuses` array.
fn container_statuses<'a>(
    pod: &'a Value,
    path: &str,
) -> std::collections::HashMap<&'a str, &'a Value> {
    let mut map = std::collections::HashMap::new();
    if let Some(arr) = pod.pointer(path).and_then(|v| v.as_array()) {
        for s in arr {
            if let Some(name) = s.get("name").and_then(|v| v.as_str()) {
                map.insert(name, s);
            }
        }
    }
    map
}

/// Derive a human-readable state label from a container status object.
fn container_state_label(status: &Value) -> String {
    let state = match status.get("state") {
        Some(s) => s,
        None => return "Unknown".to_owned(),
    };

    if state.get("running").is_some() {
        return "Running".to_owned();
    }
    if let Some(terminated) = state.get("terminated") {
        let reason = terminated
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Terminated");
        return reason.to_owned();
    }
    if let Some(waiting) = state.get("waiting") {
        let reason = waiting
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Waiting");
        return reason.to_owned();
    }
    "Unknown".to_owned()
}

/// Extract resource requests and limits from a container spec.
/// Returns (cpu_req, mem_req, cpu_lim, mem_lim).
fn resource_requests(spec: &Value) -> (String, String, String, String) {
    let req = spec.pointer("/resources/requests");
    let lim = spec.pointer("/resources/limits");

    let cpu_req = req
        .and_then(|r| r.get("cpu"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let mem_req = req
        .and_then(|r| r.get("memory"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let cpu_lim = lim
        .and_then(|r| r.get("cpu"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let mem_lim = lim
        .and_then(|r| r.get("memory"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    (cpu_req, mem_req, cpu_lim, mem_lim)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn running_pod() -> Value {
        json!({
            "spec": {
                "containers": [
                    {
                        "name": "app",
                        "image": "nginx:latest",
                        "resources": {
                            "requests": {"cpu": "100m", "memory": "128Mi"},
                            "limits":   {"cpu": "500m", "memory": "256Mi"}
                        }
                    },
                    {
                        "name": "sidecar",
                        "image": "envoy:v1.28"
                    }
                ],
                "initContainers": [
                    {"name": "init-db", "image": "alpine:3.18"}
                ]
            },
            "status": {
                "containerStatuses": [
                    {
                        "name": "app",
                        "ready": true,
                        "restartCount": 2,
                        "state": {"running": {}}
                    },
                    {
                        "name": "sidecar",
                        "ready": true,
                        "restartCount": 0,
                        "state": {"running": {}}
                    }
                ],
                "initContainerStatuses": [
                    {
                        "name": "init-db",
                        "ready": true,
                        "restartCount": 0,
                        "state": {"terminated": {"reason": "Completed"}}
                    }
                ]
            }
        })
    }

    #[test]
    fn extracts_regular_containers() {
        let containers = containers_from_pod(&running_pod());
        let regulars: Vec<_> = containers
            .iter()
            .filter(|c| c.kind == ContainerKind::Regular)
            .collect();
        assert_eq!(regulars.len(), 2);
        assert_eq!(regulars[0].name, "app");
        assert_eq!(regulars[0].image, "nginx:latest");
        assert!(regulars[0].ready);
        assert_eq!(regulars[0].state, "Running");
        assert_eq!(regulars[0].restart_count, 2);
    }

    #[test]
    fn extracts_init_containers() {
        let containers = containers_from_pod(&running_pod());
        let inits: Vec<_> = containers
            .iter()
            .filter(|c| c.kind == ContainerKind::Init)
            .collect();
        assert_eq!(inits.len(), 1);
        assert_eq!(inits[0].name, "init-db");
        assert_eq!(inits[0].state, "Completed");
    }

    #[test]
    fn resource_requests_parsed() {
        let containers = containers_from_pod(&running_pod());
        let app = containers.iter().find(|c| c.name == "app").unwrap();
        assert_eq!(app.cpu_request, "100m");
        assert_eq!(app.mem_request, "128Mi");
        assert_eq!(app.cpu_limit, "500m");
        assert_eq!(app.mem_limit, "256Mi");
    }

    #[test]
    fn missing_status_is_waiting() {
        let pod = json!({"spec": {"containers": [{"name": "x", "image": "img"}]}});
        let containers = containers_from_pod(&pod);
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].state, "Waiting");
        assert!(!containers[0].ready);
    }

    #[test]
    fn empty_pod_returns_empty() {
        let containers = containers_from_pod(&json!({}));
        assert!(containers.is_empty());
    }

    #[test]
    fn to_json_has_expected_fields() {
        let containers = containers_from_pod(&running_pod());
        let j = containers[0].to_json();
        assert_eq!(j["name"], "app");
        assert_eq!(j["kind"], "C");
        assert_eq!(j["restarts"], 2);
    }
}
