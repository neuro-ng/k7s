//! Helm values sanitizer — Phase 35.
//!
//! Recursively walks Helm user-supplied values (the `config` field from the
//! release proto-JSON) and redacts any field that looks like a secret.
//!
//! Rules applied in order:
//!
//! 1. **Key-name blocklist** — any key whose name (lowercased) contains a
//!    known secret pattern (`password`, `secret`, `token`, `key`,
//!    `credential`, `dsn`, `apikey`) is replaced with `"[REDACTED]"`.
//! 2. **Value-content scan** — string values that contain typical secret
//!    substrings (connection-string markers, PEM headers) are replaced with
//!    `"[REDACTED]"`.
//! 3. Maps and arrays are traversed recursively; numbers, booleans, and null
//!    pass through unchanged.

use serde_json::Value;

/// Key substrings (all compared lowercased) that indicate a secret value.
pub(crate) const SECRET_KEY_PATTERNS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "credential",
    "credentials",
    "privatekey",
    "private_key",
    "dsn",
];

/// Value substrings (compared lowercased) that indicate a connection string
/// or other secret content.
const SECRET_VALUE_PATTERNS: &[&str] = &[
    "password=",
    "passwd=",
    "secret=",
    ":@",         // user:pass@host pattern
    "-----begin", // PEM certificate / key header
];

/// Recursively sanitize Helm user-supplied values, redacting secret fields.
///
/// # Rules
///
/// 1. Map keys whose name (lowercased) contains any entry from
///    [`SECRET_KEY_PATTERNS`] have their value replaced with `"[REDACTED]"`.
/// 2. String values that contain any entry from [`SECRET_VALUE_PATTERNS`]
///    (case-insensitive) are replaced with `"[REDACTED]"`.
/// 3. Arrays and nested maps are traversed recursively.
/// 4. Non-string primitives (numbers, booleans, null) pass through unchanged.
pub fn sanitize_helm_values(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if is_secret_key(&k) {
                    out.insert(k, Value::String("[REDACTED]".into()));
                } else {
                    out.insert(k, sanitize_helm_values(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(sanitize_helm_values).collect())
        }
        Value::String(s) if is_secret_value(&s) => Value::String("[REDACTED]".into()),
        other => other,
    }
}

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SECRET_KEY_PATTERNS.iter().any(|p| lower.contains(p))
}

fn is_secret_value(val: &str) -> bool {
    let lower = val.to_lowercase();
    SECRET_VALUE_PATTERNS.iter().any(|p| lower.contains(p))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_password_key() {
        let input = json!({"password": "super-secret", "replicas": 3});
        let out = sanitize_helm_values(input);
        assert_eq!(out["password"], "[REDACTED]");
        assert_eq!(out["replicas"], 3);
    }

    #[test]
    fn redacts_nested_secret() {
        let input = json!({"db": {"password": "hunter2", "host": "localhost"}});
        let out = sanitize_helm_values(input);
        assert_eq!(out["db"]["password"], "[REDACTED]");
        assert_eq!(out["db"]["host"], "localhost");
    }

    #[test]
    fn redacts_secret_key_case_insensitive() {
        let input = json!({"DatabasePassword": "abc123"});
        let out = sanitize_helm_values(input);
        assert_eq!(out["DatabasePassword"], "[REDACTED]");
    }

    #[test]
    fn redacts_token_key() {
        let input = json!({"apiToken": "tok_live_xxxx"});
        let out = sanitize_helm_values(input);
        assert_eq!(out["apiToken"], "[REDACTED]");
    }

    #[test]
    fn redacts_dsn_key() {
        let input = json!({"dsn": "postgres://user:pass@host:5432/db"});
        let out = sanitize_helm_values(input);
        assert_eq!(out["dsn"], "[REDACTED]");
    }

    #[test]
    fn redacts_value_containing_password_equals() {
        let input = json!({"connectionString": "Server=tcp:host;Password=secret123;"});
        let out = sanitize_helm_values(input);
        assert_eq!(out["connectionString"], "[REDACTED]");
    }

    #[test]
    fn redacts_pem_value() {
        let input = json!({"tlsCert": "-----BEGIN CERTIFICATE-----\nMIIB..."});
        let out = sanitize_helm_values(input);
        assert_eq!(out["tlsCert"], "[REDACTED]");
    }

    #[test]
    fn preserves_safe_values() {
        let input = json!({
            "replicas": 3,
            "enabled": true,
            "image": {"tag": "v1.2.3"},
            "timeout": null
        });
        let out = sanitize_helm_values(input);
        assert_eq!(out["replicas"], 3);
        assert_eq!(out["enabled"], true);
        assert_eq!(out["image"]["tag"], "v1.2.3");
        assert!(out["timeout"].is_null());
    }

    #[test]
    fn handles_arrays_recursively() {
        let input = json!({
            "envFrom": [
                {"name": "FOO", "value": "bar"},
                {"name": "DB_SECRET", "value": "should-stay"}
            ]
        });
        let out = sanitize_helm_values(input);
        // "name" and "value" keys don't match secret patterns — safe.
        assert_eq!(out["envFrom"][0]["value"], "bar");
        // "DB_SECRET" is the value of "name", not a key — not redacted here.
        assert_eq!(out["envFrom"][1]["name"], "DB_SECRET");
    }

    #[test]
    fn redacts_api_key_variants() {
        let input = json!({
            "apiKey": "sk-xxxx",
            "api_key": "ak-yyyy",
            "secretKey": "sk-zzzz"
        });
        let out = sanitize_helm_values(input);
        assert_eq!(out["apiKey"], "[REDACTED]");
        assert_eq!(out["api_key"], "[REDACTED]");
        assert_eq!(out["secretKey"], "[REDACTED]");
    }

    #[test]
    fn redacts_credential_key() {
        let input = json!({"awsCredentials": "AKIAIOSFODNN7EXAMPLE:wJalrXUtnFEMI"});
        let out = sanitize_helm_values(input);
        assert_eq!(out["awsCredentials"], "[REDACTED]");
    }

    #[test]
    fn redacts_empty_password_connection_string() {
        // ":@" pattern catches "user:@host" style URIs (empty password slot).
        let input = json!({"mongoUri": "mongodb://admin:@mongo:27017/db"});
        let out = sanitize_helm_values(input);
        assert_eq!(out["mongoUri"], "[REDACTED]");
    }
}
