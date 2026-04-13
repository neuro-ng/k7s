//! Metrics server detection — Phase 1.8.
//!
//! Checks whether a metrics-server is available on the cluster by querying the
//! `metrics.k8s.io` API group.  Used to decide whether to show CPU/memory live
//! metrics in the pod/node views.
//!
//! # k9s Reference
//! `internal/client/metrics.go`

use kube::Client;

/// Return `true` if the `metrics.k8s.io/v1beta1` API is registered on the cluster.
///
/// A `false` result means the metrics-server is absent or unreachable — the
/// caller should hide or disable metrics columns.
///
/// This is a best-effort probe: network errors are treated as "not available"
/// rather than propagated as errors.
pub async fn detect_metrics_server(client: &Client) -> bool {
    // The metrics API lives at /apis/metrics.k8s.io/v1beta1.
    // We probe the API-group-level path: /apis/metrics.k8s.io
    // kube's discovery helper is the cleanest way to do this.
    match probe_metrics_group(client).await {
        Ok(available) => {
            if available {
                tracing::debug!("metrics-server detected (metrics.k8s.io available)");
            } else {
                tracing::debug!("metrics-server not detected");
            }
            available
        }
        Err(e) => {
            tracing::debug!(error = %e, "metrics-server probe failed");
            false
        }
    }
}

async fn probe_metrics_group(client: &Client) -> Result<bool, kube::Error> {
    // List all API groups and look for "metrics.k8s.io".
    let groups = client.list_api_groups().await?;
    let found = groups.groups.iter().any(|g| g.name == "metrics.k8s.io");
    Ok(found)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Unit tests for probe logic can't easily use a live cluster; the module
    // is verified to compile correctly by the Rust compiler itself.

    #[test]
    fn probe_function_exists() {
        // Verify the function is accessible and has the expected signature shape.
        // `detect_metrics_server` is an async fn returning bool — we just need
        // it to compile; actual invocation requires a live cluster.
        fn _assert_signature(c: &kube::Client) -> impl std::future::Future<Output = bool> + '_ {
            super::detect_metrics_server(c)
        }
    }
}
