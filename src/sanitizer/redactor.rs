//! Regex-based secret and credential redaction.
//!
//! Walks the JSON tree after field-level filtering and replaces any string
//! value that matches a secret pattern with `"[REDACTED]"`.
//!
//! # Pattern priority
//!
//! 1. Built-in patterns cover the most common credential shapes.
//! 2. User-configured `custom_patterns` from `config.yaml` are applied after.
//!
//! # Performance
//!
//! Patterns are compiled once at construction. The compiled `Redactor` is
//! cheap to clone and can be stored in a long-lived context.

use regex::Regex;
use serde_json::Value;

use crate::error::SanitizeError;

/// Redaction marker substituted for detected secret values.
const REDACTED: &str = "[REDACTED]";

/// Compiled pattern set for efficient repeated redaction.
#[derive(Clone)]
pub struct Redactor {
    patterns: Vec<Regex>,
}

impl Redactor {
    /// Build a `Redactor` from built-in patterns plus optional user patterns.
    ///
    /// Returns `SanitizeError::InvalidPattern` if any regex fails to compile.
    pub fn new(custom_patterns: &[String]) -> Result<Self, SanitizeError> {
        let mut patterns = builtin_patterns()?;

        for raw in custom_patterns {
            let re = Regex::new(raw).map_err(|source| SanitizeError::InvalidPattern {
                pattern: raw.clone(),
                source,
            })?;
            patterns.push(re);
        }

        Ok(Self { patterns })
    }

    /// Walk a JSON value tree, replacing secret-like strings with `[REDACTED]`.
    pub fn redact(&self, value: Value) -> Result<Value, SanitizeError> {
        Ok(self.redact_value(value))
    }

    /// Redact secret-like patterns from a plain text string.
    ///
    /// Unlike `redact()` (which operates on JSON trees), this applies all
    /// compiled patterns directly against the raw text and replaces any
    /// matches with `[REDACTED]`.  Used to sanitize free-form user input
    /// before it enters the LLM message stream.
    pub fn redact_str(&self, text: &str) -> String {
        self.patterns.iter().fold(text.to_owned(), |acc, re| {
            re.replace_all(&acc, REDACTED).into_owned()
        })
    }

    fn redact_value(&self, value: Value) -> Value {
        match value {
            Value::String(s) => {
                if self.is_secret(&s) {
                    Value::String(REDACTED.to_string())
                } else {
                    Value::String(s)
                }
            }
            Value::Object(map) => {
                let redacted: serde_json::Map<String, Value> = map
                    .into_iter()
                    .map(|(k, v)| (k, self.redact_value(v)))
                    .collect();
                Value::Object(redacted)
            }
            Value::Array(arr) => {
                Value::Array(arr.into_iter().map(|v| self.redact_value(v)).collect())
            }
            // Primitives that are not strings can't contain secrets.
            other => other,
        }
    }

    fn is_secret(&self, s: &str) -> bool {
        self.patterns.iter().any(|p| p.is_match(s))
    }
}

/// Built-in secret detection patterns.
///
/// These cover common credential shapes seen in Kubernetes environments.
/// All patterns are case-insensitive.
fn builtin_patterns() -> Result<Vec<Regex>, SanitizeError> {
    let raw_patterns = [
        // Generic key/value pairs that look like credentials
        r"(?i)password\s*[:=]\s*\S+",
        r"(?i)passwd\s*[:=]\s*\S+",
        r"(?i)secret\s*[:=]\s*\S+",
        r"(?i)api[_-]?key\s*[:=]\s*\S+",
        r"(?i)access[_-]?key\s*[:=]\s*\S+",
        r"(?i)private[_-]?key\s*[:=]\s*\S+",
        r"(?i)auth[_-]?token\s*[:=]\s*\S+",
        r"(?i)bearer\s+[A-Za-z0-9\-._~+/]+=*",

        // Connection strings
        r"(?i)(mysql|postgres|postgresql|mongodb|redis|amqp|rabbitmq)://[^\s]+",
        r"(?i)jdbc:[a-z]+://[^\s]+",

        // AWS-style access keys (AKIA...)
        r"AKIA[0-9A-Z]{16}",

        // Generic high-entropy token shapes (base64, hex, JWT)
        // JWT: three base64url segments separated by dots
        r"ey[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",

        // PEM certificate/key blocks
        r"-----BEGIN [A-Z ]+ (KEY|CERTIFICATE)-----",

        // Kubernetes service account token prefix
        r"eyJhbGciOiJSUzI1NiIs",   // common SA token header
    ];

    raw_patterns
        .iter()
        .map(|&p| {
            Regex::new(p).map_err(|source| SanitizeError::InvalidPattern {
                pattern: p.to_string(),
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn redactor() -> Redactor {
        Redactor::new(&[]).expect("built-in patterns must compile")
    }

    #[test]
    fn password_value_is_redacted() {
        let r = redactor();
        let input = json!({ "config": "password=hunter2" });
        let output = r.redact(input).unwrap();
        let s = serde_json::to_string(&output).unwrap();
        assert!(s.contains(REDACTED), "password must be redacted: {s}");
        assert!(!s.contains("hunter2"), "raw password must not appear: {s}");
    }

    #[test]
    fn jwt_is_redacted() {
        let r = redactor();
        let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let input = json!({ "token": token });
        let output = r.redact(input).unwrap();
        let s = serde_json::to_string(&output).unwrap();
        assert!(s.contains(REDACTED), "JWT must be redacted: {s}");
    }

    #[test]
    fn connection_string_is_redacted() {
        let r = redactor();
        let input = json!({ "dsn": "postgres://user:pass@host:5432/db" });
        let output = r.redact(input).unwrap();
        let s = serde_json::to_string(&output).unwrap();
        assert!(s.contains(REDACTED), "connection string must be redacted: {s}");
    }

    #[test]
    fn safe_values_are_not_redacted() {
        let r = redactor();
        let input = json!({
            "image": "nginx:1.25",
            "phase": "Running",
            "replicas": 3,
            "app": "my-service"
        });
        let output = r.redact(input.clone()).unwrap();
        assert_eq!(input, output, "safe values must pass through unchanged");
    }

    #[test]
    fn nested_secret_is_redacted() {
        let r = redactor();
        let input = json!({
            "spec": {
                "containers": [{
                    "name": "app",
                    "env": [{ "name": "TOKEN", "value": "bearer secrettoken123" }]
                }]
            }
        });
        let output = r.redact(input).unwrap();
        let s = serde_json::to_string(&output).unwrap();
        assert!(!s.contains("secrettoken123"), "nested secret must be redacted: {s}");
    }

    #[test]
    fn custom_pattern_is_applied() {
        let patterns = vec!["my-custom-secret-\\d+".to_string()];
        let r = Redactor::new(&patterns).unwrap();
        let input = json!({ "key": "my-custom-secret-12345" });
        let output = r.redact(input).unwrap();
        let s = serde_json::to_string(&output).unwrap();
        assert!(s.contains(REDACTED), "custom pattern must be applied: {s}");
    }

    #[test]
    fn invalid_custom_pattern_returns_error() {
        let patterns = vec!["[invalid regex(".to_string()];
        assert!(
            Redactor::new(&patterns).is_err(),
            "invalid pattern should return error"
        );
    }
}
