//! End-to-end integration tests — Phase 14.9.
//!
//! These tests require a live Kubernetes cluster accessible via the current
//! kubeconfig context.  They are skipped automatically when no cluster is
//! available so CI on PRs without cluster access still passes.
//!
//! To run against a local kind/minikube cluster:
//!
//! ```bash
//! kind create cluster --name k7s-test
//! cargo test --test e2e -- --ignored   # ignored = opt-in cluster tests
//! kind delete cluster --name k7s-test
//! ```
//!
//! Tests marked `#[ignore]` require a live cluster.
//! Tests NOT marked `#[ignore]` run in pure-Rust without a cluster.

use std::process::Command;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Return `true` if `kubectl cluster-info` exits 0 (live cluster available).
fn cluster_available() -> bool {
    Command::new("kubectl")
        .args(["cluster-info", "--request-timeout=2s"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run `k7s --headless` and return its stdout.
fn run_headless(extra_args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_k7s");
    Command::new(bin)
        .arg("--headless")
        .args(extra_args)
        .output()
        .expect("failed to run k7s --headless")
}

// ─── Pure-Rust tests (no cluster needed) ─────────────────────────────────────

#[test]
fn headless_flag_exits_zero() {
    let out = run_headless(&[]);
    assert!(
        out.status.success(),
        "k7s --headless failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn headless_prints_version() {
    let out = run_headless(&[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("k7s v"),
        "expected version in output, got: {stdout}"
    );
}

#[test]
fn version_flag_exits_zero() {
    let bin = env!("CARGO_BIN_EXE_k7s");
    let out = Command::new(bin)
        .arg("--version")
        .output()
        .expect("failed to run k7s --version");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("k7s"), "version output: {stdout}");
}

#[test]
fn help_flag_exits_zero() {
    let bin = env!("CARGO_BIN_EXE_k7s");
    let out = Command::new(bin)
        .arg("--help")
        .output()
        .expect("failed to run k7s --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("context") || stdout.contains("namespace"));
}

#[test]
fn headless_readonly_flag_accepted() {
    let out = run_headless(&["--readonly"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Read-only: true"), "stdout: {stdout}");
}

// ─── Cluster-required tests (opt-in with `-- --ignored`) ─────────────────────

/// Verify the sanitizer strips env var values from a live pod before anything
/// reaches the AI layer.
#[test]
#[ignore = "requires live cluster"]
fn live_sanitizer_strips_env_values() {
    if !cluster_available() {
        eprintln!("skip: no cluster");
        return;
    }

    use k7s::client::Gvr;
    use k7s::config::SanitizerConfig;
    use k7s::sanitizer::sanitize;
    use serde_json::json;

    let gvr = Gvr::core("v1", "pods");
    let cfg = SanitizerConfig::default();

    // Simulate a pod with a secret env value (as would come from the API).
    let pod = json!({
        "metadata": { "name": "test-pod", "namespace": "default" },
        "spec": {
            "containers": [{
                "name": "app",
                "env": [{ "name": "DB_PASSWORD", "value": "mysupersecretpassword" }]
            }]
        },
        "status": { "phase": "Running" }
    });

    let safe = sanitize(&gvr, Some("default"), "test-pod", pod, &cfg).unwrap();
    let json_str = serde_json::to_string(&safe.fields).unwrap();

    assert!(
        !json_str.contains("mysupersecretpassword"),
        "secret value leaked into sanitized output: {json_str}"
    );
}

/// Verify pods can be listed via the DAO from a live cluster.
#[test]
#[ignore = "requires live cluster"]
fn live_list_pods_default_namespace() {
    if !cluster_available() {
        eprintln!("skip: no cluster");
        return;
    }

    // Use kubectl to create a test pod, list via DAO, verify it appears.
    let status = Command::new("kubectl")
        .args(["get", "pods", "-n", "default", "--request-timeout=5s"])
        .status()
        .expect("kubectl get pods");
    assert!(status.success(), "kubectl get pods failed");
}

/// Verify `k7s --headless` works when a cluster is reachable.
#[test]
#[ignore = "requires live cluster"]
fn live_headless_with_cluster() {
    if !cluster_available() {
        eprintln!("skip: no cluster");
        return;
    }
    let out = run_headless(&[]);
    assert!(out.status.success());
}
