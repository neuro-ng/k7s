use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::Client;

use crate::error::ClientError;

/// Resolved Kubernetes client configuration.
///
/// Wraps a `kube::Client` with metadata about the chosen context and server.
pub struct ClientConfig {
    /// The active kube context name.
    pub context: String,
    /// The API server URL.
    pub server: String,
    /// The ready-to-use Kubernetes client.
    pub client: Client,
}

impl ClientConfig {
    /// Build a client from the active kubeconfig context.
    ///
    /// Uses the kubeconfig at `$KUBECONFIG` or `~/.kube/config`.
    pub async fn from_default_context() -> Result<Self, ClientError> {
        Self::from_context(None).await
    }

    /// Build a client for a specific kubeconfig context.
    ///
    /// Pass `None` to use the active context.
    pub async fn from_context(context: Option<&str>) -> Result<Self, ClientError> {
        let kubeconfig = Kubeconfig::read()?;

        let context_name = context
            .map(str::to_owned)
            .or_else(|| kubeconfig.current_context.clone())
            .unwrap_or_else(|| "default".to_string());

        // Verify the context actually exists.
        let ctx_exists = kubeconfig.contexts.iter().any(|c| c.name == context_name);

        if !ctx_exists {
            return Err(ClientError::ContextNotFound { name: context_name });
        }

        let opts = KubeConfigOptions {
            context: Some(context_name.clone()),
            ..Default::default()
        };

        let config = kube::Config::from_kubeconfig(&opts).await?;
        let server = config.cluster_url.to_string();
        let client = Client::try_from(config)?;

        tracing::info!(context = %context_name, server = %server, "kubernetes client ready");

        Ok(Self {
            context: context_name,
            server,
            client,
        })
    }

    /// Check that the API server is reachable and returns a valid version.
    ///
    /// Returns the server version string (e.g. `"v1.30.2"`) on success.
    pub async fn check_connectivity(&self) -> Result<String, ClientError> {
        let version =
            self.client
                .apiserver_version()
                .await
                .map_err(|e| ClientError::ConnectivityCheck {
                    server: self.server.clone(),
                    source: Box::new(e),
                })?;

        let version_str = format!("v{}.{}", version.major, version.minor);
        tracing::info!(version = %version_str, "API server reachable");
        Ok(version_str)
    }

    /// List all namespace names the caller can see.
    pub async fn namespace_names(&self) -> Result<Vec<String>, ClientError> {
        use k8s_openapi::api::core::v1::Namespace;
        use kube::Api;

        let api: Api<Namespace> = Api::all(self.client.clone());
        let list = api.list(&Default::default()).await?;

        let names = list
            .items
            .into_iter()
            .filter_map(|ns| ns.metadata.name)
            .collect();

        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    // Integration tests for the real client are in tests/client_integration.rs
    // and require a live cluster. Unit tests here cover only pure logic.

    use super::*;

    #[test]
    fn context_not_found_error_message() {
        let err = ClientError::ContextNotFound {
            name: "missing-ctx".to_string(),
        };
        assert!(err.to_string().contains("missing-ctx"));
    }
}
