use std::collections::HashMap;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::traits::ResourceMeta;

/// Central registry mapping GVRs (and aliases) to resource metadata.
///
/// The registry is the single source of truth for "what resource types does
/// k7s know about?" It is populated at startup with the built-in types and
/// extended at runtime when CRDs are discovered.
pub struct Registry {
    /// Primary index: GVR → metadata.
    by_gvr: HashMap<Gvr, ResourceMeta>,
    /// Secondary index: lowercase alias → GVR for fast command-prompt lookup.
    by_alias: HashMap<String, Gvr>,
}

impl Registry {
    /// Build the registry pre-populated with all built-in resource types.
    pub fn with_builtins() -> Self {
        let mut reg = Self {
            by_gvr: HashMap::new(),
            by_alias: HashMap::new(),
        };
        reg.register_builtins();
        reg
    }

    /// Register a resource type.
    ///
    /// Overwrites any existing entry with the same GVR (used for CRD updates).
    pub fn register(&mut self, meta: ResourceMeta) {
        for alias in &meta.aliases {
            self.by_alias.insert(alias.clone(), meta.gvr.clone());
        }
        // Also register the resource name itself as an alias.
        self.by_alias
            .insert(meta.gvr.resource.clone(), meta.gvr.clone());
        self.by_gvr.insert(meta.gvr.clone(), meta);
    }

    pub fn get_by_gvr(&self, gvr: &Gvr) -> Option<&ResourceMeta> {
        self.by_gvr.get(gvr)
    }

    /// Resolve a command-prompt string (alias or resource name) to metadata.
    pub fn get_by_alias(&self, alias: &str) -> Option<&ResourceMeta> {
        let gvr = self.by_alias.get(&alias.to_lowercase())?;
        self.by_gvr.get(gvr)
    }

    /// All registered resource types sorted by display name.
    pub fn all_sorted(&self) -> Vec<&ResourceMeta> {
        let mut metas: Vec<_> = self.by_gvr.values().collect();
        metas.sort_by_key(|m| &m.display_name);
        metas
    }

    fn register_builtins(&mut self) {
        // Core group
        self.register(ResourceMeta::new(
            well_known::pods(),
            "Pods",
            vec!["po", "pod"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::nodes(),
            "Nodes",
            vec!["no", "node"],
            false,
        ));
        self.register(ResourceMeta::new(
            well_known::namespaces(),
            "Namespaces",
            vec!["ns", "namespace"],
            false,
        ));
        self.register(ResourceMeta::new(
            well_known::services(),
            "Services",
            vec!["svc", "service"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::config_maps(),
            "ConfigMaps",
            vec!["cm", "configmap"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::secrets(),
            "Secrets",
            vec!["secret"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::events(),
            "Events",
            vec!["ev", "event"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::service_accounts(),
            "ServiceAccounts",
            vec!["sa", "serviceaccount"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::persistent_volumes(),
            "PersistentVolumes",
            vec!["pv"],
            false,
        ));
        self.register(ResourceMeta::new(
            well_known::persistent_volume_claims(),
            "PersistentVolumeClaims",
            vec!["pvc"],
            true,
        ));

        // Apps group
        self.register(ResourceMeta::new(
            well_known::deployments(),
            "Deployments",
            vec!["dp", "deploy", "deployment"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::stateful_sets(),
            "StatefulSets",
            vec!["sts", "statefulset"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::daemon_sets(),
            "DaemonSets",
            vec!["ds", "daemonset"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::replica_sets(),
            "ReplicaSets",
            vec!["rs", "replicaset"],
            true,
        ));

        // Batch group
        self.register(ResourceMeta::new(
            well_known::jobs(),
            "Jobs",
            vec!["job"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::cron_jobs(),
            "CronJobs",
            vec!["cj", "cronjob"],
            true,
        ));

        // Networking
        self.register(ResourceMeta::new(
            well_known::ingresses(),
            "Ingresses",
            vec!["ing", "ingress"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::network_policies(),
            "NetworkPolicies",
            vec!["netpol", "networkpolicy"],
            true,
        ));

        // RBAC
        self.register(ResourceMeta::new(
            well_known::roles(),
            "Roles",
            vec!["role"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::role_bindings(),
            "RoleBindings",
            vec!["rb", "rolebinding"],
            true,
        ));
        self.register(ResourceMeta::new(
            well_known::cluster_roles(),
            "ClusterRoles",
            vec!["cr", "clusterrole"],
            false,
        ));
        self.register(ResourceMeta::new(
            well_known::cluster_role_bindings(),
            "ClusterRoleBindings",
            vec!["crb", "clusterrolebinding"],
            false,
        ));

        // Extensions
        self.register(ResourceMeta::new(
            well_known::custom_resource_definitions(),
            "CRDs",
            vec!["crd"],
            false,
        ));
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_pod_by_alias() {
        let reg = Registry::with_builtins();
        let meta = reg.get_by_alias("po").expect("po alias should resolve");
        assert_eq!(meta.display_name, "Pods");
    }

    #[test]
    fn resolve_deployment_by_resource_name() {
        let reg = Registry::with_builtins();
        let meta = reg
            .get_by_alias("deployments")
            .expect("deployments should resolve");
        assert_eq!(meta.display_name, "Deployments");
    }

    #[test]
    fn all_sorted_is_alphabetical() {
        let reg = Registry::with_builtins();
        let names: Vec<_> = reg.all_sorted().iter().map(|m| &m.display_name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn unknown_alias_returns_none() {
        let reg = Registry::with_builtins();
        assert!(reg.get_by_alias("nonexistent-resource").is_none());
    }
}
