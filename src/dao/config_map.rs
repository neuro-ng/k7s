//! ConfigMap DAO — Phase 3.9.
//!
//! ConfigMaps are displayed with keys only (no values) in the sanitized view.
//! The DAO provides full CRUD but the renderer enforces the sanitizer rules.
//!
//! # k9s Reference
//! `internal/dao/cm.go`

use async_trait::async_trait;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{Api, Client};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

pub struct ConfigMapDao {
    inner: GenericDao,
}

impl ConfigMapDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::config_maps()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<ConfigMap> {
        Api::namespaced(client.clone(), namespace)
    }

    /// Return only the keys of a ConfigMap (values are not shown per security model).
    pub async fn list_keys(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<Vec<String>> {
        let cm = self.api(client, namespace).get(name).await?;
        let keys = cm
            .data
            .map(|d| d.into_keys().collect())
            .unwrap_or_default();
        Ok(keys)
    }
}

impl Default for ConfigMapDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for ConfigMapDao {
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
impl Nuker for ConfigMapDao {
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
impl Describer for ConfigMapDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let cm = self.api(client, ns).get(name).await?;
        // Show only keys, not values (sanitizer rule).
        let keys: Vec<String> = cm
            .data
            .map(|d| d.into_keys().collect())
            .unwrap_or_default();
        Ok(format!(
            "ConfigMap: {name}\nNamespace: {ns}\nKeys ({}):\n  {}",
            keys.len(),
            keys.join("\n  ")
        ))
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
    fn config_map_dao_has_correct_gvr() {
        let dao = ConfigMapDao::new();
        assert_eq!(dao.gvr(), &well_known::config_maps());
    }
}
