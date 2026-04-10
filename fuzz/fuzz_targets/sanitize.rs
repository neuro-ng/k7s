//! Fuzz target: unstructured JSON input through the sanitizer pipeline.
//!
//! Property tested: `sanitize()` never panics on arbitrary bytes that parse
//! as valid JSON.
//!
//! Run with:
//!   cargo +nightly fuzz run sanitize
//!
//! Recommended corpus seed:
//!   mkdir -p fuzz/corpus/sanitize
//!   echo '{}' > fuzz/corpus/sanitize/empty.json
//!   echo '{"metadata":{"name":"pod"},"spec":{"containers":[]}}' \
//!     > fuzz/corpus/sanitize/pod.json

#![no_main]

use k7s::client::gvr::well_known;
use k7s::config::SanitizerConfig;
use k7s::sanitizer::sanitize;
use libfuzzer_sys::fuzz_target;

// Exercise a representative subset of GVRs on every input.
static GVRS: std::sync::LazyLock<Vec<k7s::client::Gvr>> = std::sync::LazyLock::new(|| {
    vec![
        well_known::pods(),
        well_known::deployments(),
        well_known::secrets(),
        well_known::config_maps(),
        well_known::services(),
        well_known::nodes(),
        well_known::namespaces(),
        well_known::persistent_volumes(),
        well_known::persistent_volume_claims(),
        well_known::roles(),
        well_known::cluster_role_bindings(),
        k7s::client::Gvr::new("custom.io", "v1alpha1", "widgets"), // unknown GVR
    ]
});

fuzz_target!(|data: &[u8]| {
    // Skip non-JSON bytes — we're fuzzing sanitizer logic, not the JSON parser.
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };

    let cfg = SanitizerConfig::default();

    for gvr in GVRS.iter() {
        // Must never panic regardless of input shape.
        let _ = sanitize(gvr, Some("default"), "fuzz", value.clone(), &cfg);
    }
});
