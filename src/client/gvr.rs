use std::fmt;

/// Group/Version/Resource identifier for a Kubernetes resource type.
///
/// Examples:
/// - `apps/v1/deployments`
/// - `v1/pods` (core group — group is empty string)
/// - `batch/v1/cronjobs`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Gvr {
    /// API group. Empty string for the core group (formerly "v1").
    pub group: String,
    /// API version (e.g. "v1", "v1beta1").
    pub version: String,
    /// Resource plural name in lowercase (e.g. "pods", "deployments").
    pub resource: String,
}

impl Gvr {
    /// Construct a core-group GVR (group = "").
    ///
    /// ```
    /// # use k7s::client::Gvr;
    /// let pods = Gvr::core("v1", "pods");
    /// assert_eq!(pods.group, "");
    /// ```
    pub fn core(version: impl Into<String>, resource: impl Into<String>) -> Self {
        Self {
            group: String::new(),
            version: version.into(),
            resource: resource.into(),
        }
    }

    /// Construct a named-group GVR.
    pub fn new(
        group: impl Into<String>,
        version: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            resource: resource.into(),
        }
    }

    /// Returns the API version string as used in Kubernetes manifests.
    ///
    /// Core group: `"v1"`, named groups: `"apps/v1"`.
    pub fn api_version(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }
}

impl fmt::Display for Gvr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.group.is_empty() {
            write!(f, "{}/{}", self.version, self.resource)
        } else {
            write!(f, "{}/{}/{}", self.group, self.version, self.resource)
        }
    }
}

/// Well-known GVRs for the resources k7s cares about most.
pub mod well_known {
    use super::Gvr;

    pub fn pods() -> Gvr { Gvr::core("v1", "pods") }
    pub fn nodes() -> Gvr { Gvr::core("v1", "nodes") }
    pub fn namespaces() -> Gvr { Gvr::core("v1", "namespaces") }
    pub fn services() -> Gvr { Gvr::core("v1", "services") }
    pub fn config_maps() -> Gvr { Gvr::core("v1", "configmaps") }
    pub fn secrets() -> Gvr { Gvr::core("v1", "secrets") }
    pub fn events() -> Gvr { Gvr::core("v1", "events") }
    pub fn persistent_volumes() -> Gvr { Gvr::core("v1", "persistentvolumes") }
    pub fn persistent_volume_claims() -> Gvr { Gvr::core("v1", "persistentvolumeclaims") }
    pub fn service_accounts() -> Gvr { Gvr::core("v1", "serviceaccounts") }

    pub fn deployments() -> Gvr { Gvr::new("apps", "v1", "deployments") }
    pub fn stateful_sets() -> Gvr { Gvr::new("apps", "v1", "statefulsets") }
    pub fn daemon_sets() -> Gvr { Gvr::new("apps", "v1", "daemonsets") }
    pub fn replica_sets() -> Gvr { Gvr::new("apps", "v1", "replicasets") }

    pub fn jobs() -> Gvr { Gvr::new("batch", "v1", "jobs") }
    pub fn cron_jobs() -> Gvr { Gvr::new("batch", "v1", "cronjobs") }

    pub fn ingresses() -> Gvr { Gvr::new("networking.k8s.io", "v1", "ingresses") }
    pub fn network_policies() -> Gvr { Gvr::new("networking.k8s.io", "v1", "networkpolicies") }

    pub fn roles() -> Gvr { Gvr::new("rbac.authorization.k8s.io", "v1", "roles") }
    pub fn role_bindings() -> Gvr { Gvr::new("rbac.authorization.k8s.io", "v1", "rolebindings") }
    pub fn cluster_roles() -> Gvr { Gvr::new("rbac.authorization.k8s.io", "v1", "clusterroles") }
    pub fn cluster_role_bindings() -> Gvr {
        Gvr::new("rbac.authorization.k8s.io", "v1", "clusterrolebindings")
    }

    pub fn custom_resource_definitions() -> Gvr {
        Gvr::new("apiextensions.k8s.io", "v1", "customresourcedefinitions")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_gvr_api_version() {
        let gvr = Gvr::core("v1", "pods");
        assert_eq!(gvr.api_version(), "v1");
    }

    #[test]
    fn named_gvr_api_version() {
        let gvr = Gvr::new("apps", "v1", "deployments");
        assert_eq!(gvr.api_version(), "apps/v1");
    }

    #[test]
    fn gvr_display_core() {
        let gvr = Gvr::core("v1", "pods");
        assert_eq!(gvr.to_string(), "v1/pods");
    }

    #[test]
    fn gvr_display_named() {
        let gvr = Gvr::new("apps", "v1", "deployments");
        assert_eq!(gvr.to_string(), "apps/v1/deployments");
    }
}
