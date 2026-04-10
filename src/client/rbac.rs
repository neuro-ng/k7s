use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
};
use kube::api::PostParams;
use kube::{Api, Client};

use crate::error::ClientError;

/// Kubernetes RBAC verb for a resource action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Get,
    List,
    Watch,
    Create,
    Update,
    Patch,
    Delete,
    DeleteCollection,
    Exec,
}

impl Verb {
    fn as_str(self) -> &'static str {
        match self {
            Verb::Get => "get",
            Verb::List => "list",
            Verb::Watch => "watch",
            Verb::Create => "create",
            Verb::Update => "update",
            Verb::Patch => "patch",
            Verb::Delete => "delete",
            Verb::DeleteCollection => "deletecollection",
            Verb::Exec => "create", // exec uses "create" on pods/exec subresource
        }
    }
}

/// Check whether the current user (identity in the kubeconfig) is allowed
/// to perform a verb on a resource.
///
/// Uses `SelfSubjectAccessReview` — the same mechanism `kubectl auth can-i` uses.
pub async fn can_i(
    client: &Client,
    verb: Verb,
    group: &str,
    resource: &str,
    namespace: Option<&str>,
) -> Result<bool, ClientError> {
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());

    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                group: Some(group.to_owned()),
                resource: Some(resource.to_owned()),
                namespace: namespace.map(str::to_owned),
                verb: Some(verb.as_str().to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    let response = api
        .create(&PostParams::default(), &review)
        .await
        .map_err(ClientError::Api)?;

    let allowed = response
        .status
        .map(|s| s.allowed)
        .unwrap_or(false);

    tracing::debug!(
        verb = verb.as_str(),
        resource = resource,
        namespace = ?namespace,
        allowed,
        "RBAC can-i check"
    );

    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verb_str_values() {
        assert_eq!(Verb::List.as_str(), "list");
        assert_eq!(Verb::Delete.as_str(), "delete");
        assert_eq!(Verb::Exec.as_str(), "create");
    }
}
