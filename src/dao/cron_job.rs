use async_trait::async_trait;
use k8s_openapi::api::batch::v1::{CronJob, Job, JobSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::PostParams;
use kube::{Api, Client};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

pub struct CronJobDao {
    inner: GenericDao,
}

impl CronJobDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::cron_jobs()),
        }
    }

    fn api(&self, client: &Client, namespace: &str) -> Api<CronJob> {
        Api::namespaced(client.clone(), namespace)
    }

    /// Manually trigger a CronJob by creating a Job from its spec template.
    ///
    /// Equivalent to `kubectl create job --from=cronjob/<name>`.
    pub async fn trigger(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<String> {
        let cj_api = self.api(client, namespace);
        let cj: CronJob = cj_api.get(name).await?;

        let job_spec: JobSpec = cj
            .spec
            .as_ref()
            .and_then(|s| s.job_template.spec.clone())
            .ok_or_else(|| anyhow::anyhow!("cronjob {name} has no job template spec"))?;

        // Generate a unique name for the manually-triggered job.
        let job_name = format!("{}-manual-{}", name, &uuid_suffix());

        let job = Job {
            metadata: ObjectMeta {
                name: Some(job_name.clone()),
                namespace: Some(namespace.to_owned()),
                annotations: Some({
                    let mut ann = std::collections::BTreeMap::new();
                    ann.insert(
                        "cronjob.kubernetes.io/instantiate".to_owned(),
                        "manual".to_owned(),
                    );
                    ann
                }),
                ..Default::default()
            },
            spec: Some(job_spec),
            ..Default::default()
        };

        let job_api: Api<Job> = Api::namespaced(client.clone(), namespace);
        job_api.create(&PostParams::default(), &job).await?;

        tracing::info!(
            cronjob = name,
            job = job_name,
            namespace,
            "cronjob triggered manually"
        );
        Ok(job_name)
    }
}

impl Default for CronJobDao {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a short random suffix for manual job names (e.g. "a3f2b1").
fn uuid_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_millis();
    format!("{:06x}", millis)
}

#[async_trait]
impl Accessor for CronJobDao {
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
impl Nuker for CronJobDao {
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
impl Describer for CronJobDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        let ns = namespace.unwrap_or("default");
        let api = self.api(client, ns);
        let cj: CronJob = api.get(name).await?;

        let mut lines = Vec::new();
        lines.push(format!("Name:             {}", name));
        lines.push(format!("Namespace:        {}", ns));

        if let Some(spec) = &cj.spec {
            lines.push(format!("Schedule:         {}", spec.schedule));
            lines.push(format!(
                "Suspend:          {}",
                spec.suspend.unwrap_or(false)
            ));
            lines.push(format!(
                "Concurrency:      {}",
                spec.concurrency_policy.as_deref().unwrap_or("Allow")
            ));
            if let Some(keep) = spec.successful_jobs_history_limit {
                lines.push(format!("SuccessHistory:   {}", keep));
            }
            if let Some(keep) = spec.failed_jobs_history_limit {
                lines.push(format!("FailedHistory:    {}", keep));
            }
        }

        if let Some(status) = &cj.status {
            if let Some(last) = &status.last_schedule_time {
                lines.push(format!("LastSchedule:     {}", last.0));
            }
            lines.push(format!(
                "Active Jobs:      {}",
                status.active.as_ref().map_or(0, |a| a.len())
            ));
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
    fn cronjob_dao_gvr() {
        let dao = CronJobDao::new();
        assert_eq!(*dao.gvr(), well_known::cron_jobs());
    }

    #[test]
    fn uuid_suffix_is_six_chars() {
        let s = uuid_suffix();
        assert_eq!(s.len(), 6);
    }
}
