use thiserror::Error;

/// Top-level application errors.
///
/// Each variant represents a distinct failure domain.
/// Downstream crates use their own error types; this is the boundary
/// type for errors that reach `main`.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("kubernetes client error: {0}")]
    Client(#[from] ClientError),

    #[error("terminal I/O error: {0}")]
    Terminal(#[from] std::io::Error),

    #[error("sanitizer error: {0}")]
    Sanitizer(#[from] SanitizeError),
}

/// Errors originating from configuration loading and validation.
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("reading config file {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("parsing config file {path}: {source}")]
    Parse {
        path: std::path::PathBuf,
        source: serde_yaml::Error,
    },

    #[error("XDG config directory unavailable")]
    NoConfigDir,
}

/// Errors originating from the Kubernetes client layer.
#[derive(Error, Debug)]
pub enum ClientError {
    #[error("loading kubeconfig: {0}")]
    KubeConfig(#[from] kube::config::KubeconfigError),

    #[error("kubernetes API error: {0}")]
    Api(#[from] kube::Error),

    #[error("no context named '{name}' found in kubeconfig")]
    ContextNotFound { name: String },

    #[error("connectivity check failed for {server}: {source}")]
    ConnectivityCheck {
        server: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("RBAC check failed: {0}")]
    Rbac(String),
}

/// Errors originating from the data sanitizer pipeline.
///
/// These are security-critical; every error must be logged for audit.
#[derive(Error, Debug)]
pub enum SanitizeError {
    #[error("sanitizing {gvr} resource {ns}/{name}: {source}")]
    Resource {
        gvr: String,
        ns: String,
        name: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("invalid regex pattern '{pattern}': {source}")]
    InvalidPattern {
        pattern: String,
        source: regex::Error,
    },

    #[error("JSON serialization during sanitization: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_error_display_includes_path() {
        let err = ConfigError::Read {
            path: std::path::PathBuf::from("/etc/k7s/config.yaml"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        };
        let msg = err.to_string();
        assert!(msg.contains("/etc/k7s/config.yaml"));
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn sanitize_error_display_includes_resource_coordinates() {
        let err = SanitizeError::Resource {
            gvr: "v1/pods".to_string(),
            ns: "default".to_string(),
            name: "my-pod".to_string(),
            source: "unexpected field".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("v1/pods"));
        assert!(msg.contains("default/my-pod"));
    }
}
