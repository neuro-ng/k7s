//! Data sanitization pipeline — the security boundary between the cluster and the LLM.
//!
//! # Security Guarantee
//!
//! **No secret, token, password, or credential may pass through this module.**
//!
//! Every piece of cluster data destined for the AI layer MUST flow through
//! `sanitize()`. The pipeline is *default-deny*: only fields on the
//! per-GVR allowlist are forwarded; everything else is dropped.
//!
//! # Architecture
//!
//! ```text
//! raw K8s JSON
//!      │
//!      ▼
//! ┌─────────────────────────────────────┐
//! │  FieldFilter  (allowlist per GVR)   │ ← drops non-allowlisted fields
//! └──────────────────┬──────────────────┘
//!                    │ allowlisted JSON
//!                    ▼
//! ┌─────────────────────────────────────┐
//! │  Redactor  (regex pattern matching) │ ← redacts remaining secret-like values
//! └──────────────────┬──────────────────┘
//!                    │ redacted JSON
//!                    ▼
//!            SafeMetadata (output)
//! ```

pub mod filter;
pub mod log_analysis;
pub mod log_compressor;
pub mod redactor;

pub use filter::FieldFilter;
pub use log_analysis::{
    detect_temporal_patterns, summarise_stack_trace, SmartTruncator, StackTraceSummary,
    TemporalPattern, TraceRuntime, TruncatedText,
};
pub use log_compressor::{compress, CompressedLog, CompressionStats, LogLevel};
pub use redactor::Redactor;

use crate::client::Gvr;
use crate::config::SanitizerConfig;
use crate::error::SanitizeError;

/// The sanitized, LLM-safe representation of a Kubernetes resource.
///
/// # Construction
///
/// `SafeMetadata` is `#[non_exhaustive]` — it cannot be constructed with a
/// struct literal outside this crate.  All external code **must** produce
/// instances via [`sanitize()`], which enforces the full FieldFilter →
/// Redactor pipeline.  This is a compile-time guarantee: any code path that
/// bypasses `sanitize()` will fail to compile when building against the
/// published library.
///
/// Within the crate, tests may still construct `SafeMetadata { … }` for
/// mocking purposes, but production call sites must not.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct SafeMetadata {
    pub gvr: String,
    pub namespace: Option<String>,
    pub name: String,
    /// Sanitized fields. Only metadata and structural data remain.
    pub fields: serde_json::Value,
}

/// Sanitize a raw Kubernetes resource JSON value for transmission to an LLM.
///
/// # Errors
///
/// Returns `SanitizeError` if the field filter or redactor encounters an
/// internal error (e.g. a malformed custom regex pattern).
///
/// # Security
///
/// The caller must treat every `Ok(SafeMetadata)` as *potentially* containing
/// information the cluster operator considers sensitive at a metadata level
/// (e.g. resource names, image tags). The guarantee is that *secret material*
/// (tokens, passwords, TLS certs, connection strings) has been stripped.
pub fn sanitize(
    gvr: &Gvr,
    namespace: Option<&str>,
    name: &str,
    raw: serde_json::Value,
    cfg: &SanitizerConfig,
) -> Result<SafeMetadata, SanitizeError> {
    let ns = namespace.unwrap_or("");
    tracing::debug!(
        gvr = %gvr,
        namespace = ns,
        name = name,
        "sanitizer: starting pipeline"
    );

    // Step 1: field-level allowlist filter.
    let filter = FieldFilter::for_gvr(gvr);
    let filtered = filter.apply(raw).map_err(|e| SanitizeError::Resource {
        gvr: gvr.to_string(),
        ns: ns.to_string(),
        name: name.to_string(),
        source: e.into(),
    })?;

    tracing::debug!(
        gvr = %gvr,
        namespace = ns,
        name = name,
        "sanitizer: field filter applied"
    );

    // Step 2: pattern-based redaction of any remaining secret-like values.
    let redactor = Redactor::new(&cfg.custom_patterns)?;
    let before_redact = serde_json::to_string(&filtered).unwrap_or_default();
    let redacted = redactor.redact(filtered)?;
    let after_redact = serde_json::to_string(&redacted).unwrap_or_default();

    if before_redact != after_redact {
        tracing::warn!(
            gvr = %gvr,
            namespace = ns,
            name = name,
            "sanitizer: redactor modified values — secret-like patterns detected after field filter"
        );
    } else {
        tracing::debug!(
            gvr = %gvr,
            namespace = ns,
            name = name,
            "sanitizer: redactor pass clean — no patterns matched"
        );
    }

    Ok(SafeMetadata {
        gvr: gvr.to_string(),
        namespace: namespace.map(str::to_string),
        name: name.to_string(),
        fields: redacted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;
    use serde_json::json;

    fn default_cfg() -> SanitizerConfig {
        SanitizerConfig::default()
    }

    #[test]
    fn sanitize_pod_strips_secret_env_values() {
        let raw = json!({
            "metadata": {
                "name": "my-pod",
                "namespace": "default",
                "labels": { "app": "web" }
            },
            "spec": {
                "containers": [{
                    "name": "app",
                    "image": "nginx:1.25",
                    "env": [{
                        "name": "DB_PASSWORD",
                        "value": "super-secret-pass"
                    }]
                }]
            }
        });

        let result = sanitize(
            &well_known::pods(),
            Some("default"),
            "my-pod",
            raw,
            &default_cfg(),
        )
        .expect("sanitize should succeed");

        let s = serde_json::to_string(&result.fields).unwrap();
        assert!(
            !s.contains("super-secret-pass"),
            "env value must be stripped: {s}"
        );
    }

    #[test]
    fn sanitize_secret_resource_strips_data() {
        let raw = json!({
            "metadata": { "name": "my-secret", "namespace": "default" },
            "data": {
                "password": "dXNlcjpwYXNz",   // base64
                "api_key":  "c2Vuc2l0aXZl"
            }
        });

        let result = sanitize(
            &well_known::secrets(),
            Some("default"),
            "my-secret",
            raw,
            &default_cfg(),
        )
        .expect("sanitize should succeed");

        let s = serde_json::to_string(&result.fields).unwrap();
        assert!(
            !s.contains("dXNlcjpwYXNz"),
            "secret data must be stripped: {s}"
        );
    }

    #[test]
    fn sanitize_preserves_pod_name_and_labels() {
        let raw = json!({
            "metadata": {
                "name": "my-pod",
                "namespace": "default",
                "labels": { "app": "web", "version": "v1" }
            },
            "spec": {
                "containers": [{ "name": "app", "image": "nginx:latest" }]
            },
            "status": { "phase": "Running" }
        });

        let result = sanitize(
            &well_known::pods(),
            Some("default"),
            "my-pod",
            raw,
            &default_cfg(),
        )
        .unwrap();

        let s = serde_json::to_string(&result.fields).unwrap();
        assert!(s.contains("nginx:latest"), "image tag should be preserved");
        assert!(s.contains("Running"), "pod phase should be preserved");
    }
}
