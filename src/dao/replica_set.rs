use async_trait::async_trait;
use k8s_openapi::api::apps::v1::ReplicaSet;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::json;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource, Scalable};

pub struct ReplicaSetDao {
    inner: GenericDao,
}

impl ReplicaSetDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::replica_sets()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<ReplicaSet> {
        Api::namespaced(client.clone(), namespace)
    }
}

impl Default for ReplicaSetDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for ReplicaSetDao {
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
impl Nuker for ReplicaSetDao {
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
impl Describer for ReplicaSetDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let api = self.api(client, ns);
        let rs: ReplicaSet = api.get(name).await?;

        let mut lines = Vec::new();
        lines.push(format!("Name:      {}", name));
        lines.push(format!("Namespace: {}", ns));

        if let Some(spec) = &rs.spec {
            lines.push(format!("Replicas:  {}", spec.replicas.unwrap_or(1)));
            if let Some(selector) = &spec.selector.match_labels {
                lines.push("Selector:".to_owned());
                for (k, v) in selector {
                    lines.push(format!("  {}={}", k, v));
                }
            }
        }

        if let Some(status) = &rs.status {
            lines.push(format!(
                "Ready:     {}/{}",
                status.ready_replicas.unwrap_or(0),
                status.replicas
            ));
            lines.push(format!(
                "Available: {}",
                status.available_replicas.unwrap_or(0)
            ));
        }

        // Owner reference (usually a Deployment).
        if let Some(owners) = &rs.metadata.owner_references {
            for owner in owners {
                lines.push(format!("Owned by:  {}/{}", owner.kind, owner.name));
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
impl Scalable for ReplicaSetDao {
    async fn scale(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
        replicas: i32,
    ) -> anyhow::Result<()> {
        let api = self.api(client, namespace);
        let patch = json!({ "spec": { "replicas": replicas } });
        api.patch(name, &PatchParams::apply("k7s"), &Patch::Merge(&patch))
            .await?;
        tracing::info!(replicaset = name, namespace, replicas, "replicaset scaled");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn replicaset_dao_gvr() {
        let dao = ReplicaSetDao::new();
        assert_eq!(*dao.gvr(), well_known::replica_sets());
    }
}
