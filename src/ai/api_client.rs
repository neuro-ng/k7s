//! OpenAI-compatible API key LLM client.
//!
//! Works with any provider exposing an OpenAI-compatible `/v1/chat/completions`
//! endpoint: OpenAI, Anthropic (via shim), Azure OpenAI, local Ollama, etc.

use async_trait::async_trait;
use futures::StreamExt as _;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::provider::{Message, Provider, Role, StreamChunk};

/// Configuration for the API key provider.
#[derive(Debug, Clone)]
pub struct ApiKeyProviderConfig {
    /// Full URL of the completions endpoint.
    pub endpoint: String,
    /// Bearer token / API key.
    pub api_key: String,
    /// Model identifier (e.g. `"gpt-4o"`, `"claude-3-5-sonnet-20241022"`).
    pub model: String,
    /// Maximum tokens to generate in a single response.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0). Lower = more deterministic.
    pub temperature: f32,
}

impl Default for ApiKeyProviderConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://api.openai.com/v1/chat/completions".to_owned(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_owned(),
            max_tokens: 2048,
            temperature: 0.3,
        }
    }
}

/// OpenAI-compatible LLM provider using API key authentication.
pub struct ApiKeyProvider {
    config: ApiKeyProviderConfig,
    client: Client,
}

impl ApiKeyProvider {
    pub fn new(config: ApiKeyProviderConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("failed to build reqwest client");
        Self { config, client }
    }
}

// ─── Wire protocol types ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ApiMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Serialize)]
struct ApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// Non-streaming response.
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

/// SSE streaming response — one event per `data:` line.
#[derive(Deserialize)]
struct SseEvent {
    choices: Vec<SseChoice>,
}

#[derive(Deserialize)]
struct SseChoice {
    delta: SseDelta,
}

#[derive(Deserialize)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}

// ─── Provider impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl Provider for ApiKeyProvider {
    fn name(&self) -> &str {
        "api-key"
    }

    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String> {
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
            model: &self.config.model,
            messages: api_messages,
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
        };

        let response = self
            .client
            .post(&self.config.endpoint)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("LLM API error {status}: {body}");
        }

        let parsed: ChatResponse = response.json().await?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("LLM response contained no choices"))
    }

    async fn stream(
        &self,
        messages: &[Message],
        tx: mpsc::Sender<StreamChunk>,
    ) -> anyhow::Result<()> {
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

        // Build request with `stream: true` injected as an extra field.
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": api_messages,
            "max_tokens": self.config.max_tokens,
            "temperature": self.config.temperature,
            "stream": true,
        });

        let response = self
            .client
            .post(&self.config.endpoint)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("LLM stream request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            let _ = tx
                .send(StreamChunk::Error(format!(
                    "LLM API error {status}: {body_text}"
                )))
                .await;
            return Ok(());
        }

        let mut byte_stream = response.bytes_stream();
        let mut line_buf = String::new();

        while let Some(item) = byte_stream.next().await {
            let bytes = match item {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                    return Ok(());
                }
            };

            line_buf.push_str(&String::from_utf8_lossy(&bytes));

            // OpenAI SSE: each event is prefixed with "data: " on its own line.
            while let Some(newline) = line_buf.find('\n') {
                let line = line_buf[..newline].trim().to_owned();
                line_buf = line_buf[newline + 1..].to_owned();

                let data = match line.strip_prefix("data: ") {
                    Some(d) => d.trim(),
                    None => continue,
                };

                if data == "[DONE]" {
                    let _ = tx.send(StreamChunk::Done).await;
                    return Ok(());
                }

                if let Ok(event) = serde_json::from_str::<SseEvent>(data) {
                    if let Some(content) = event
                        .choices
                        .into_iter()
                        .next()
                        .and_then(|c| c.delta.content)
                        .filter(|c| !c.is_empty())
                    {
                        if tx.send(StreamChunk::Delta(content)).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }

        let _ = tx.send(StreamChunk::Done).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_openai_endpoint() {
        let cfg = ApiKeyProviderConfig::default();
        assert!(cfg.endpoint.contains("openai.com"));
    }

    #[test]
    fn provider_name() {
        let p = ApiKeyProvider::new(ApiKeyProviderConfig::default());
        assert_eq!(p.name(), "api-key");
    }
}
