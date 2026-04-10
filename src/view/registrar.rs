//! View registrar — maps GVRs to view factories and action sets.
//!
//! The registrar is the single entry point for "given this GVR (or alias),
//! what view should I show and what actions are available?"
//!
//! # k9s Reference
//!
//! Corresponds to `internal/view/registrar.go` — the map of GVR → `ViewFunc`.

use crate::client::Gvr;
use crate::dao::Registry;
use crate::view::actions::{actions_for, ActionId, ResourceAction};
use crate::view::browser::{browser_for_resource, BrowserView};

/// A fully resolved view entry for a resource type.
pub struct ViewEntry {
    /// The browser list view for this resource.
    pub browser: BrowserView,
    /// Available actions on the selected resource, ordered for hints display.
    pub actions: Vec<ResourceAction>,
    /// The GVR this entry is for.
    pub gvr: Gvr,
}

/// Resolve a command-prompt string (alias or resource name) to a view entry.
///
/// Returns `None` if the alias is not recognised.
pub fn view_for(alias: &str, registry: &Registry) -> Option<ViewEntry> {
    let meta = registry.get_by_alias(alias)?;
    let gvr = meta.gvr.clone();
    let browser = browser_for_resource(alias, registry)?;
    let actions = actions_for(&gvr);

    Some(ViewEntry {
        browser,
        actions,
        gvr,
    })
}

impl ViewEntry {
    /// Look up an action by its key string (e.g. `"l"`, `"d"`).
    pub fn action_for_key(&self, key: &str) -> Option<&ResourceAction> {
        self.actions.iter().find(|a| a.key == key)
    }

    /// Returns true if the given action is available for this resource type.
    pub fn supports(&self, id: ActionId) -> bool {
        self.actions.iter().any(|a| a.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dao::Registry;
    use crate::view::actions::ActionId;

    #[test]
    fn resolve_pods_view() {
        let reg = Registry::with_builtins();
        let entry = view_for("po", &reg).expect("po should resolve");
        assert!(entry.supports(ActionId::Logs));
        assert!(entry.supports(ActionId::Shell));
        assert!(entry.supports(ActionId::Describe));
    }

    #[test]
    fn resolve_deployment_view() {
        let reg = Registry::with_builtins();
        let entry = view_for("dp", &reg).expect("dp should resolve");
        assert!(entry.supports(ActionId::Scale));
        assert!(entry.supports(ActionId::Restart));
    }

    #[test]
    fn resolve_cronjob_view() {
        let reg = Registry::with_builtins();
        let entry = view_for("cj", &reg).expect("cj should resolve");
        assert!(entry.supports(ActionId::Trigger));
    }

    #[test]
    fn unknown_alias_returns_none() {
        let reg = Registry::with_builtins();
        assert!(view_for("nonexistent", &reg).is_none());
    }

    #[test]
    fn action_for_key_lookup() {
        let reg = Registry::with_builtins();
        let entry = view_for("po", &reg).unwrap();
        let action = entry
            .action_for_key("l")
            .expect("l key should exist for pods");
        assert_eq!(action.id, ActionId::Logs);
    }
}
