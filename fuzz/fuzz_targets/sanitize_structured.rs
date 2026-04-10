//! Fuzz target: structured inputs that inject a known canary at arbitrary
//! positions in a Kubernetes-shaped JSON document.
//!
//! Properties tested:
//!   1. `sanitize()` never panics.
//!   2. The known canary string never appears in the sanitized output.
//!
//! The `Arbitrary` derive generates random but structurally valid inputs,
//! covering deep nesting and adversarial field injection that regex patterns
//! and field filters must both handle.
//!
//! Run with:
//!   cargo +nightly fuzz run sanitize_structured

#![no_main]

use arbitrary::Arbitrary;
use k7s::client::gvr::well_known;
use k7s::config::SanitizerConfig;
use k7s::sanitizer::sanitize;
use libfuzzer_sys::fuzz_target;

/// Canary injected at every user-controlled string position.
/// If this appears in sanitizer output, it is a leak.
const CANARY: &str = "FUZZ_SECRET_CANARY_9e3f2c";

/// Fuzz-generated Kubernetes-shaped document.
///
/// Fields mirror the JSON structure of real K8s resources so the fuzzer can
/// explore paths that the sanitizer must block.
#[derive(Debug, Arbitrary)]
struct FuzzResource {
    meta_name: String,
    meta_namespace: String,
    /// Injected into label values — must be blocked.
    label_value: String,
    /// Injected into annotation values — must be blocked.
    annotation_value: String,
    /// Injected into a container env var value — must be blocked.
    env_value: String,
    /// Injected into `spec.arbitrary_field` — must be dropped by field filter.
    spec_arbitrary: String,
    /// Injected into `data` values (ConfigMap / Secret) — must be blocked.
    data_value: String,
    /// Controls which GVR index [0..N) we exercise.
    gvr_index: u8,
}

fuzz_target!(|input: FuzzResource| {
    // Build a resource JSON with the canary substituted at every position.
    let label_val = if input.label_value.is_empty() {
        CANARY.to_owned()
    } else {
        format!("{CANARY}{}", input.label_value)
    };
    let ann_val = if input.annotation_value.is_empty() {
        CANARY.to_owned()
    } else {
        format!("{CANARY}{}", input.annotation_value)
    };
    let env_val = if input.env_value.is_empty() {
        CANARY.to_owned()
    } else {
        format!("{CANARY}{}", input.env_value)
    };
    let spec_val = if input.spec_arbitrary.is_empty() {
        CANARY.to_owned()
    } else {
        format!("{CANARY}{}", input.spec_arbitrary)
    };
    let data_val = if input.data_value.is_empty() {
        CANARY.to_owned()
    } else {
        format!("{CANARY}{}", input.data_value)
    };

    let raw = serde_json::json!({
        "metadata": {
            "name": input.meta_name,
            "namespace": input.meta_namespace,
            "labels": { "fuzz-label": label_val },
            "annotations": { "fuzz-annotation": ann_val }
        },
        "spec": {
            "containers": [{
                "name": "fuzz-container",
                "image": "fuzz:latest",
                "env": [{ "name": "FUZZ_VAR", "value": env_val }]
            }],
            "arbitraryField": spec_val
        },
        "data": { "fuzz-key": data_val },
        "stringData": { "fuzz-string-key": data_val }
    });

    let gvrs = [
        well_known::pods(),
        well_known::deployments(),
        well_known::secrets(),
        well_known::config_maps(),
        well_known::services(),
        well_known::nodes(),
        well_known::namespaces(),
        well_known::persistent_volumes(),
        well_known::roles(),
        well_known::cluster_role_bindings(),
    ];

    let gvr = &gvrs[input.gvr_index as usize % gvrs.len()];
    let cfg = SanitizerConfig::default();

    let result = sanitize(gvr, Some("default"), "fuzz", raw, &cfg);

    // Property 1: must not panic (enforced by reaching here).
    // Property 2: canary must not appear in sanitized output.
    if let Ok(safe) = result {
        let out = serde_json::to_string(&safe.fields).expect("output must be serializable");
        assert!(
            !out.contains(CANARY),
            "Canary leaked in {}: {out}",
            gvr
        );
    }
});
