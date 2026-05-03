//! Azure OpenAI provider.
//!
//! Connects to an Azure OpenAI deployment using the `api-key` header
//! authentication scheme and Azure's deployment-scoped endpoint URL.
//!
//! Azure OpenAI is API-compatible with OpenAI but differs in two key ways:
//! - Auth: `api-key: {key}` header, not `Authorization: Bearer {key}`
//! - URL: `https://{resource}.openai.azure.com/openai/deployments/{deployment}/chat/completions?api-version={version}`
//!
//! # Quick start
//!
//! ```bash
//! export AZURE_OPENAI_API_KEY="..."
//! export AZURE_OPENAI_ENDPOINT="https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions"
//! ```
//!
//! `~/.config/k7s/config.yaml`:
//! ```yaml
//! k7s:
//!   ai:
//!     provider: azure
//!     azureEndpoint: https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions
//!     azureApiVersion: "2024-02-01"   # optional; defaults to this value
//! ```

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::ai::provider::{Message, Provider, Role};

/// Default Azure OpenAI API version.
pub const DEFAULT_API_VERSION: &str = "2024-02-01";

/// Configuration for the Azure OpenAI provider.
#[derive(Debug, Clone)]
pub struct AzureConfig {
    /// Full endpoint URL including the deployment path.
    ///
    /// Example: `https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions`
    pub endpoint: String,
    /// Azure OpenAI API key (the `api-key` header value, not a Bearer token).
    pub api_key: String,
    /// API version query parameter (e.g. `"2024-02-01"`).
    pub api_version: String,
    /// Model name — used only for display and logging; Azure ignores it
    /// (the deployment URL already encodes the model).
    pub model: String,
    /// Maximum tokens to generate in a single response.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0). Lower = more deterministic.
    pub temperature: f32,
}

impl Default for AzureConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            api_key: String::new(),
            api_version: DEFAULT_API_VERSION.to_owned(),
            model: "gpt-4o".to_owned(),
            max_tokens: 2048,
            temperature: 0.3,
        }
    }
}

/// Azure OpenAI provider using `api-key` header authentication.
pub struct AzureOpenAIProvider {
    config: AzureConfig,
    client: Client,
}

impl AzureOpenAIProvider {
    pub fn new(config: AzureConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("failed to build reqwest client");
        Self { config, client }
    }

    /// Build the full request URL with the `api-version` query parameter appended.
    fn request_url(&self) -> String {
        let base = self.config.endpoint.trim_end_matches('?');
        if base.contains('?') {
            format!("{base}&api-version={}", self.config.api_version)
        } else {
            format!("{base}?api-version={}", self.config.api_version)
        }
    }
}

// ─── Wire protocol types ───────────────────────────────────────────────────────
//
// Azure uses the same request/response shape as OpenAI `/v1/chat/completions`.

#[derive(Serialize)]
struct ChatRequest<'a> {
    messages: Vec<ApiMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Serialize)]
struct ApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

// ─── Error helpers ─────────────────────────────────────────────────────────────

fn format_azure_error(status: reqwest::StatusCode, body: &str) -> String {
    match status.as_u16() {
        401 => format!(
            "Azure OpenAI authentication failed (401).\n\
             Check your AZURE_OPENAI_API_KEY or azureApiKey config value.\n\
             Details: {body}"
        ),
        404 => format!(
            "Azure OpenAI deployment not found (404).\n\
             Verify your azureEndpoint URL contains the correct resource name and deployment.\n\
             Details: {body}"
        ),
        429 => format!(
            "Azure OpenAI rate limit exceeded (429).\n\
             Wait a moment and retry, or increase your quota in the Azure portal.\n\
             Details: {body}"
        ),
        _ => format!("Azure OpenAI error {status}: {body}"),
    }
}

// ─── Provider impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl Provider for AzureOpenAIProvider {
    fn name(&self) -> &str {
        "azure"
    }

    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String> {
        if self.config.endpoint.is_empty() {
            anyhow::bail!(
                "Azure OpenAI endpoint is not configured.\n\
                 Set azureEndpoint in config.yaml or AZURE_OPENAI_ENDPOINT env var.\n\
                 Example: https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions"
            );
        }

        let api_messages: Vec<ApiMessage> = messages
            .iter()
            .map(|m| ApiMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                content: &m.content,
            })
            .collect();

        let body = ChatRequest {
            messages: api_messages,
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
        };

        let url = self.request_url();
        let response = self
            .client
            .post(&url)
            .header("api-key", &self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Azure OpenAI request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", format_azure_error(status, &body_text));
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Azure OpenAI response: {e}"))?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("Azure OpenAI response contained no choices"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_api_version() {
        let cfg = AzureConfig::default();
        assert_eq!(cfg.api_version, DEFAULT_API_VERSION);
    }

    #[test]
    fn provider_name() {
        let p = AzureOpenAIProvider::new(AzureConfig::default());
        assert_eq!(p.name(), "azure");
    }

    #[test]
    fn request_url_appends_api_version_query() {
        let cfg = AzureConfig {
            endpoint: "https://my.openai.azure.com/openai/deployments/gpt-4o/chat/completions"
                .to_owned(),
            api_version: "2024-02-01".to_owned(),
            ..Default::default()
        };
        let p = AzureOpenAIProvider::new(cfg);
        let url = p.request_url();
        assert!(url.contains("api-version=2024-02-01"));
        assert!(url.contains("?api-version="), "should use ? separator");
    }

    #[test]
    fn request_url_uses_ampersand_when_query_already_present() {
        let cfg = AzureConfig {
            endpoint: "https://my.openai.azure.com/openai/deployments/gpt-4o/chat/completions?foo=bar".to_owned(),
            api_version: "2024-02-01".to_owned(),
            ..Default::default()
        };
        let p = AzureOpenAIProvider::new(cfg);
        let url = p.request_url();
        assert!(url.contains("&api-version=2024-02-01"));
    }

    #[test]
    fn format_error_401_has_api_key_hint() {
        let msg = format_azure_error(reqwest::StatusCode::UNAUTHORIZED, "invalid key");
        assert!(msg.contains("401"));
        assert!(msg.contains("AZURE_OPENAI_API_KEY"));
    }

    #[test]
    fn format_error_404_has_deployment_hint() {
        let msg = format_azure_error(reqwest::StatusCode::NOT_FOUND, "deployment not found");
        assert!(msg.contains("404"));
        assert!(msg.contains("deployment"));
    }

    #[test]
    fn format_error_429_has_rate_limit_hint() {
        let msg = format_azure_error(reqwest::StatusCode::TOO_MANY_REQUESTS, "rate limited");
        assert!(msg.contains("429"));
        assert!(msg.contains("rate limit") || msg.contains("quota"));
    }

    #[test]
    fn format_error_generic_includes_status() {
        let msg = format_azure_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "server error");
        assert!(msg.contains("500"));
        assert!(msg.contains("server error"));
    }

    #[test]
    fn chat_request_omits_model_field() {
        let req = ChatRequest {
            messages: vec![ApiMessage {
                role: "user",
                content: "hi",
            }],
            max_tokens: 100,
            temperature: 0.3,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("model").is_none(), "Azure request must not include model");
        assert_eq!(json["messages"][0]["role"], "user");
    }

    #[test]
    fn chat_response_deserialises() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"Hello from Azure!"}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.choices[0].message.content, "Hello from Azure!");
    }

    #[test]
    fn default_config_model_and_max_tokens() {
        let cfg = AzureConfig::default();
        assert_eq!(cfg.model, "gpt-4o");
        assert_eq!(cfg.max_tokens, 2048);
    }
}
