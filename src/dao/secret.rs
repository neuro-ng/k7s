//! Secret DAO — Phase 3.10.
//!
//! Secrets are displayed with type + key count only (no values ever shown).
//! The sanitizer enforces this: `v1/Secret.data` is in the blocked list.
//!
//! # k9s Reference
//! `internal/dao/secret.go`

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

pub struct SecretDao {
    inner: GenericDao,
}

impl SecretDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::secrets()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<Secret> {
        Api::namespaced(client.clone(), namespace)
    }

    /// Return only the key names of a Secret (values are never revealed).
    pub async fn list_keys(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<Vec<String>> {
        let secret = self.api(client, namespace).get(name).await?;
        let keys = secret
            .data
            .map(|d| d.into_keys().collect())
            .unwrap_or_default();
        Ok(keys)
    }

    /// Return the secret type (e.g. `"kubernetes.io/tls"`).
    pub async fn secret_type(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<String> {
        let secret = self.api(client, namespace).get(name).await?;
        Ok(secret
            .type_
            .unwrap_or_else(|| "Opaque".to_owned()))
    }
}

impl Default for SecretDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for SecretDao {
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
impl Nuker for SecretDao {
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
impl Describer for SecretDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let secret = self.api(client, ns).get(name).await?;
        let key_count = secret.data.map(|d| d.len()).unwrap_or(0);
        let secret_type = secret.type_.unwrap_or_else(|| "Opaque".to_owned());
        // Values are NEVER shown — security model requires key-name-only.
        Ok(format!(
            "Secret: {name}\nNamespace: {ns}\nType: {secret_type}\nKeys: {key_count} (values redacted)"
        ))
    }

    async fn to_yaml(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        // YAML representation omits data values (sanitizer model).
        let res = self.inner.get(client, namespace, name).await?;
        let mut value = res.data;
        // Strip all data values, keep only structure.
        if let Some(data) = value.get_mut("data") {
            if let Some(obj) = data.as_object_mut() {
                for v in obj.values_mut() {
                    *v = serde_json::Value::String("[REDACTED]".to_owned());
                }
            }
        }
        serde_yaml::to_string(&value).map_err(|e| anyhow::anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_dao_has_correct_gvr() {
        let dao = SecretDao::new();
        assert_eq!(dao.gvr(), &well_known::secrets());
    }
}
