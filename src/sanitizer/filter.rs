//! Field-level allowlist filter.
//!
//! For each GVR, only the fields on the allowlist are forwarded.
//! Everything else is dropped silently (default-deny).

use serde_json::{Map, Value};

use crate::client::gvr::well_known;
use crate::client::Gvr;

/// Which top-level fields pass through for a given GVR.
///
/// For nested objects (e.g. `spec.containers`) we apply specialised
/// stripping within `apply()` rather than maintaining a deeply-nested
/// allowlist — it keeps the logic explicit and auditable.
struct Allowlist {
    /// Top-level JSON keys allowed to pass.
    top_level_keys: &'static [&'static str],
    /// Special handling for `spec` sub-fields.
    spec_handler: SpecHandler,
}

#[derive(Clone, Copy)]
enum SpecHandler {
    /// Drop the entire `spec` field.
    DropAll,
    /// Keep only structural metadata from containers (names, images, ports,
    /// resource requests/limits). Drop env var values and unknown spec keys.
    ContainerStructure,
    /// Keep only the explicitly listed spec keys (default-deny for infra resources).
    /// Also strips container env values if containers appear in spec.
    KeepAllowlisted(&'static [&'static str]),
}

impl Allowlist {
    fn for_gvr(gvr: &Gvr) -> Self {
        if *gvr == well_known::secrets() {
            // Secrets: keep only metadata (name, ns, labels, type). Drop ALL data.
            return Self {
                top_level_keys: &["metadata", "type"],
                spec_handler: SpecHandler::DropAll,
            };
        }

        if *gvr == well_known::config_maps() {
            // ConfigMaps: metadata only, no data values.
            return Self {
                top_level_keys: &["metadata"],
                spec_handler: SpecHandler::DropAll,
            };
        }

        if *gvr == well_known::pods() {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::ContainerStructure,
            };
        }

        // Workloads (Deployment, StatefulSet, DaemonSet, ReplicaSet)
        if [
            well_known::deployments(),
            well_known::stateful_sets(),
            well_known::daemon_sets(),
            well_known::replica_sets(),
        ]
        .contains(gvr)
        {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::ContainerStructure,
            };
        }

        // Jobs, CronJobs
        if [well_known::jobs(), well_known::cron_jobs()].contains(gvr) {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::ContainerStructure,
            };
        }

        // Nodes — only topology/scheduling metadata (no containers, no data)
        if *gvr == well_known::nodes() {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::KeepAllowlisted(&[
                    "podCIDR",
                    "podCIDRs",
                    "providerID",
                    "taints",
                    "unschedulable",
                    "configSource",
                ]),
            };
        }

        // Namespaces — only lifecycle metadata
        if *gvr == well_known::namespaces() {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::KeepAllowlisted(&["finalizers"]),
            };
        }

        // Services: safe structural data (ports, selector, type, clusterIP)
        if *gvr == well_known::services() {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::KeepAllowlisted(&[
                    "type",
                    "selector",
                    "ports",
                    "clusterIP",
                    "clusterIPs",
                    "sessionAffinity",
                    "sessionAffinityConfig",
                    "externalName",
                    "externalIPs",
                    "loadBalancerIP",
                    "ipFamilies",
                    "ipFamilyPolicy",
                    "allocateLoadBalancerNodePorts",
                    "loadBalancerClass",
                    "internalTrafficPolicy",
                    "externalTrafficPolicy",
                    "publishNotReadyAddresses",
                    "healthCheckNodePort",
                ]),
            };
        }

        // Events: reason, message, type, count are diagnostic gold
        if *gvr == well_known::events() {
            return Self {
                top_level_keys: &[
                    "metadata",
                    "reason",
                    "message",
                    "type",
                    "count",
                    "firstTimestamp",
                    "lastTimestamp",
                    "source",
                    "involvedObject",
                ],
                spec_handler: SpecHandler::DropAll,
            };
        }

        // PersistentVolumes: capacity, access modes, storage class — no data paths
        if *gvr == well_known::persistent_volumes() {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::KeepAllowlisted(&[
                    "capacity",
                    "accessModes",
                    "storageClassName",
                    "persistentVolumeReclaimPolicy",
                    "volumeMode",
                    "nodeAffinity",
                    // Volume type fields (structural only — no credentials)
                    "hostPath",
                    "nfs",
                    "emptyDir",
                    "csi",
                    "local",
                    "gcePersistentDisk",
                    "awsElasticBlockStore",
                    "azureDisk",
                    "azureFile",
                ]),
            };
        }

        // PersistentVolumeClaims: request/binding metadata — no data
        if *gvr == well_known::persistent_volume_claims() {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::KeepAllowlisted(&[
                    "accessModes",
                    "storageClassName",
                    "volumeMode",
                    "resources",
                    "volumeName",
                    "selector",
                    "dataSource",
                    "dataSourceRef",
                ]),
            };
        }

        // ServiceAccounts: metadata and secrets list (names only — data is dropped by Secrets handler)
        if *gvr == well_known::service_accounts() {
            return Self {
                top_level_keys: &["metadata"],
                spec_handler: SpecHandler::DropAll,
            };
        }

        // RBAC — rules and subjects are useful for audit; no secret material
        if [
            well_known::roles(),
            well_known::role_bindings(),
            well_known::cluster_roles(),
            well_known::cluster_role_bindings(),
        ]
        .contains(gvr)
        {
            return Self {
                top_level_keys: &["metadata", "rules", "subjects", "roleRef"],
                spec_handler: SpecHandler::DropAll,
            };
        }

        // NetworkPolicies: ingress/egress rules are structural; no credential material
        if *gvr == well_known::network_policies() {
            return Self {
                top_level_keys: &["metadata", "spec"],
                spec_handler: SpecHandler::KeepAllowlisted(&[
                    "podSelector",
                    "ingress",
                    "egress",
                    "policyTypes",
                ]),
            };
        }

        // Ingresses: routing rules — no credential material in spec
        if *gvr == well_known::ingresses() {
            return Self {
                top_level_keys: &["metadata", "spec", "status"],
                spec_handler: SpecHandler::KeepAllowlisted(&[
                    "rules",
                    "tls",
                    "ingressClassName",
                    "defaultBackend",
                ]),
            };
        }

        // Unknown GVRs — metadata only, drop spec entirely to be safe
        Self {
            top_level_keys: &["metadata", "status"],
            spec_handler: SpecHandler::DropAll,
        }
    }
}

/// Applies the per-GVR allowlist to a raw JSON resource.
pub struct FieldFilter {
    allowlist: Allowlist,
}

impl FieldFilter {
    pub fn for_gvr(gvr: &Gvr) -> Self {
        Self {
            allowlist: Allowlist::for_gvr(gvr),
        }
    }

    /// Apply the allowlist. Returns filtered JSON.
    pub fn apply(&self, mut value: Value) -> Result<Value, String> {
        let obj = match value.as_object_mut() {
            Some(o) => o,
            None => return Ok(value),
        };

        // Drop all top-level keys not in the allowlist.
        let allowed_keys: std::collections::HashSet<&str> =
            self.allowlist.top_level_keys.iter().copied().collect();

        obj.retain(|k, _| allowed_keys.contains(k.as_str()));

        // Apply metadata sanitization (strip managed fields, last-applied).
        if let Some(meta) = obj.get_mut("metadata") {
            strip_metadata_noise(meta);
        }

        // Apply spec handler.
        match self.allowlist.spec_handler {
            SpecHandler::DropAll => {
                obj.remove("spec");
            }
            SpecHandler::ContainerStructure => {
                if let Some(spec) = obj.get_mut("spec") {
                    strip_container_secrets(spec);
                }
            }
            SpecHandler::KeepAllowlisted(allowed_keys) => {
                if let Some(spec) = obj.get_mut("spec") {
                    if let Some(spec_obj) = spec.as_object_mut() {
                        // Default-deny: drop any spec key not in the allowlist.
                        spec_obj.retain(|k, _| allowed_keys.contains(&k.as_str()));
                    }
                    // Defense in depth: strip container env values if injected.
                    strip_container_env_defense(spec);
                }
            }
        }

        Ok(Value::Object(obj.clone()))
    }
}

/// Remove high-noise, low-value metadata fields and redact annotation values.
///
/// Annotation values are stripped entirely (only keys kept) because annotations
/// frequently contain Helm values with credentials, injected sidecar configs with
/// tokens, and operator-specific secrets in formats that regex patterns may not
/// reliably catch.
fn strip_metadata_noise(meta: &mut Value) {
    if let Some(obj) = meta.as_object_mut() {
        // managedFields is huge and useless for LLM analysis.
        obj.remove("managedFields");

        // Strip ALL annotation values — keep only keys.
        // Rationale: annotation values can contain Helm-injected credentials,
        // sidecar configs with tokens, and operator secrets not covered by regex.
        if let Some(annotations) = obj.get_mut("annotations") {
            if let Some(ann_obj) = annotations.as_object_mut() {
                let keys: Vec<String> = ann_obj.keys().cloned().collect();
                let count = keys.len();
                ann_obj.clear();
                for k in keys {
                    ann_obj.insert(k, Value::String("[REDACTED]".to_string()));
                }
                if count > 0 {
                    tracing::debug!(
                        annotation_count = count,
                        "sanitizer: annotation values redacted (keys preserved)"
                    );
                }
            }
        }

        // Strip ALL label values — keep only keys.
        // Rationale: same as annotations — label values may carry operator-injected
        // tokens or sensitive identifiers that regex patterns may not catch reliably.
        if let Some(labels) = obj.get_mut("labels") {
            if let Some(lbl_obj) = labels.as_object_mut() {
                let keys: Vec<String> = lbl_obj.keys().cloned().collect();
                let count = keys.len();
                lbl_obj.clear();
                for k in keys {
                    lbl_obj.insert(k, Value::String("[REDACTED]".to_string()));
                }
                if count > 0 {
                    tracing::debug!(
                        label_count = count,
                        "sanitizer: label values redacted (keys preserved)"
                    );
                }
            }
        }
    }
}

/// Known structural spec keys for workload resources (Pods, Deployments, etc.).
///
/// Only these keys are forwarded when `ContainerStructure` mode is active.
/// Any key not in this list is dropped — default-deny for spec content.
const KNOWN_SPEC_KEYS: &[&str] = &[
    // Pod-level
    "containers",
    "initContainers",
    "ephemeralContainers",
    "volumes",
    "serviceAccountName",
    "restartPolicy",
    "nodeName",
    "nodeSelector",
    "tolerations",
    "affinity",
    "dnsConfig",
    "hostNetwork",
    "hostPID",
    "hostIPC",
    "priority",
    "priorityClassName",
    "topologySpreadConstraints",
    "runtimeClassName",
    "schedulerName",
    "preemptionPolicy",
    "automountServiceAccountToken",
    // Workload-level (Deployment / StatefulSet / DaemonSet / ReplicaSet)
    "replicas",
    "selector",
    "template",
    "strategy",
    "updateStrategy",
    "minReadySeconds",
    "revisionHistoryLimit",
    "paused",
    // StatefulSet-specific
    "serviceName",
    "podManagementPolicy",
    "volumeClaimTemplates",
    "persistentVolumeClaimRetentionPolicy",
    // Job / CronJob
    "schedule",
    "jobTemplate",
    "completions",
    "parallelism",
    "backoffLimit",
    "activeDeadlineSeconds",
    "ttlSecondsAfterFinished",
    "completionMode",
    "suspend",
];

/// Strip secret material from container specs.
///
/// Container env var *values* are dropped (names are kept).
/// Volume mounts are kept (no secret material there).
/// Image pull secrets are dropped entirely.
/// Unknown spec fields are dropped (default-deny).
fn strip_container_secrets(spec: &mut Value) {
    if let Some(obj) = spec.as_object_mut() {
        // Default-deny: drop any spec key not in the known-structural allowlist.
        obj.retain(|k, _| KNOWN_SPEC_KEYS.contains(&k.as_str()));

        // Image pull secrets contain registry credentials.
        obj.remove("imagePullSecrets");

        // Process containers and initContainers.
        for key in &["containers", "initContainers", "ephemeralContainers"] {
            if let Some(containers) = obj.get_mut(*key) {
                strip_container_array_secrets(containers);
            }
        }

        // Template spec (Deployment/StatefulSet nested pod template).
        if let Some(template) = obj.get_mut("template") {
            if let Some(tpl_obj) = template.as_object_mut() {
                // Strip template metadata (annotations/labels) like top-level metadata.
                if let Some(tpl_meta) = tpl_obj.get_mut("metadata") {
                    strip_metadata_noise(tpl_meta);
                }
                if let Some(tpl_spec) = tpl_obj.get_mut("spec") {
                    strip_container_secrets(tpl_spec);
                }
            }
        }
    }
}

/// Defense-in-depth: walk a spec value and strip env var values from any
/// `containers` or `initContainers` arrays found at any depth.
///
/// Used on `KeepAllowlisted` specs to catch adversarially injected container
/// structures that wouldn't normally appear in infra resource specs.
fn strip_container_env_defense(spec: &mut Value) {
    match spec {
        Value::Object(obj) => {
            for key in &["containers", "initContainers", "ephemeralContainers"] {
                if let Some(arr) = obj.get_mut(*key) {
                    strip_container_array_secrets(arr);
                }
            }
            for v in obj.values_mut() {
                strip_container_env_defense(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_container_env_defense(v);
            }
        }
        _ => {}
    }
}

fn strip_container_array_secrets(containers: &mut Value) {
    if let Some(arr) = containers.as_array_mut() {
        for container in arr.iter_mut() {
            if let Some(c) = container.as_object_mut() {
                // Drop env var values (keep names for diagnostic context).
                if let Some(env) = c.get_mut("env") {
                    redact_env_values(env);
                }
                // envFrom mounts ConfigMaps/Secrets wholesale — drop entirely.
                c.remove("envFrom");
            }
        }
    }
}

/// Replace each env var value with a redaction marker, keeping the name.
fn redact_env_values(env: &mut Value) {
    if let Some(arr) = env.as_array_mut() {
        for item in arr.iter_mut() {
            if let Some(entry) = item.as_object_mut() {
                // Remove "value" and "valueFrom" fields.
                entry.remove("value");
                entry.remove("valueFrom");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;
    use serde_json::json;

    #[test]
    fn secret_resource_data_is_stripped() {
        let raw = json!({
            "metadata": { "name": "s", "namespace": "ns" },
            "data": { "token": "aGVsbG8=" },
            "type": "Opaque"
        });
        let filter = FieldFilter::for_gvr(&well_known::secrets());
        let result = filter.apply(raw).unwrap();
        assert!(result.get("data").is_none(), "data field must be stripped");
        assert!(result.get("type").is_some(), "type field must be kept");
    }

    #[test]
    fn configmap_data_is_stripped() {
        let raw = json!({
            "metadata": { "name": "cm" },
            "data": { "config.yaml": "password: hunter2" }
        });
        let filter = FieldFilter::for_gvr(&well_known::config_maps());
        let result = filter.apply(raw).unwrap();
        assert!(result.get("data").is_none());
    }

    #[test]
    fn pod_env_values_are_stripped() {
        let raw = json!({
            "metadata": { "name": "p" },
            "spec": {
                "containers": [{
                    "name": "app",
                    "image": "nginx",
                    "env": [{ "name": "API_KEY", "value": "secret-value" }]
                }]
            }
        });
        let filter = FieldFilter::for_gvr(&well_known::pods());
        let result = filter.apply(raw).unwrap();
        let s = serde_json::to_string(&result).unwrap();
        assert!(
            !s.contains("secret-value"),
            "env value must be stripped: {s}"
        );
        assert!(
            s.contains("API_KEY"),
            "env name should be kept for diagnostics: {s}"
        );
    }

    #[test]
    fn pod_image_is_preserved() {
        let raw = json!({
            "metadata": { "name": "p" },
            "spec": {
                "containers": [{ "name": "app", "image": "my-registry/app:v2.3" }]
            },
            "status": { "phase": "Running" }
        });
        let filter = FieldFilter::for_gvr(&well_known::pods());
        let result = filter.apply(raw).unwrap();
        let s = serde_json::to_string(&result).unwrap();
        assert!(
            s.contains("my-registry/app:v2.3"),
            "image must be preserved: {s}"
        );
    }

    #[test]
    fn managed_fields_stripped_from_metadata() {
        let raw = json!({
            "metadata": {
                "name": "pod",
                "managedFields": [{ "manager": "kubectl", "operation": "Apply" }]
            },
            "spec": {}
        });
        let filter = FieldFilter::for_gvr(&well_known::pods());
        let result = filter.apply(raw).unwrap();
        let meta = result.get("metadata").unwrap();
        assert!(
            meta.get("managedFields").is_none(),
            "managedFields must be stripped"
        );
    }

    #[test]
    fn annotation_values_are_redacted_keys_kept() {
        let raw = json!({
            "metadata": {
                "name": "pod",
                "namespace": "default",
                "annotations": {
                    "app.kubernetes.io/version": "v1.2.3",
                    "helm.sh/chart": "my-chart-1.0.0",
                    "vault.hashicorp.com/token": "s.SuperSecret123"
                }
            },
            "spec": { "containers": [{ "name": "app", "image": "nginx" }] }
        });
        let filter = FieldFilter::for_gvr(&well_known::pods());
        let result = filter.apply(raw).unwrap();
        let s = serde_json::to_string(&result).unwrap();
        // Keys must be present for diagnostic context.
        assert!(
            s.contains("app.kubernetes.io/version"),
            "annotation key must be kept"
        );
        assert!(
            s.contains("vault.hashicorp.com/token"),
            "annotation key must be kept"
        );
        // Values must all be redacted.
        assert!(
            !s.contains("v1.2.3"),
            "annotation value must be redacted: {s}"
        );
        assert!(
            !s.contains("s.SuperSecret123"),
            "secret annotation value must be redacted: {s}"
        );
        assert!(
            s.contains("[REDACTED]"),
            "redaction marker must appear: {s}"
        );
    }

    #[test]
    fn event_resource_drops_spec_keeps_message() {
        let raw = json!({
            "metadata": { "name": "evt" },
            "reason": "BackOff",
            "message": "Back-off restarting failed container",
            "type": "Warning",
            "count": 47,
            "spec": { "shouldBeDropped": true }
        });
        let filter = FieldFilter::for_gvr(&well_known::events());
        let result = filter.apply(raw).unwrap();
        assert!(result.get("reason").is_some(), "reason must be kept");
        assert!(result.get("message").is_some(), "message must be kept");
        assert!(
            result.get("spec").is_none(),
            "spec must be dropped for events"
        );
    }

    #[test]
    fn service_account_drops_secrets_list() {
        let raw = json!({
            "metadata": { "name": "default", "namespace": "default" },
            "secrets": [{ "name": "default-token-xxxx" }],
            "imagePullSecrets": [{ "name": "registry-creds" }]
        });
        let filter = FieldFilter::for_gvr(&well_known::service_accounts());
        let result = filter.apply(raw).unwrap();
        assert!(
            result.get("secrets").is_none(),
            "secrets list must be stripped"
        );
        assert!(
            result.get("imagePullSecrets").is_none(),
            "imagePullSecrets must be stripped"
        );
    }

    #[test]
    fn rbac_role_keeps_rules() {
        let raw = json!({
            "metadata": { "name": "read-pods" },
            "rules": [{ "verbs": ["get", "list"], "resources": ["pods"] }],
            "data": { "should_be_dropped": true }
        });
        let filter = FieldFilter::for_gvr(&well_known::roles());
        let result = filter.apply(raw).unwrap();
        assert!(
            result.get("rules").is_some(),
            "rules must be kept for RBAC analysis"
        );
        assert!(
            result.get("data").is_none(),
            "data must be dropped for roles"
        );
    }

    #[test]
    fn unknown_gvr_drops_spec() {
        let raw = json!({
            "metadata": { "name": "custom" },
            "spec": { "potentiallySecret": "value" },
            "status": { "state": "Ready" }
        });
        let unknown = crate::client::Gvr::new("custom.io", "v1alpha1", "widgets");
        let filter = FieldFilter::for_gvr(&unknown);
        let result = filter.apply(raw).unwrap();
        assert!(
            result.get("spec").is_none(),
            "spec must be dropped for unknown GVRs"
        );
        assert!(
            result.get("status").is_some(),
            "status must be kept for unknown GVRs"
        );
    }
}
