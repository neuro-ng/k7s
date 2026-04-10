use async_trait::async_trait;
use k8s_openapi::api::batch::v1::Job;
use kube::{Api, Client};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

pub struct JobDao {
    inner: GenericDao,
}

impl JobDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::jobs()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<Job> {
        Api::namespaced(client.clone(), namespace)
    }
}

impl Default for JobDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for JobDao {
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
impl Nuker for JobDao {
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
impl Describer for JobDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let api = self.api(client, ns);
        let job: Job = api.get(name).await?;

        let mut lines = Vec::new();
        lines.push(format!("Name:        {}", name));
        lines.push(format!("Namespace:   {}", ns));

        if let Some(spec) = &job.spec {
            lines.push(format!("Completions: {}", spec.completions.unwrap_or(1)));
            lines.push(format!("Parallelism: {}", spec.parallelism.unwrap_or(1)));
            if let Some(deadline) = spec.active_deadline_seconds {
                lines.push(format!("Deadline:    {}s", deadline));
            }
        }

        if let Some(status) = &job.status {
            lines.push(format!("Active:      {}", status.active.unwrap_or(0)));
            lines.push(format!("Succeeded:   {}", status.succeeded.unwrap_or(0)));
            lines.push(format!("Failed:      {}", status.failed.unwrap_or(0)));

            if let Some(conditions) = &status.conditions {
                lines.push("Conditions:".to_owned());
                for c in conditions {
                    lines.push(format!(
                        "  {} = {} ({})",
                        c.type_,
                        c.status,
                        c.message.as_deref().unwrap_or("")
                    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;

    #[test]
    fn job_dao_gvr() {
        let dao = JobDao::new();
        assert_eq!(*dao.gvr(), well_known::jobs());
    }
}
