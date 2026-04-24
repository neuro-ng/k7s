//! Service DAO — Phase 3.6.
//!
//! Services are read-only from k7s's perspective (no scaling, no restart).
//! Port-forward operations are handled separately by `PortForwardManager`.
//!
//! # k9s Reference
//! `internal/dao/svc.go`

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use kube::{Api, Client};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

pub struct ServiceDao {
    inner: GenericDao,
}

impl ServiceDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::services()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<Service> {
        Api::namespaced(client.clone(), namespace)
    }
}

impl Default for ServiceDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for ServiceDao {
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
impl Nuker for ServiceDao {
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
impl Describer for ServiceDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let svc = self.api(client, ns).get(name).await?;
        Ok(format!("{svc:#?}"))
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
    fn service_dao_has_correct_gvr() {
        let dao = ServiceDao::new();
        assert_eq!(dao.gvr(), &well_known::services());
    }
}
