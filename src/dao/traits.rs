use std::collections::BTreeMap;

use async_trait::async_trait;
use futures::stream::BoxStream;
use kube::Client;
use serde_json::Value;

use crate::client::Gvr;

/// A single Kubernetes resource as a raw JSON value plus its coordinates.
#[derive(Debug, Clone)]
pub struct Resource {
    pub gvr: Gvr,
    pub namespace: Option<String>,
    pub name: String,
    pub data: Value,
}

/// Options controlling delete behaviour.
#[derive(Debug, Clone)]
pub struct DeleteOptions {
    /// Cascade deletion to owned resources.
    pub propagation: PropagationPolicy,
    /// Override the finalizer grace period (seconds).
    pub grace_period: Option<i64>,
}

impl Default for DeleteOptions {
    fn default() -> Self {
        Self {
            propagation: PropagationPolicy::Background,
            grace_period: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropagationPolicy {
    Background,
    Foreground,
    Orphan,
}

/// Options for streaming container logs.
#[derive(Debug, Clone)]
pub struct LogOptions {
    /// Container name within the pod. Required when the pod has more than one.
    pub container: Option<String>,
    /// Number of log lines to tail from the end.
    pub tail_lines: Option<i64>,
    /// Whether to include timestamps.
    pub timestamps: bool,
    /// Retrieve logs from the previous (terminated) container instance.
    pub previous: bool,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            container: None,
            tail_lines: Some(200),
            timestamps: false,
            previous: false,
        }
    }
}

/// Core read operations — every resource type must implement this.
///
/// Corresponds to k9s's `Accessor` interface.
#[async_trait]
pub trait Accessor: Send + Sync {
    /// The GVR this DAO manages.
    fn gvr(&self) -> &Gvr;

    /// List all resources, optionally scoped to a namespace.
    ///
    /// Returns `None` namespace to list across all namespaces.
    async fn list(
        &self,
        client: &Client,
        namespace: Option<&str>,
    ) -> anyhow::Result<Vec<Resource>>;

    /// Fetch a single resource by namespace + name.
    async fn get(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<Resource>;
}

/// Delete a resource from the cluster.
///
/// Corresponds to k9s's `Nuker` interface.
#[async_trait]
pub trait Nuker: Accessor {
    async fn delete(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
        opts: DeleteOptions,
    ) -> anyhow::Result<()>;
}

/// Describe and YAML-dump a resource.
///
/// Corresponds to k9s's `Describer` interface.
#[async_trait]
pub trait Describer: Accessor {
    /// Human-readable description (analogous to `kubectl describe`).
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String>;

    /// Raw YAML representation of the resource manifest.
    async fn to_yaml(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String>;
}

/// Stream container logs from a pod.
///
/// Corresponds to k9s's `Loggable` interface.
#[async_trait]
pub trait Loggable: Accessor {
    /// Stream log lines from a pod container.
    ///
    /// Returns a pinned, boxed `Stream` of raw log line strings.
    /// Using `BoxStream` makes the trait object-safe.
    async fn tail_logs(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
        opts: LogOptions,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>>;
}

/// Scale a workload's replica count.
///
/// Corresponds to k9s's `Scalable` interface.
#[async_trait]
pub trait Scalable: Accessor {
    async fn scale(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
        replicas: i32,
    ) -> anyhow::Result<()>;
}

/// Trigger a rolling restart of a workload.
///
/// Corresponds to k9s's `Restartable` interface.
#[async_trait]
pub trait Restartable: Accessor {
    /// Annotates the pod template with the current timestamp to trigger
    /// a rolling restart without changing the spec.
    async fn restart(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<()>;
}

/// Metadata about a registered resource type.
#[derive(Debug, Clone)]
pub struct ResourceMeta {
    pub gvr: Gvr,
    /// Display name shown in the TUI header and breadcrumbs.
    pub display_name: String,
    /// Aliases for the command prompt (e.g. "po" for pods).
    pub aliases: Vec<String>,
    /// Whether the resource is namespace-scoped.
    pub namespaced: bool,
    /// Whether the resource supports log streaming.
    pub loggable: bool,
    /// Whether the resource can be scaled.
    pub scalable: bool,
    /// Custom column definitions (JSON path → display label).
    pub custom_columns: BTreeMap<String, String>,
}

impl ResourceMeta {
    pub fn new(
        gvr: Gvr,
        display_name: impl Into<String>,
        aliases: Vec<&str>,
        namespaced: bool,
    ) -> Self {
        Self {
            gvr,
            display_name: display_name.into(),
            aliases: aliases.into_iter().map(str::to_string).collect(),
            namespaced,
            loggable: false,
            scalable: false,
            custom_columns: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn resource_meta_aliases() {
        let meta = ResourceMeta::new(well_known::pods(), "Pods", vec!["po", "pod"], true);
        assert!(meta.aliases.contains(&"po".to_string()));
        assert!(meta.namespaced);
    }
}
