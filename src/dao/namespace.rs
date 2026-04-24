//! Namespace DAO — Phase 3.8.
//!
//! Namespaces are cluster-scoped resources. The main operations are list
//! (for the namespace switcher) and delete.
//!
//! # k9s Reference
//! `internal/dao/ns.go`

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Namespace;
use kube::{Api, Client};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

pub struct NamespaceDao {
    inner: GenericDao,
}

impl NamespaceDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::namespaces()),
        }
    }

    fn api(&self, client: &Client) -> Api<Namespace> {
        Api::all(client.clone())
    }

    /// Return all namespace names visible to the current credentials.
    pub async fn list_names(&self, client: &Client) -> anyhow::Result<Vec<String>> {
        let ns_list = self.api(client).list(&Default::default()).await?;
        let names = ns_list
            .items
            .into_iter()
            .filter_map(|ns| ns.metadata.name)
            .collect();
        Ok(names)
    }
}

impl Default for NamespaceDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for NamespaceDao {
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
impl Nuker for NamespaceDao {
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
impl Describer for NamespaceDao {
    async fn describe(
        &self,
        client: &Client,
        _namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = self.api(client).get(name).await?;
        Ok(format!("{ns:#?}"))
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
    fn namespace_dao_has_correct_gvr() {
        let dao = NamespaceDao::new();
        assert_eq!(dao.gvr(), &well_known::namespaces());
    }
}
