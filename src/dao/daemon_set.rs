use async_trait::async_trait;
use chrono::Utc;
use k8s_openapi::api::apps::v1::DaemonSet;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::json;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource, Restartable};

pub struct DaemonSetDao {
    inner: GenericDao,
}

impl DaemonSetDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::daemon_sets()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<DaemonSet> {
        Api::namespaced(client.clone(), namespace)
    }
}

impl Default for DaemonSetDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for DaemonSetDao {
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
impl Nuker for DaemonSetDao {
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
impl Describer for DaemonSetDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let api = self.api(client, ns);
        let ds: DaemonSet = api.get(name).await?;

        let mut lines = Vec::new();
        lines.push(format!("Name:            {}", name));
        lines.push(format!("Namespace:       {}", ns));

        if let Some(spec) = &ds.spec {
            if let Some(selector) = &spec.selector.match_labels {
                lines.push("Selector:".to_owned());
                for (k, v) in selector {
                    lines.push(format!("  {}={}", k, v));
                }
            }
        }

        if let Some(status) = &ds.status {
            lines.push(format!(
                "Desired:         {}",
                status.desired_number_scheduled
            ));
            lines.push(format!(
                "Current:         {}",
                status.current_number_scheduled
            ));
            lines.push(format!("Ready:           {}", status.number_ready));
            lines.push(format!(
                "Up-to-date:      {}",
                status.updated_number_scheduled.unwrap_or(0)
            ));
            lines.push(format!(
                "Available:       {}",
                status.number_available.unwrap_or(0)
            ));
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
impl Restartable for DaemonSetDao {
    async fn restart(&self, client: &Client, namespace: &str, name: &str) -> anyhow::Result<()> {
        let api = self.api(client, namespace);
        let now = Utc::now().to_rfc3339();
        let patch = json!({
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            "kubectl.kubernetes.io/restartedAt": now
                        }
                    }
                }
            }
        });
        api.patch(name, &PatchParams::apply("k7s"), &Patch::Merge(&patch))
            .await?;
        tracing::info!(daemonset = name, namespace, "daemonset restart triggered");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn daemonset_dao_gvr() {
        let dao = DaemonSetDao::new();
        assert_eq!(*dao.gvr(), well_known::daemon_sets());
    }
}
