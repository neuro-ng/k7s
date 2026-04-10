//! Pseudo-fuzz tests for the sanitizer pipeline.
//!
//! These tests construct adversarial and random-like JSON payloads and assert
//! two invariants for every GVR:
//!
//! 1. **No panic** — `sanitize()` must never panic on arbitrary input.
//! 2. **No secret leak** — known secret strings must not appear in the output.
//!
//! This is property-based testing without a fuzzing harness: we enumerate a
//! wide variety of structurally-valid-but-adversarial inputs.  For true
//! coverage-guided fuzzing, use `cargo fuzz` with the `sanitize` fuzz target.

use k7s::client::gvr::well_known;
use k7s::client::Gvr;
use k7s::config::SanitizerConfig;
use k7s::sanitizer::sanitize;
use serde_json::{json, Value};

const SECRET_CANARY: &str = "SUPER_SECRET_CANARY_VALUE_12345";

fn cfg() -> SanitizerConfig {
    SanitizerConfig::default()
}

fn assert_no_canary(out: &Value, context: &str) {
    let s = serde_json::to_string(out).unwrap();
    assert!(
        !s.contains(SECRET_CANARY),
        "Secret canary leaked in {context}: {s}"
    );
}

fn run(gvr: &Gvr, raw: Value) -> Value {
    sanitize(gvr, Some("default"), "test-resource", raw, &cfg())
        .expect("sanitize must not error on valid JSON")
        .fields
}

// ─── Structural robustness ────────────────────────────────────────────────────

#[test]
fn empty_object_does_not_panic() {
    for gvr in all_gvrs() {
        let out = run(&gvr, json!({}));
        let _ = serde_json::to_string(&out).unwrap();
    }
}

#[test]
fn null_input_does_not_panic() {
    for gvr in all_gvrs() {
        let out = run(&gvr, Value::Null);
        let _ = serde_json::to_string(&out).unwrap();
    }
}

#[test]
fn array_input_does_not_panic() {
    for gvr in all_gvrs() {
        let out = run(&gvr, json!([1, 2, 3]));
        let _ = serde_json::to_string(&out).unwrap();
    }
}

#[test]
fn deeply_nested_canary_never_leaks() {
    // Place the canary 10 levels deep in every position that could plausibly
    // survive field filtering.
    let raw = json!({
        "metadata": {
            "name": "test",
            "annotations": { "any.key": SECRET_CANARY },
            "labels": { "label": SECRET_CANARY }
        },
        "spec": {
            "containers": [{
                "name": "app",
                "env": [{ "name": "VAR", "value": SECRET_CANARY }],
                "envFrom": [{ "secretRef": { "name": SECRET_CANARY } }]
            }],
            "deep": {
                "deeper": {
                    "deepest": SECRET_CANARY
                }
            }
        },
        "data": { "key": SECRET_CANARY },
        "stringData": { "key": SECRET_CANARY }
    });

    for gvr in all_gvrs() {
        let out = run(&gvr, raw.clone());
        assert_no_canary(&out, &gvr.to_string());
    }
}

// ─── Secret resource ─────────────────────────────────────────────────────────

#[test]
fn secret_data_field_never_leaks() {
    let raw = json!({
        "metadata": { "name": "s", "namespace": "default" },
        "type": "Opaque",
        "data": { "password": SECRET_CANARY, "token": SECRET_CANARY },
        "stringData": { "apiKey": SECRET_CANARY }
    });
    let out = run(&well_known::secrets(), raw);
    assert_no_canary(&out, "secrets/data");
    // Type should still be there.
    assert_eq!(out.get("type").and_then(|v| v.as_str()), Some("Opaque"));
}

// ─── ConfigMap ────────────────────────────────────────────────────────────────

#[test]
fn configmap_values_never_leak() {
    let raw = json!({
        "metadata": { "name": "cfg" },
        "data": {
            "config.yaml": format!("password: {SECRET_CANARY}"),
            "db_url": format!("postgres://user:{SECRET_CANARY}@host/db")
        }
    });
    let out = run(&well_known::config_maps(), raw);
    assert_no_canary(&out, "configmaps/data");
}

// ─── Pod ─────────────────────────────────────────────────────────────────────

#[test]
fn pod_env_values_never_leak() {
    let raw = json!({
        "metadata": { "name": "p", "namespace": "default" },
        "spec": {
            "containers": [{
                "name": "app",
                "image": "nginx",
                "env": [
                    { "name": "DB_PASS", "value": SECRET_CANARY },
                    { "name": "API_KEY", "value": SECRET_CANARY }
                ],
                "envFrom": [{ "secretRef": { "name": "my-secret" } }]
            }],
            "initContainers": [{
                "name": "init",
                "env": [{ "name": "INIT_SECRET", "value": SECRET_CANARY }]
            }]
        }
    });
    let out = run(&well_known::pods(), raw);
    assert_no_canary(&out, "pods/env");
    // Env names should survive.
    let s = serde_json::to_string(&out).unwrap();
    assert!(s.contains("DB_PASS"), "env name must survive: {s}");
}

#[test]
fn pod_annotations_never_leak() {
    let raw = json!({
        "metadata": {
            "name": "p",
            "annotations": {
                "vault.io/token": SECRET_CANARY,
                "custom/secret": SECRET_CANARY
            }
        },
        "spec": { "containers": [] }
    });
    let out = run(&well_known::pods(), raw);
    assert_no_canary(&out, "pods/annotations");
    // Annotation keys should survive.
    let s = serde_json::to_string(&out).unwrap();
    assert!(
        s.contains("vault.io/token"),
        "annotation key must survive: {s}"
    );
}

// ─── Deployment (nested pod template) ────────────────────────────────────────

#[test]
fn deployment_template_env_never_leaks() {
    let raw = json!({
        "metadata": { "name": "d" },
        "spec": {
            "template": {
                "spec": {
                    "containers": [{
                        "name": "app",
                        "env": [{ "name": "DB_PASS", "value": SECRET_CANARY }]
                    }]
                }
            }
        }
    });
    let out = run(&well_known::deployments(), raw);
    assert_no_canary(&out, "deployments/template/env");
}

// ─── Redactor catches what field filter doesn't ──────────────────────────────

#[test]
fn connection_string_in_status_is_redacted() {
    let raw = json!({
        "metadata": { "name": "svc" },
        "spec": { "type": "ClusterIP", "clusterIP": "10.0.0.1" },
        "status": {
            "message": format!("connected to postgres://user:{SECRET_CANARY}@db/prod")
        }
    });
    let out = run(&well_known::services(), raw);
    assert_no_canary(&out, "services/status/message");
}

#[test]
fn jwt_in_label_value_is_redacted() {
    let jwt = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ0ZXN0In0.sig";
    let raw = json!({
        "metadata": {
            "name": "p",
            "annotations": { "injected-token": jwt }
        },
        "spec": { "containers": [] }
    });
    let out = run(&well_known::pods(), raw);
    let s = serde_json::to_string(&out).unwrap();
    // The JWT was in an annotation value — which is now replaced with [REDACTED] by field filter.
    assert!(
        !s.contains("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9"),
        "JWT must not appear: {s}"
    );
}

// ─── Unknown GVR defaults to safe behaviour ──────────────────────────────────

#[test]
fn unknown_gvr_drops_spec_with_canary() {
    let raw = json!({
        "metadata": { "name": "custom" },
        "spec": { "credentialField": SECRET_CANARY },
        "status": { "state": "Ready" }
    });
    let unknown = Gvr::new("custom.io", "v1alpha1", "widgets");
    let out = run(&unknown, raw);
    assert_no_canary(&out, "unknown-gvr/spec");
    // Status should still be there.
    assert!(
        out.get("status").is_some(),
        "status must survive for unknown GVR"
    );
}

// ─── Custom patterns ─────────────────────────────────────────────────────────

#[test]
fn custom_pattern_blocks_canary() {
    let cfg = SanitizerConfig {
        custom_patterns: vec![format!("SUPER_SECRET_CANARY_VALUE_\\d+")],
        ..Default::default()
    };
    let raw = json!({
        "metadata": { "name": "p" },
        "status": { "info": SECRET_CANARY }
    });
    let out = sanitize(&well_known::pods(), Some("default"), "test", raw, &cfg)
        .unwrap()
        .fields;
    assert_no_canary(&out, "custom-pattern");
}

// ─── All GVR enumeration helper ──────────────────────────────────────────────

fn all_gvrs() -> Vec<Gvr> {
    vec![
        well_known::pods(),
        well_known::nodes(),
        well_known::namespaces(),
        well_known::services(),
        well_known::config_maps(),
        well_known::secrets(),
        well_known::events(),
        well_known::persistent_volumes(),
        well_known::persistent_volume_claims(),
        well_known::service_accounts(),
        well_known::deployments(),
        well_known::stateful_sets(),
        well_known::daemon_sets(),
        well_known::replica_sets(),
        well_known::jobs(),
        well_known::cron_jobs(),
        well_known::ingresses(),
        well_known::network_policies(),
        well_known::roles(),
        well_known::role_bindings(),
        well_known::cluster_roles(),
        well_known::cluster_role_bindings(),
        well_known::custom_resource_definitions(),
        Gvr::new("custom.io", "v1alpha1", "widgets"), // unknown GVR
    ]
}
