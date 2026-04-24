//! Node DAO — Phase 3.7.
//!
//! Provides cordon, uncordon, and drain operations on cluster nodes.
//! Drain is implemented by delegating to `kubectl drain` (same approach as
//! k9s) since the drain algorithm is complex and kubectl handles eviction
//! policy correctly.
//!
//! # k9s Reference
//! `internal/dao/node.go`

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Node;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::json;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

pub struct NodeDao {
    inner: GenericDao,
}

impl NodeDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::nodes()),
        }
    }

    fn api(&self, client: &Client) -> Api<Node> {
        Api::all(client.clone())
    }

    /// Cordon a node — mark it as unschedulable.
    pub async fn cordon(&self, client: &Client, name: &str) -> anyhow::Result<()> {
        self.set_unschedulable(client, name, true).await
    }

    /// Uncordon a node — re-enable scheduling.
    pub async fn uncordon(&self, client: &Client, name: &str) -> anyhow::Result<()> {
        self.set_unschedulable(client, name, false).await
    }

    /// Drain a node by delegating to `kubectl drain`.
    ///
    /// Flags: `--ignore-daemonsets --delete-emptydir-data --force`
    pub async fn drain(&self, name: &str) -> anyhow::Result<()> {
        let status = tokio::process::Command::new("kubectl")
            .args([
                "drain",
                name,
                "--ignore-daemonsets",
                "--delete-emptydir-data",
                "--force",
            ])
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("kubectl drain {name} failed with exit code {:?}", status.code())
        }
    }

    async fn set_unschedulable(
        &self,
        client: &Client,
        name: &str,
        unschedulable: bool,
    ) -> anyhow::Result<()> {
        let patch = json!({"spec": {"unschedulable": unschedulable}});
        self.api(client)
            .patch(
                name,
                &PatchParams::apply("k7s"),
                &Patch::Merge(patch),
            )
            .await?;
        Ok(())
    }
}

impl Default for NodeDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for NodeDao {
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
impl Nuker for NodeDao {
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
impl Describer for NodeDao {
    async fn describe(
        &self,
        client: &Client,
        _namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let node = self.api(client).get(name).await?;
        Ok(format!("{node:#?}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_dao_has_correct_gvr() {
        let dao = NodeDao::new();
        assert_eq!(dao.gvr(), &well_known::nodes());
    }
}
