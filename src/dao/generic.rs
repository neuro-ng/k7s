use async_trait::async_trait;
use kube::api::{ApiResource, DeleteParams, DynamicObject, GroupVersionKind};
use kube::{Api, Client};

use crate::client::Gvr;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, PropagationPolicy, Resource};

/// DAO that works with any Kubernetes resource via the dynamic API.
///
/// Used for CRDs and any resource type not covered by a typed DAO.
/// Also used as the base implementation that typed DAOs delegate to.
pub struct GenericDao {
    gvr: Gvr,
}

impl GenericDao {
    pub fn new(gvr: Gvr) -> Self {
        Self { gvr }
    }

    fn api_resource(&self) -> ApiResource {
        let gvk = GroupVersionKind {
            group: self.gvr.group.clone(),
            version: self.gvr.version.clone(),
            kind: crate::watch::factory::resource_to_kind(&self.gvr.resource),
        };
        ApiResource::from_gvk(&gvk)
    }

    fn make_api(&self, client: &Client, namespace: Option<&str>) -> Api<DynamicObject> {
        let ar = self.api_resource();
        match namespace {
            Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
            None => Api::all_with(client.clone(), &ar),
        }
    }
}

#[async_trait]
impl Accessor for GenericDao {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    async fn list(
        &self,
        client: &Client,
        namespace: Option<&str>,
    ) -> anyhow::Result<Vec<Resource>> {
        let api = self.make_api(client, namespace);
        let list = api.list(&Default::default()).await?;

        let resources = list
            .items
            .into_iter()
            .map(|obj| dynamic_to_resource(&self.gvr, obj))
            .collect();

        Ok(resources)
    }

    async fn get(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<Resource> {
        let api = self.make_api(client, namespace);
        let obj = api.get(name).await?;
        Ok(dynamic_to_resource(&self.gvr, obj))
    }
}

#[async_trait]
impl Nuker for GenericDao {
    async fn delete(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
        opts: DeleteOptions,
    ) -> anyhow::Result<()> {
        let api = self.make_api(client, namespace);
        let dp = delete_params_from_opts(&opts);
        api.delete(name, &dp).await?;
        Ok(())
    }
}

#[async_trait]
impl Describer for GenericDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let resource = self.get(client, namespace, name).await?;
        // Simple description: emit key fields as text.
        let mut lines = Vec::new();
        lines.push(format!("Name:      {}", resource.name));
        if let Some(ns) = &resource.namespace {
            lines.push(format!("Namespace: {}", ns));
        }
        lines.push(format!("GVR:       {}", resource.gvr));

        if let Some(meta) = resource.data.get("metadata") {
            if let Some(labels) = meta.get("labels").and_then(|v| v.as_object()) {
                lines.push("Labels:".to_owned());
                for (k, v) in labels {
                    lines.push(format!("  {}: {}", k, v.as_str().unwrap_or("-")));
                }
            }
            if let Some(anns) = meta.get("annotations").and_then(|v| v.as_object()) {
                lines.push("Annotations:".to_owned());
                for (k, v) in anns {
                    lines.push(format!("  {}: {}", k, v.as_str().unwrap_or("-")));
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
        let resource = self.get(client, namespace, name).await?;
        Ok(serde_yaml::to_string(&resource.data)?)
    }
}

/// Convert a `DynamicObject` into our internal `Resource` type.
pub fn dynamic_to_resource(gvr: &Gvr, obj: DynamicObject) -> Resource {
    let namespace = obj.metadata.namespace.clone();
    let name = obj.metadata.name.clone().unwrap_or_default();
    let data = serde_json::to_value(&obj).unwrap_or(serde_json::Value::Null);
    Resource {
        gvr: gvr.clone(),
        namespace,
        name,
        data,
    }
}

fn delete_params_from_opts(opts: &DeleteOptions) -> DeleteParams {
    let propagation_policy = match opts.propagation {
        PropagationPolicy::Background => Some(kube::api::PropagationPolicy::Background),
        PropagationPolicy::Foreground => Some(kube::api::PropagationPolicy::Foreground),
        PropagationPolicy::Orphan => Some(kube::api::PropagationPolicy::Orphan),
    };

    let mut dp = DeleteParams {
        propagation_policy,
        ..Default::default()
    };
    if let Some(grace) = opts.grace_period {
        dp.grace_period_seconds = Some(grace as u32);
    }
    dp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn generic_dao_gvr() {
        let dao = GenericDao::new(well_known::pods());
        assert_eq!(dao.gvr().resource, "pods");
    }
}
