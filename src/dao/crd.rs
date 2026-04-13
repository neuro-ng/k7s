//! CRD discovery — Phase 2.12.
//!
//! Fetches all `CustomResourceDefinition` objects from the cluster and
//! converts them into [`ResourceMeta`] entries that can be inserted into the
//! [`Registry`].  After this call, users can navigate to any CRD group via
//! its resource name (e.g. `:foos` for `foos.example.com`).
//!
//! # Design
//!
//! - CRDs are discovered at startup and refreshed on demand.
//! - Each CRD's first served version is used to build the GVR.
//! - The short names from `spec.names.shortNames` become aliases.
//!
//! # k9s Reference
//! `internal/dao/registry.go` → `loadCRDs()`

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};

use crate::client::Gvr;
use crate::dao::traits::ResourceMeta;
use crate::error::DaoError;

/// A discovered CRD converted into k7s metadata.
#[derive(Debug, Clone)]
pub struct CrdMeta {
    /// The GVR using the first served version.
    pub gvr: Gvr,
    /// Display name derived from `spec.names.kind`.
    pub display_name: String,
    /// Short names from `spec.names.shortNames`.
    pub aliases: Vec<String>,
    /// Whether this CRD is namespace-scoped.
    pub namespaced: bool,
}

impl CrdMeta {
    /// Convert into a [`ResourceMeta`] for insertion into the [`Registry`].
    pub fn into_resource_meta(self) -> ResourceMeta {
        let alias_refs: Vec<&str> = self.aliases.iter().map(|s| s.as_str()).collect();
        ResourceMeta::new(self.gvr, self.display_name, alias_refs, self.namespaced)
    }
}

/// Fetch all CRDs from the cluster and return them as [`CrdMeta`] entries.
///
/// Skips any CRD that has no served versions (degenerate/invalid CRDs).
///
/// # Errors
///
/// Returns [`DaoError::Api`] if the API call fails (e.g. no access to CRDs).
pub async fn discover_crds(client: &Client) -> Result<Vec<CrdMeta>, DaoError> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let list = api.list(&Default::default()).await?;

    let metas = list.items.into_iter().filter_map(crd_to_meta).collect();

    Ok(metas)
}

/// Convert a single CRD object into a [`CrdMeta`], or `None` for invalid CRDs.
fn crd_to_meta(crd: CustomResourceDefinition) -> Option<CrdMeta> {
    let spec = crd.spec;

    // Find the first served+storage version.
    let version = spec
        .versions
        .iter()
        .find(|v| v.served && v.storage)
        .or_else(|| spec.versions.first())?;

    let group = spec.group.clone();
    let version_name = version.name.clone();
    let resource = spec.names.plural.clone();

    let gvr = Gvr::new(group, version_name, resource);

    let display_name = spec.names.kind.clone();
    let namespaced = spec.scope == "Namespaced";

    // Short names become aliases for the command prompt.
    let mut aliases: Vec<String> = spec.names.short_names.clone().unwrap_or_default();

    // Also add the plural resource name as an alias.
    aliases.push(spec.names.plural.clone());
    aliases.dedup();

    Some(CrdMeta {
        gvr,
        display_name,
        aliases,
        namespaced,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
        CustomResourceDefinitionNames, CustomResourceDefinitionSpec,
        CustomResourceDefinitionVersion,
    };

    fn make_crd(
        plural: &str,
        kind: &str,
        group: &str,
        scope: &str,
        short_names: Option<Vec<String>>,
    ) -> CustomResourceDefinition {
        let version = CustomResourceDefinitionVersion {
            name: "v1alpha1".to_owned(),
            served: true,
            storage: true,
            ..Default::default()
        };
        let spec = CustomResourceDefinitionSpec {
            group: group.to_owned(),
            names: CustomResourceDefinitionNames {
                plural: plural.to_owned(),
                singular: Some(plural.trim_end_matches('s').to_owned()),
                kind: kind.to_owned(),
                short_names,
                ..Default::default()
            },
            scope: scope.to_owned(),
            versions: vec![version],
            ..Default::default()
        };
        CustomResourceDefinition {
            metadata: kube::core::ObjectMeta {
                name: Some(format!("{plural}.{group}")),
                ..Default::default()
            },
            spec,
            status: None,
        }
    }

    #[test]
    fn crd_to_meta_namespaced() {
        let crd = make_crd(
            "foos",
            "Foo",
            "example.com",
            "Namespaced",
            Some(vec!["fo".into()]),
        );
        let meta = crd_to_meta(crd).expect("should convert");
        assert_eq!(meta.gvr.group, "example.com");
        assert_eq!(meta.gvr.version, "v1alpha1");
        assert_eq!(meta.gvr.resource, "foos");
        assert_eq!(meta.display_name, "Foo");
        assert!(meta.namespaced);
        assert!(meta.aliases.contains(&"fo".to_owned()));
        assert!(meta.aliases.contains(&"foos".to_owned()));
    }

    #[test]
    fn crd_to_meta_cluster_scoped() {
        let crd = make_crd("bars", "Bar", "acme.io", "Cluster", None);
        let meta = crd_to_meta(crd).expect("should convert");
        assert!(!meta.namespaced);
        assert_eq!(meta.gvr.group, "acme.io");
    }

    #[test]
    fn crd_into_resource_meta_round_trip() {
        let crd = make_crd(
            "widgets",
            "Widget",
            "store.io",
            "Namespaced",
            Some(vec!["wi".into()]),
        );
        let meta = crd_to_meta(crd).unwrap().into_resource_meta();
        assert_eq!(meta.gvr.resource, "widgets");
        assert_eq!(meta.display_name, "Widget");
    }
}
