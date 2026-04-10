use async_trait::async_trait;
use chrono::Utc;
use k8s_openapi::api::apps::v1::StatefulSet;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::json;

use crate::client::Gvr;
use crate::client::gvr::well_known;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource, Restartable, Scalable};

pub struct StatefulSetDao {
    inner: GenericDao,
}

impl StatefulSetDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::stateful_sets()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<StatefulSet> {
        Api::namespaced(client.clone(), namespace)
    }
}

impl Default for StatefulSetDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for StatefulSetDao {
    fn gvr(&self) -> &Gvr {
        self.inner.gvr()
    }

    async fn list(&self, client: &Client, namespace: Option<&str>) -> anyhow::Result<Vec<Resource>> {
        self.inner.list(client, namespace).await
    }

    async fn get(&self, client: &Client, namespace: Option<&str>, name: &str) -> anyhow::Result<Resource> {
        self.inner.get(client, namespace, name).await
    }
}

#[async_trait]
impl Nuker for StatefulSetDao {
    async fn delete(&self, client: &Client, namespace: Option<&str>, name: &str, opts: DeleteOptions) -> anyhow::Result<()> {
        self.inner.delete(client, namespace, name, opts).await
    }
}

#[async_trait]
impl Describer for StatefulSetDao {
    async fn describe(&self, client: &Client, namespace: Option<&str>, name: &str) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let api = self.api(client, ns);
        let sts: StatefulSet = api.get(name).await?;

        let mut lines = Vec::new();
        lines.push(format!("Name:        {}", name));
        lines.push(format!("Namespace:   {}", ns));

        if let Some(spec) = &sts.spec {
            lines.push(format!("Replicas:    {}", spec.replicas.unwrap_or(1)));
            lines.push(format!("ServiceName: {}", spec.service_name));
            if let Some(selector) = &spec.selector.match_labels {
                lines.push("Selector:".to_owned());
                for (k, v) in selector {
                    lines.push(format!("  {}={}", k, v));
                }
            }
        }

        if let Some(status) = &sts.status {
            lines.push(format!(
                "Ready:       {}/{}",
                status.ready_replicas.unwrap_or(0),
                status.replicas
            ));
            lines.push(format!("Updated:     {}", status.updated_replicas.unwrap_or(0)));
            lines.push(format!("Current:     {}", status.current_replicas.unwrap_or(0)));
        }

        Ok(lines.join("\n"))
    }

    async fn to_yaml(&self, client: &Client, namespace: Option<&str>, name: &str) -> anyhow::Result<String> {
        self.inner.to_yaml(client, namespace, name).await
    }
}

#[async_trait]
impl Scalable for StatefulSetDao {
    async fn scale(&self, client: &Client, namespace: &str, name: &str, replicas: i32) -> anyhow::Result<()> {
        let api = self.api(client, namespace);
        let patch = json!({ "spec": { "replicas": replicas } });
        api.patch(name, &PatchParams::apply("k7s"), &Patch::Merge(&patch)).await?;
        tracing::info!(statefulset = name, namespace, replicas, "statefulset scaled");
        Ok(())
    }
}

#[async_trait]
impl Restartable for StatefulSetDao {
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
        api.patch(name, &PatchParams::apply("k7s"), &Patch::Merge(&patch)).await?;
        tracing::info!(statefulset = name, namespace, "statefulset restart triggered");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn statefulset_dao_gvr() {
        let dao = StatefulSetDao::new();
        assert_eq!(*dao.gvr(), well_known::stateful_sets());
    }
}
