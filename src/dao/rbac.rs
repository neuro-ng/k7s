//! RBAC Data Access Objects — Phase 3.14.
//!
//! Provides list + describe operations for the four core RBAC resource types:
//! - `roles` — namespaced policy rules
//! - `rolebindings` — binds a role to subjects within a namespace
//! - `clusterroles` — cluster-scoped policy rules
//! - `clusterrolebindings` — binds a cluster role to subjects cluster-wide
//!
//! # Extra capability: `policies_for_subject`
//!
//! Given a subject name (ServiceAccount, User, or Group) this DAO can flatten
//! all the rules that apply to that subject across every bound role — useful
//! for permission auditing.
//!
//! # Security
//!
//! RBAC data contains no secret material. Only structural policy information
//! (verb/resource/apiGroup lists and subject references) is exposed. This
//! module is safe to pipe through the sanitizer "as metadata".
//!
//! # k9s Reference: `internal/dao/rbac.go`, `internal/dao/rbac_policy.go`

use async_trait::async_trait;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use kube::{Api, Client};
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::generic::GenericDao;
use crate::dao::traits::{Accessor, DeleteOptions, Describer, Nuker, Resource};

// ─── Subject identity ─────────────────────────────────────────────────────────

/// The kind of RBAC subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectKind {
    ServiceAccount,
    User,
    Group,
}

/// A fully-qualified RBAC subject (kind + optional namespace + name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    pub kind: SubjectKind,
    /// Namespace — only set for `ServiceAccount`.
    pub namespace: Option<String>,
    pub name: String,
}

impl Subject {
    pub fn service_account(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: SubjectKind::ServiceAccount,
            namespace: Some(namespace.into()),
            name: name.into(),
        }
    }

    pub fn user(name: impl Into<String>) -> Self {
        Self {
            kind: SubjectKind::User,
            namespace: None,
            name: name.into(),
        }
    }

    pub fn group(name: impl Into<String>) -> Self {
        Self {
            kind: SubjectKind::Group,
            namespace: None,
            name: name.into(),
        }
    }
}

// ─── Flattened policy rule ────────────────────────────────────────────────────

/// A single flattened RBAC rule attributed to a binding.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    /// The role that originated this rule.
    pub from_role: String,
    /// Whether this came from a ClusterRole (vs a namespaced Role).
    pub cluster_scoped: bool,
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
}

impl PolicyRule {
    /// One-line summary, e.g. `"pods get,list,watch"`.
    pub fn summary(&self) -> String {
        format!("{} → {}", self.resources.join(","), self.verbs.join(","))
    }
}

// ─── RoleDao ──────────────────────────────────────────────────────────────────

/// DAO for `rbac.authorization.k8s.io/v1/roles`.
pub struct RoleDao {
    inner: GenericDao,
}

impl RoleDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::roles()),
        }
    }

    /// List all rules in a specific role.
    pub async fn rules(
        &self,
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<Vec<PolicyRule>> {
        let api: Api<Role> = Api::namespaced(client.clone(), namespace);
        let role = api.get(name).await?;
        Ok(extract_role_rules(
            &name.to_owned(),
            false,
            role.rules.as_deref(),
        ))
    }
}

impl Default for RoleDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for RoleDao {
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
impl Nuker for RoleDao {
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
impl Describer for RoleDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        self.inner.describe(client, namespace, name).await
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

// ─── RoleBindingDao ───────────────────────────────────────────────────────────

/// DAO for `rbac.authorization.k8s.io/v1/rolebindings`.
pub struct RoleBindingDao {
    inner: GenericDao,
}

impl RoleBindingDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::role_bindings()),
        }
    }
}

impl Default for RoleBindingDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for RoleBindingDao {
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
impl Nuker for RoleBindingDao {
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
impl Describer for RoleBindingDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        self.inner.describe(client, namespace, name).await
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

// ─── ClusterRoleDao ───────────────────────────────────────────────────────────

/// DAO for `rbac.authorization.k8s.io/v1/clusterroles`.
pub struct ClusterRoleDao {
    inner: GenericDao,
}

impl ClusterRoleDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::cluster_roles()),
        }
    }

    /// List all rules defined in a ClusterRole.
    pub async fn rules(&self, client: &Client, name: &str) -> anyhow::Result<Vec<PolicyRule>> {
        let api: Api<ClusterRole> = Api::all(client.clone());
        let role = api.get(name).await?;
        Ok(extract_role_rules(
            &name.to_owned(),
            true,
            role.rules.as_deref(),
        ))
    }
}

impl Default for ClusterRoleDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for ClusterRoleDao {
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
impl Nuker for ClusterRoleDao {
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
impl Describer for ClusterRoleDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        self.inner.describe(client, namespace, name).await
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

// ─── ClusterRoleBindingDao ────────────────────────────────────────────────────

/// DAO for `rbac.authorization.k8s.io/v1/clusterrolebindings`.
pub struct ClusterRoleBindingDao {
    inner: GenericDao,
}

impl ClusterRoleBindingDao {
    pub fn new() -> Self {
        Self {
            inner: GenericDao::new(well_known::cluster_role_bindings()),
        }
    }
}

impl Default for ClusterRoleBindingDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Accessor for ClusterRoleBindingDao {
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
impl Nuker for ClusterRoleBindingDao {
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
impl Describer for ClusterRoleBindingDao {
    async fn describe(
        &self,
        client: &Client,
        namespace: Option<&str>,
        name: &str,
    ) -> anyhow::Result<String> {
        self.inner.describe(client, namespace, name).await
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

// ─── Subject-centric policy aggregation ──────────────────────────────────────

/// All RBAC rules that apply to a given subject, gathered from all bindings
/// in a namespace (and optionally cluster-wide bindings).
///
/// This is equivalent to `kubectl auth can-i --list --as=<subject>`.
///
/// # k9s Reference: `internal/dao/rbac_policy.go`
pub async fn policies_for_subject(
    client: &Client,
    subject: &Subject,
    namespace: Option<&str>,
) -> anyhow::Result<Vec<PolicyRule>> {
    let mut rules = Vec::new();

    // ── Namespaced RoleBindings ───────────────────────────────────────────────
    if let Some(ns) = namespace.or(subject.namespace.as_deref()) {
        let rb_api: Api<RoleBinding> = Api::namespaced(client.clone(), ns);
        let bindings = rb_api.list(&Default::default()).await?;

        for binding in bindings.items {
            if !binding_references_subject(&binding.subjects, subject) {
                continue;
            }
            let role_ref = &binding.role_ref;
            match role_ref.kind.as_str() {
                "Role" => {
                    let role_api: Api<Role> = Api::namespaced(client.clone(), ns);
                    if let Ok(role) = role_api.get(&role_ref.name).await {
                        rules.extend(extract_role_rules(
                            &role_ref.name,
                            false,
                            role.rules.as_deref(),
                        ));
                    }
                }
                "ClusterRole" => {
                    let cr_api: Api<ClusterRole> = Api::all(client.clone());
                    if let Ok(role) = cr_api.get(&role_ref.name).await {
                        rules.extend(extract_role_rules(
                            &role_ref.name,
                            true,
                            role.rules.as_deref(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    // ── ClusterRoleBindings ───────────────────────────────────────────────────
    let crb_api: Api<ClusterRoleBinding> = Api::all(client.clone());
    let crb_list = crb_api.list(&Default::default()).await?;

    for binding in crb_list.items {
        if !crb_binding_references_subject(&binding.subjects, subject) {
            continue;
        }
        let role_ref = &binding.role_ref;
        if role_ref.kind == "ClusterRole" {
            let cr_api: Api<ClusterRole> = Api::all(client.clone());
            if let Ok(role) = cr_api.get(&role_ref.name).await {
                rules.extend(extract_role_rules(
                    &role_ref.name,
                    true,
                    role.rules.as_deref(),
                ));
            }
        }
    }

    Ok(rules)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn extract_role_rules(
    role_name: &str,
    cluster_scoped: bool,
    rules: Option<&[k8s_openapi::api::rbac::v1::PolicyRule]>,
) -> Vec<PolicyRule> {
    let Some(rules) = rules else {
        return Vec::new();
    };
    rules
        .iter()
        .map(|r| PolicyRule {
            from_role: role_name.to_owned(),
            cluster_scoped,
            api_groups: r.api_groups.clone().unwrap_or_default(),
            resources: r.resources.clone().unwrap_or_default(),
            verbs: r.verbs.clone(),
        })
        .collect()
}

fn binding_references_subject(
    subjects: &Option<Vec<k8s_openapi::api::rbac::v1::Subject>>,
    target: &Subject,
) -> bool {
    let Some(subjects) = subjects else {
        return false;
    };
    subjects.iter().any(|s| subject_matches(s, target))
}

fn crb_binding_references_subject(
    subjects: &Option<Vec<k8s_openapi::api::rbac::v1::Subject>>,
    target: &Subject,
) -> bool {
    binding_references_subject(subjects, target)
}

fn subject_matches(s: &k8s_openapi::api::rbac::v1::Subject, target: &Subject) -> bool {
    let kind_matches = match target.kind {
        SubjectKind::ServiceAccount => s.kind == "ServiceAccount",
        SubjectKind::User => s.kind == "User",
        SubjectKind::Group => s.kind == "Group",
    };
    if !kind_matches || s.name != target.name {
        return false;
    }
    if target.kind == SubjectKind::ServiceAccount {
        // namespace must also match for ServiceAccounts
        s.namespace.as_deref() == target.namespace.as_deref()
    } else {
        true
    }
}

/// Produce a flat JSON-compatible summary of RBAC policies for the AI sanitizer.
///
/// Includes only structural data — no secret material.
pub fn policies_to_value(rules: &[PolicyRule]) -> Value {
    let items: Vec<Value> = rules
        .iter()
        .map(|r| {
            serde_json::json!({
                "role": r.from_role,
                "cluster": r.cluster_scoped,
                "apiGroups": r.api_groups,
                "resources": r.resources,
                "verbs": r.verbs,
            })
        })
        .collect();
    Value::Array(items)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rules_from_none_returns_empty() {
        let rules = extract_role_rules("my-role", false, None);
        assert!(rules.is_empty());
    }

    #[test]
    fn extract_rules_from_empty_slice() {
        let rules = extract_role_rules("my-role", false, Some(&[]));
        assert!(rules.is_empty());
    }

    #[test]
    fn extract_rules_single_rule() {
        let rule = k8s_openapi::api::rbac::v1::PolicyRule {
            api_groups: Some(vec!["".to_owned()]),
            resources: Some(vec!["pods".to_owned()]),
            verbs: vec!["get".to_owned(), "list".to_owned()],
            ..Default::default()
        };
        let rules = extract_role_rules("read-pods", false, Some(&[rule]));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].from_role, "read-pods");
        assert_eq!(rules[0].resources, vec!["pods"]);
        assert_eq!(rules[0].verbs, vec!["get", "list"]);
        assert!(!rules[0].cluster_scoped);
    }

    #[test]
    fn policy_rule_summary() {
        let rule = PolicyRule {
            from_role: "reader".to_owned(),
            cluster_scoped: false,
            api_groups: vec!["".to_owned()],
            resources: vec!["pods".to_owned(), "services".to_owned()],
            verbs: vec!["get".to_owned(), "list".to_owned()],
        };
        let s = rule.summary();
        assert!(s.contains("pods,services"));
        assert!(s.contains("get,list"));
    }

    #[test]
    fn policies_to_value_produces_array() {
        let rules = vec![PolicyRule {
            from_role: "r".to_owned(),
            cluster_scoped: true,
            api_groups: vec!["apps".to_owned()],
            resources: vec!["deployments".to_owned()],
            verbs: vec!["*".to_owned()],
        }];
        let v = policies_to_value(&rules);
        assert!(v.is_array());
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["role"], "r");
        assert_eq!(arr[0]["cluster"], true);
    }

    #[test]
    fn subject_service_account_constructor() {
        let s = Subject::service_account("default", "my-sa");
        assert_eq!(s.kind, SubjectKind::ServiceAccount);
        assert_eq!(s.namespace, Some("default".to_owned()));
        assert_eq!(s.name, "my-sa");
    }

    #[test]
    fn subject_matching_user() {
        let target = Subject::user("alice");
        let k8s_subject = k8s_openapi::api::rbac::v1::Subject {
            kind: "User".to_owned(),
            name: "alice".to_owned(),
            ..Default::default()
        };
        assert!(subject_matches(&k8s_subject, &target));
    }

    #[test]
    fn subject_mismatch_different_name() {
        let target = Subject::user("alice");
        let k8s_subject = k8s_openapi::api::rbac::v1::Subject {
            kind: "User".to_owned(),
            name: "bob".to_owned(),
            ..Default::default()
        };
        assert!(!subject_matches(&k8s_subject, &target));
    }

    #[test]
    fn rbac_dao_default_gvr() {
        let dao = RoleDao::new();
        assert_eq!(dao.gvr(), &well_known::roles());

        let dao = RoleBindingDao::new();
        assert_eq!(dao.gvr(), &well_known::role_bindings());

        let dao = ClusterRoleDao::new();
        assert_eq!(dao.gvr(), &well_known::cluster_roles());

        let dao = ClusterRoleBindingDao::new();
        assert_eq!(dao.gvr(), &well_known::cluster_role_bindings());
    }
}
