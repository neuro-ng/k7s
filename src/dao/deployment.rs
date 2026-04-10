use async_trait::async_trait;
use chrono::Utc;
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::json;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{
    Accessor, DeleteOptions, Describer, Nuker, Resource, Restartable, Scalable,
};

pub struct DeploymentDao {
    inner: GenericDao,
}

impl DeploymentDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::deployments()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<Deployment> {
        Api::namespaced(client.clone(), namespace)
    }
}

impl Default for DeploymentDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for DeploymentDao {
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
impl Nuker for DeploymentDao {
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
impl Describer for DeploymentDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let api = self.api(client, ns);
        let dp: Deployment = api.get(name).await?;

        let mut lines = Vec::new();
        lines.push(format!("Name:      {}", name));
        lines.push(format!("Namespace: {}", ns));

        if let Some(spec) = &dp.spec {
            lines.push(format!("Replicas:  {}", spec.replicas.unwrap_or(1)));
            if let Some(selector) = &spec.selector.match_labels {
                lines.push("Selector:".to_owned());
                for (k, v) in selector {
                    lines.push(format!("  {}={}", k, v));
                }
            }
        }

        if let Some(status) = &dp.status {
            lines.push(format!(
                "Ready:     {}/{}",
                status.ready_replicas.unwrap_or(0),
                status.replicas.unwrap_or(0)
            ));
            lines.push(format!(
                "Updated:   {}",
                status.updated_replicas.unwrap_or(0)
            ));
            lines.push(format!(
                "Available: {}",
                status.available_replicas.unwrap_or(0)
            ));

            if let Some(conditions) = &status.conditions {
                lines.push("Conditions:".to_owned());
                for c in conditions {
                    lines.push(format!("  {} = {}", c.type_, c.status));
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
impl Scalable for DeploymentDao {
    async fn scale(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
        replicas: i32,
    ) -> anyhow::Result<()> {
        let api = self.api(client, namespace);
        let patch = json!({
            "spec": { "replicas": replicas }
        });
        api.patch(name, &PatchParams::apply("k7s"), &Patch::Merge(&patch))
            .await?;

        tracing::info!(deployment = name, namespace, replicas, "deployment scaled");
        Ok(())
    }
}

#[async_trait]
impl Restartable for DeploymentDao {
    /// Trigger a rolling restart by annotating the pod template with a timestamp.
    ///
    /// Equivalent to `kubectl rollout restart deployment/<name>`.
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

        tracing::info!(deployment = name, namespace, "deployment restart triggered");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn deployment_dao_gvr() {
        let dao = DeploymentDao::new();
        assert_eq!(*dao.gvr(), well_known::deployments());
    }
}
