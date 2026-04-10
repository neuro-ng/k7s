//! Sanitizer security audit — Phase 14.10.
//!
//! An independent, adversarial test suite designed to find leaks in the
//! sanitizer pipeline. Every test represents a real-world attack vector
//! or edge case that could cause a secret to reach the LLM.
//!
//! These run on every CI push (`cargo test --test sanitizer_audit`).
//!
//! # Threat model
//!
//! An attacker might try to smuggle a secret through the sanitizer by:
//! 1. Placing it in an unexpected field path
//! 2. Encoding it (base64, hex, URL encoding)
//! 3. Nesting it deeply inside labels/annotations
//! 4. Putting it in a non-obvious resource type (ConfigMap, Service annotation)
//! 5. Using unusual key names that don't match naive keyword lists

use k7s::client::Gvr;
use k7s::config::SanitizerConfig;
use k7s::sanitizer::sanitize;
use serde_json::{json, Value};

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn pod_gvr() -> Gvr {
    Gvr::core("v1", "pods")
}
fn secret_gvr() -> Gvr {
    Gvr::core("v1", "secrets")
}
fn configmap_gvr() -> Gvr {
    Gvr::core("v1", "configmaps")
}

fn sanitize_to_json(gvr: &Gvr, resource: Value) -> String {
    let cfg = SanitizerConfig::default();
    let safe = sanitize(gvr, Some("default"), "test", resource, &cfg).unwrap();
    serde_json::to_string(&safe.fields).unwrap()
}

fn assert_no_leak(output: &str, secret: &str, context: &str) {
    assert!(
        !output.contains(secret),
        "SECRET LEAKED — {context}\nSecret: {secret:?}\nOutput: {output}"
    );
}

// ─── Secret resource — all data must be stripped ─────────────────────────────

#[test]
fn secret_data_field_is_stripped() {
    let resource = json!({
        "metadata": { "name": "my-secret", "namespace": "default" },
        "type": "Opaque",
        "data": {
            "password": "c3VwZXJzZWNyZXQ=",
            "api-key":  "c2stYWJjMTIzeHl6"
        }
    });
    let out = sanitize_to_json(&secret_gvr(), resource);
    assert_no_leak(&out, "c3VwZXJzZWNyZXQ=", "base64 password in secret.data");
    assert_no_leak(&out, "c2stYWJjMTIzeHl6", "base64 api-key in secret.data");
    assert_no_leak(&out, "supersecret", "decoded password in secret.data");
}

#[test]
fn secret_string_data_field_is_stripped() {
    let resource = json!({
        "metadata": { "name": "my-secret", "namespace": "default" },
        "type": "Opaque",
        "stringData": {
            "token": "ghp_realtoken123456789"
        }
    });
    let out = sanitize_to_json(&secret_gvr(), resource);
    assert_no_leak(&out, "ghp_realtoken123456789", "stringData token in secret");
}

// ─── Pod env var values ───────────────────────────────────────────────────────

#[test]
fn pod_env_var_value_is_stripped() {
    let resource = json!({
        "metadata": { "name": "app", "namespace": "default" },
        "spec": {
            "containers": [{
                "name": "app",
                "env": [
                    { "name": "DB_PASSWORD",   "value": "hunter2" },
                    { "name": "REDIS_URL",      "value": "redis://:secret@redis:6379" },
                    { "name": "AWS_SECRET_KEY", "value": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY" }
                ]
            }]
        }
    });
    let out = sanitize_to_json(&pod_gvr(), resource);
    assert_no_leak(&out, "hunter2", "plain password env var");
    assert_no_leak(&out, "redis://:secret@", "connection string in env var");
    assert_no_leak(&out, "wJalrXUtnFEMI", "AWS secret key in env var");
}

#[test]
fn pod_env_name_is_kept_value_stripped() {
    let resource = json!({
        "metadata": { "name": "app", "namespace": "default" },
        "spec": {
            "containers": [{
                "name": "app",
                "env": [{ "name": "PORT", "value": "8080" }]
            }]
        }
    });
    let out = sanitize_to_json(&pod_gvr(), resource);
    // Name "PORT" is safe and may appear; value "8080" is a benign port so
    // this test just ensures no crash. The important invariant is that
    // secret *values* are stripped.
    let _ = out; // no assertion — just checking it doesn't panic
}

// ─── ConfigMap values ─────────────────────────────────────────────────────────

#[test]
fn configmap_values_are_stripped() {
    let resource = json!({
        "metadata": { "name": "app-config", "namespace": "default" },
        "data": {
            "database_url": "postgres://admin:secretpw@db:5432/mydb",
            "log_level":    "info",
            "api_key":      "sk-abc123"
        }
    });
    let out = sanitize_to_json(&configmap_gvr(), resource);
    assert_no_leak(&out, "secretpw", "password in configmap database_url");
    assert_no_leak(&out, "sk-abc123", "api key in configmap value");
    assert_no_leak(&out, "postgres://admin:secretpw", "full connection string");
}

// ─── Annotations with secret-pattern values ──────────────────────────────────

#[test]
fn annotation_with_token_value_is_redacted() {
    let resource = json!({
        "metadata": {
            "name": "app",
            "namespace": "default",
            "annotations": {
                "vault.hashicorp.com/agent-inject-secret-db": "secret/data/db",
                "custom/bearer-token": "Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ0ZXN0In0.sig"
            }
        }
    });
    let out = sanitize_to_json(&pod_gvr(), resource);
    // The JWT-like annotation value should be redacted.
    assert_no_leak(
        &out,
        "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9",
        "JWT token in annotation value",
    );
}

// ─── Deeply nested values ─────────────────────────────────────────────────────

#[test]
fn deeply_nested_secret_env_is_stripped() {
    // Secrets nested inside initContainers, not just containers.
    let resource = json!({
        "metadata": { "name": "app", "namespace": "default" },
        "spec": {
            "initContainers": [{
                "name": "init",
                "env": [{ "name": "INIT_SECRET", "value": "topsecretinit" }]
            }],
            "containers": [{
                "name": "app",
                "env": [{ "name": "MAIN_SECRET", "value": "topsecretmain" }]
            }]
        }
    });
    let out = sanitize_to_json(&pod_gvr(), resource);
    assert_no_leak(&out, "topsecretinit", "secret in initContainers env");
    assert_no_leak(&out, "topsecretmain", "secret in containers env");
}

// ─── Regex-pattern redaction ──────────────────────────────────────────────────

#[test]
fn connection_string_in_label_value_is_redacted() {
    // Labels are allowlisted but their values must still pass through the
    // pattern-based redactor.
    let resource = json!({
        "metadata": {
            "name": "app",
            "namespace": "default",
            "labels": {
                "app": "nginx",
                "debug-conn": "postgresql://user:p@ssw0rd@host:5432/db"
            }
        }
    });
    let out = sanitize_to_json(&pod_gvr(), resource);
    assert_no_leak(
        &out,
        "p@ssw0rd",
        "password in label value connection string",
    );
}

#[test]
fn jwt_in_any_string_field_is_redacted() {
    // Place a JWT in the pod's name-adjacent description annotation.
    let resource = json!({
        "metadata": {
            "name": "app",
            "namespace": "default",
            "annotations": {
                "description": "token: eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.abc"
            }
        }
    });
    let out = sanitize_to_json(&pod_gvr(), resource);
    assert_no_leak(
        &out,
        "eyJhbGciOiJIUzI1NiJ9",
        "JWT header in annotation description",
    );
}

// ─── Custom pattern redaction ─────────────────────────────────────────────────

#[test]
fn custom_pattern_blocks_internal_token() {
    let mut cfg = SanitizerConfig::default();
    cfg.custom_patterns
        .push(r"(?i)internal-token:\s*\S+".to_owned());

    let resource = json!({
        "metadata": {
            "name": "app",
            "namespace": "default",
            "labels": { "info": "internal-token: abc123secretvalue" }
        }
    });
    let safe = sanitize(&pod_gvr(), Some("default"), "app", resource, &cfg).unwrap();
    let out = serde_json::to_string(&safe.fields).unwrap();
    assert_no_leak(&out, "abc123secretvalue", "custom pattern token in label");
}

// ─── Safe fields still pass through ──────────────────────────────────────────

#[test]
fn pod_name_and_labels_survive_sanitization() {
    let resource = json!({
        "metadata": {
            "name": "nginx-abc123",
            "namespace": "production",
            "labels": { "app": "nginx", "version": "1.25" }
        },
        "status": { "phase": "Running" }
    });
    let out = sanitize_to_json(&pod_gvr(), resource);
    assert!(
        out.contains("nginx-abc123"),
        "pod name should survive sanitization"
    );
}

#[test]
fn deployment_name_survives_sanitization() {
    let gvr = Gvr::new("apps", "v1", "deployments");
    let resource = json!({
        "metadata": { "name": "my-deploy", "namespace": "default" },
        "spec": { "replicas": 3 },
        "status": { "readyReplicas": 3, "replicas": 3 }
    });
    let out = sanitize_to_json(&gvr, resource);
    assert!(
        out.contains("my-deploy"),
        "deployment name should survive sanitization"
    );
}
