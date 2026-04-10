//! Renderers for RBAC resources.
//!
//! Four resource types share this module:
//! - Roles (`rbac.authorization.k8s.io/v1/roles`)
//! - RoleBindings (`rbac.authorization.k8s.io/v1/rolebindings`)
//! - ClusterRoles (`rbac.authorization.k8s.io/v1/clusterroles`)
//! - ClusterRoleBindings (`rbac.authorization.k8s.io/v1/clusterrolebindings`)
//!
//! # Security
//!
//! RBAC resources contain no secret material — only structural policy data
//! (verb/resource/apiGroup lists and subject names).  All fields are safe to
//! display.

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, meta_namespace, ColumnDef, RenderedRow, Renderer};

// ─── Role ─────────────────────────────────────────────────────────────────────

pub struct RoleRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl RoleRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::roles(),
            columns: vec![
                ColumnDef::new("NAMESPACE", Constraint::Length(18)),
                ColumnDef::new("NAME", Constraint::Min(28)),
                ColumnDef::new("RULES", Constraint::Length(6)),
                ColumnDef::new("VERBS", Constraint::Min(30)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for RoleRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for RoleRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let namespace = meta_namespace(obj).to_owned();
        let name = meta_name(obj).to_owned();
        let (rule_count, verbs) = summarise_rules(obj);
        let (age, age_secs) = age_from_obj(obj);
        RenderedRow {
            cells: vec![namespace, name, rule_count.to_string(), verbs, age],
            age_secs,
        }
    }
}

// ─── ClusterRole ──────────────────────────────────────────────────────────────

pub struct ClusterRoleRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl ClusterRoleRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::cluster_roles(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(35)),
                ColumnDef::new("RULES", Constraint::Length(6)),
                ColumnDef::new("VERBS", Constraint::Min(30)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for ClusterRoleRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ClusterRoleRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let (rule_count, verbs) = summarise_rules(obj);
        let (age, age_secs) = age_from_obj(obj);
        RenderedRow {
            cells: vec![name, rule_count.to_string(), verbs, age],
            age_secs,
        }
    }
}

// ─── RoleBinding ──────────────────────────────────────────────────────────────

pub struct RoleBindingRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl RoleBindingRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::role_bindings(),
            columns: vec![
                ColumnDef::new("NAMESPACE", Constraint::Length(18)),
                ColumnDef::new("NAME", Constraint::Min(28)),
                ColumnDef::new("ROLE", Constraint::Min(28)),
                ColumnDef::new("SUBJECTS", Constraint::Length(6)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for RoleBindingRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for RoleBindingRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let namespace = meta_namespace(obj).to_owned();
        let name = meta_name(obj).to_owned();
        let role_ref = role_ref_display(obj);
        let subject_count = count_subjects(obj);
        let (age, age_secs) = age_from_obj(obj);
        RenderedRow {
            cells: vec![namespace, name, role_ref, subject_count.to_string(), age],
            age_secs,
        }
    }
}

// ─── ClusterRoleBinding ───────────────────────────────────────────────────────

pub struct ClusterRoleBindingRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl ClusterRoleBindingRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::cluster_role_bindings(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(35)),
                ColumnDef::new("ROLE", Constraint::Min(28)),
                ColumnDef::new("SUBJECTS", Constraint::Length(8)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for ClusterRoleBindingRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ClusterRoleBindingRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let role_ref = role_ref_display(obj);
        let subject_count = count_subjects(obj);
        let (age, age_secs) = age_from_obj(obj);
        RenderedRow {
            cells: vec![name, role_ref, subject_count.to_string(), age],
            age_secs,
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Count the number of policy rules and collect unique verbs across all rules.
fn summarise_rules(obj: &Value) -> (usize, String) {
    let rules = match obj.get("rules").and_then(|v| v.as_array()) {
        Some(r) => r,
        None => return (0, String::new()),
    };

    let mut all_verbs: Vec<&str> = Vec::new();
    for rule in rules {
        if let Some(verbs) = rule.get("verbs").and_then(|v| v.as_array()) {
            for v in verbs {
                if let Some(s) = v.as_str() {
                    if !all_verbs.contains(&s) {
                        all_verbs.push(s);
                    }
                }
            }
        }
    }

    // Put wildcard first, then sort the rest.
    all_verbs.sort_by(|a, b| {
        if *a == "*" {
            std::cmp::Ordering::Less
        } else if *b == "*" {
            std::cmp::Ordering::Greater
        } else {
            a.cmp(b)
        }
    });

    (rules.len(), all_verbs.join(","))
}

/// Format the roleRef as `kind/name`.
fn role_ref_display(obj: &Value) -> String {
    let kind = obj
        .pointer("/roleRef/kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let name = obj
        .pointer("/roleRef/name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if kind.is_empty() {
        name.to_owned()
    } else {
        format!("{kind}/{name}")
    }
}

/// Count subjects in a RoleBinding / ClusterRoleBinding.
fn count_subjects(obj: &Value) -> usize {
    obj.get("subjects")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_role_counts_rules() {
        let obj = json!({
            "metadata": { "name": "read-pods", "namespace": "default" },
            "rules": [
                { "verbs": ["get", "list", "watch"], "resources": ["pods"] },
                { "verbs": ["get"], "resources": ["services"] }
            ]
        });
        let r = RoleRenderer::new().render(&obj);
        assert_eq!(r.cells[1], "read-pods");
        assert_eq!(r.cells[2], "2");
        assert!(r.cells[3].contains("get"), "verbs must appear");
        assert!(r.cells[3].contains("list"), "verbs must appear");
    }

    #[test]
    fn render_cluster_role_wildcard() {
        let obj = json!({
            "metadata": { "name": "admin" },
            "rules": [{ "verbs": ["*"], "resources": ["*"] }]
        });
        let r = ClusterRoleRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "admin");
        assert_eq!(r.cells[1], "1");
        assert_eq!(r.cells[2], "*");
    }

    #[test]
    fn render_role_binding_shows_role_ref() {
        let obj = json!({
            "metadata": { "name": "rb", "namespace": "ns" },
            "roleRef": { "kind": "Role", "name": "read-pods" },
            "subjects": [
                { "kind": "ServiceAccount", "name": "default" },
                { "kind": "User", "name": "alice" }
            ]
        });
        let r = RoleBindingRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "Role/read-pods");
        assert_eq!(r.cells[3], "2");
    }

    #[test]
    fn render_cluster_role_binding() {
        let obj = json!({
            "metadata": { "name": "crb" },
            "roleRef": { "kind": "ClusterRole", "name": "cluster-admin" },
            "subjects": [{ "kind": "User", "name": "ops" }]
        });
        let r = ClusterRoleBindingRenderer::new().render(&obj);
        assert_eq!(r.cells[1], "ClusterRole/cluster-admin");
        assert_eq!(r.cells[2], "1");
    }
}
