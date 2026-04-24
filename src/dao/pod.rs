use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::api::LogParams;
use kube::{Api, Client};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::{dynamic_to_resource, GenericDao};
use crate::dao::traits::{
    Accessor, DeleteOptions, Describer, LogOptions, Loggable, Nuker, Resource,
};

pub struct PodDao {
    inner: GenericDao,
}

impl PodDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::pods()),
        }
    }

    fn pod_api(&self, client: &Client, namespace: &str) -> Api<Pod> {
        Api::namespaced(client.clone(), namespace)
    }
}

impl Default for PodDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for PodDao {
    fn gvr(&self) -> &Gvr {
        self.inner.gvr()
    }

    async fn list(
        &self,
        client: &Client,
        namespace: Option<&str>,
    ) -> anyhow::Result<Vec<Resource>> {
        self.inner.list(client, namespace).await
    }

    async fn get(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<Resource> {
        self.inner.get(client, namespace, name).await
    }
}

#[async_trait]
impl Nuker for PodDao {
    async fn delete(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
        opts: DeleteOptions,
    ) -> anyhow::Result<()> {
        self.inner.delete(client, namespace, name, opts).await
    }
}

#[async_trait]
impl Describer for PodDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let api = self.pod_api(client, ns);
        let pod: Pod = api.get(name).await?;

        let mut lines = Vec::new();
        lines.push(format!("Name:      {}", name));
        lines.push(format!("Namespace: {}", ns));

        // Status
        if let Some(status) = &pod.status {
            lines.push(format!(
                "Phase:     {}",
                status.phase.as_deref().unwrap_or("Unknown")
            ));
            lines.push(format!(
                "IP:        {}",
                status.pod_ip.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "Node:      {}",
                pod.spec
                    .as_ref()
                    .and_then(|s| s.node_name.as_deref())
                    .unwrap_or("-")
            ));

            if let Some(conditions) = &status.conditions {
                lines.push("Conditions:".to_owned());
                for c in conditions {
                    lines.push(format!("  {} = {}", c.type_, c.status));
                }
            }

            if let Some(container_statuses) = &status.container_statuses {
                lines.push("Containers:".to_owned());
                for cs in container_statuses {
                    let ready = if cs.ready { "Ready" } else { "NotReady" };
                    lines.push(format!(
                        "  {} ({}) restarts={}",
                        cs.name, ready, cs.restart_count
                    ));
                }
            }
        }

        Ok(lines.join("\n"))
    }

    async fn to_yaml(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        self.inner.to_yaml(client, namespace, name).await
    }
}

#[async_trait]
impl Loggable for PodDao {
    async fn tail_logs(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
        opts: LogOptions,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        let api = self.pod_api(client, namespace);

        let params = LogParams {
            container: opts.container.clone(),
            follow: true,
            tail_lines: opts.tail_lines,
            timestamps: opts.timestamps,
            previous: opts.previous,
            ..Default::default()
        };

        // log_stream returns impl AsyncBufRead; convert to a line Stream.
        let reader = api.log_stream(name, &params).await?;
        let line_stream = reader
            .lines()
            .map(|result| result.map_err(|e| anyhow::anyhow!("log stream error: {e}")));

        Ok(Box::pin(line_stream))
    }
}

/// List all pods in the given namespace (or all namespaces if `None`)
/// as raw JSON values, suitable for passing to `FailureDetector`.
pub async fn list_pods(
    client: &Client,
    namespace: Option<&str>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let api: Api<Pod> = match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };
    let list = api.list(&Default::default()).await?;
    let values = list
        .items
        .into_iter()
        .filter_map(|p| serde_json::to_value(p).ok())
        .collect();
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn pod_dao_gvr_is_pods() {
        let dao = PodDao::new();
        assert_eq!(*dao.gvr(), well_known::pods());
    }

    #[test]
    fn log_options_default_tail() {
        let opts = LogOptions::default();
        assert_eq!(opts.tail_lines, Some(200));
        assert!(!opts.previous);
    }
}
