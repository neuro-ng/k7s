//! Event DAO — Phase 21 (expert mode failure detection).
//!
//! Provides a thin typed wrapper over `k8s_openapi::api::core::v1::Event`
//! for listing cluster events as raw JSON values.

use k8s_openapi::api::core::v1::Event;
use kube::{Api, Client};

/// List all events in `namespace` (or all namespaces when `None`) as raw
/// JSON values, suitable for passing to `FailureDetector::check_event`.
pub async fn list_events(
    client: &Client,
    namespace: Option<&str>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let api: Api<Event> = match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };
    let list = api.list(&Default::default()).await?;
    let values = list
        .items
        .into_iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect();
    Ok(values)
}

#[cfg(test)]
mod tests {
    // Integration tests require a live cluster — unit tests live in expert.rs.
}
