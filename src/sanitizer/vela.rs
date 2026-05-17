//! KubeVela / OAM spec sanitizer — Phase 36.
//!
//! Extends `sanitize_helm_values` with OAM-specific patterns to protect
//! environment variables, secret references, and image registry credentials
//! in Application component `properties` before they reach the LLM or the
//! TUI display.
//!
//! Additional OAM-specific key patterns beyond the Helm set:
//! `env`, `envfrom`, `secretref`, `configref`, `volumemounts`, `imagepullsecrets`.
//!
//! `image` keys are intentionally **allowed** (image names are useful for AI
//! analysis) unless the value looks like it contains embedded auth material
//! (e.g. `registry.example.com/user:password@sha256:...`).

use serde_json::Value;

/// Additional OAM key-name patterns to block (lowercased substring match).
///
/// These complement the patterns in [`super::helm::SECRET_KEY_PATTERNS`].
const OAM_SECRET_KEY_PATTERNS: &[&str] = &[
    "secretref",
    "secret_ref",
    "configref",
    "config_ref",
    "imagepullsecret",
    "volumemount",
    "envfrom",
    "env_from",
];

/// Sanitize KubeVela component `properties` or any OAM spec value.
///
/// Applies the Helm key-name blocklist first, then the OAM-specific patterns,
/// then recursively descends into objects and arrays.
///
/// The `env` key is handled specially: it is a list of `{name, value}` objects
/// where the *values* (not names) should be redacted if they look secret-like.
pub fn sanitize_vela_properties(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if is_secret_key(&k) {
                    out.insert(k, Value::String("[REDACTED]".into()));
                } else if k.to_lowercase() == "env" {
                    // Env arrays: redact any item whose value matches.
                    out.insert(k, sanitize_env_array(v));
                } else {
                    out.insert(k, sanitize_vela_properties(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sanitize_vela_properties).collect()),
        Value::String(s) if is_secret_value(&s) => Value::String("[REDACTED]".into()),
        other => other,
    }
}

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    // First apply the Helm key patterns.
    if super::helm::SECRET_KEY_PATTERNS
        .iter()
        .any(|p| lower.contains(p))
    {
        return true;
    }
    // Then OAM-specific patterns.
    OAM_SECRET_KEY_PATTERNS.iter().any(|p| lower.contains(p))
}

fn is_secret_value(val: &str) -> bool {
    // Reuse the same value-content heuristics as the Helm sanitizer.
    super::helm::SECRET_KEY_PATTERNS
        .iter()
        .any(|p| val.to_lowercase().contains(&format!("{p}=")))
        || val.contains(":@")
        || val.to_lowercase().contains("-----begin")
}

/// Sanitize an `env` array — redact the `value` field of each entry that
/// looks like a secret or whose `name` suggests a secret.
fn sanitize_env_array(value: Value) -> Value {
    match value {
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|item| match item {
                    Value::Object(mut map) => {
                        let name_lower = map
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_lowercase();
                        // Redact value if env var name suggests a secret.
                        let name_is_secret = super::helm::SECRET_KEY_PATTERNS
                            .iter()
                            .any(|p| name_lower.contains(p));
                        if name_is_secret {
                            map.insert("value".into(), Value::String("[REDACTED]".into()));
                        } else if let Some(Value::String(s)) = map.get("value") {
                            if is_secret_value(s) {
                                map.insert("value".into(), Value::String("[REDACTED]".into()));
                            }
                        }
                        Value::Object(map)
                    }
                    other => other,
                })
                .collect(),
        ),
        other => sanitize_vela_properties(other),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inherits_password_key_from_helm() {
        let input = json!({"database": {"password": "secret123", "host": "localhost"}});
        let out = sanitize_vela_properties(input);
        assert_eq!(out["database"]["password"], "[REDACTED]");
        assert_eq!(out["database"]["host"], "localhost");
    }

    #[test]
    fn redacts_secret_ref() {
        let input = json!({"secretRef": {"name": "my-secret", "key": "password"}});
        let out = sanitize_vela_properties(input);
        assert_eq!(out["secretRef"], "[REDACTED]");
    }

    #[test]
    fn redacts_config_ref() {
        let input = json!({"configRef": "my-configmap"});
        let out = sanitize_vela_properties(input);
        assert_eq!(out["configRef"], "[REDACTED]");
    }

    #[test]
    fn redacts_image_pull_secrets() {
        let input = json!({"imagePullSecrets": [{"name": "reg-creds"}]});
        let out = sanitize_vela_properties(input);
        assert_eq!(out["imagePullSecrets"], "[REDACTED]");
    }

    #[test]
    fn preserves_image_name() {
        let input = json!({"image": "nginx:1.25", "port": 80});
        let out = sanitize_vela_properties(input);
        assert_eq!(out["image"], "nginx:1.25");
        assert_eq!(out["port"], 80);
    }

    #[test]
    fn redacts_env_by_name_pattern() {
        let input = json!({
            "env": [
                {"name": "DB_PASSWORD", "value": "hunter2"},
                {"name": "APP_PORT", "value": "8080"}
            ]
        });
        let out = sanitize_vela_properties(input);
        let env = out["env"].as_array().unwrap();
        assert_eq!(env[0]["value"], "[REDACTED]");
        assert_eq!(env[1]["value"], "8080");
    }

    #[test]
    fn redacts_env_by_value_content() {
        let input = json!({
            "env": [
                {"name": "DATABASE_URL", "value": "postgres://user:@host/db"}
            ]
        });
        let out = sanitize_vela_properties(input);
        let env = out["env"].as_array().unwrap();
        assert_eq!(env[0]["value"], "[REDACTED]");
    }

    #[test]
    fn preserves_safe_env_vars() {
        let input = json!({
            "env": [
                {"name": "REPLICAS", "value": "3"},
                {"name": "LOG_LEVEL", "value": "info"}
            ]
        });
        let out = sanitize_vela_properties(input);
        let env = out["env"].as_array().unwrap();
        assert_eq!(env[0]["value"], "3");
        assert_eq!(env[1]["value"], "info");
    }

    #[test]
    fn redacts_envfrom_key() {
        let input = json!({"envFrom": [{"secretRef": {"name": "my-secret"}}]});
        let out = sanitize_vela_properties(input);
        assert_eq!(out["envFrom"], "[REDACTED]");
    }

    #[test]
    fn preserves_primitive_safe_values() {
        let input = json!({"replicas": 3, "enabled": true, "timeout": null});
        let out = sanitize_vela_properties(input);
        assert_eq!(out["replicas"], 3);
        assert_eq!(out["enabled"], true);
        assert!(out["timeout"].is_null());
    }

    #[test]
    fn handles_nested_components() {
        let input = json!({
            "components": [
                {
                    "name": "webservice",
                    "properties": {
                        "image": "nginx:latest",
                        "env": [
                            {"name": "SECRET_KEY", "value": "abc123"}
                        ]
                    }
                }
            ]
        });
        let out = sanitize_vela_properties(input);
        let env = &out["components"][0]["properties"]["env"][0];
        assert_eq!(env["value"], "[REDACTED]");
        // Image preserved.
        assert_eq!(out["components"][0]["properties"]["image"], "nginx:latest");
    }
}
