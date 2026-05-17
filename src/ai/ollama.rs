//! Ollama local LLM provider.
//!
//! Connects to a locally-running [Ollama](https://ollama.com) instance using the
//! native `/api/chat` endpoint (no auth required).  The default host is
//! `http://localhost:11434`, which can be overridden via `OLLAMA_HOST` or the
//! `ollamaHost` config key.
//!
//! # Quick start
//!
//! ```bash
//! ollama pull llama3.2
//! ollama serve          # or: systemctl start ollama
//! ```
//!
//! `~/.config/k7s/config.yaml`:
//! ```yaml
//! k7s:
//!   ai:
//!     provider: ollama
//!     ollamaModel: llama3.2   # optional; defaults to this value
//!     ollamaHost: http://localhost:11434  # optional
//! ```

use async_trait::async_trait;
use futures::StreamExt as _;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::provider::{Message, Provider, Role, StreamChunk};

/// Default Ollama host when neither `OLLAMA_HOST` nor `ollamaHost` config is set.
pub const DEFAULT_HOST: &str = "http://localhost:11434";
/// Default model to use when `ollamaModel` is not configured.
pub const DEFAULT_MODEL: &str = "llama3.2";

/// Configuration for the Ollama provider.
#[derive(Debug, Clone)]
pub struct OllamaConfig {
    /// Base URL of the Ollama server (e.g. `http://localhost:11434`).
    pub host: String,
    /// Model to use (e.g. `"llama3.2"`, `"mistral"`, `"codellama"`).
    pub model: String,
    /// Maximum tokens to generate in a single response.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0). Lower = more deterministic.
    pub temperature: f32,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            max_tokens: 2048,
            temperature: 0.3,
        }
    }
}

/// LLM provider backed by a local Ollama instance.
pub struct OllamaProvider {
    config: OllamaConfig,
    client: Client,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("failed to build reqwest client");
        Self { config, client }
    }

    /// Fetch the list of locally available model names for error hints.
    async fn available_models(&self) -> Vec<String> {
        let url = format!("{}/api/tags", self.config.host);
        let Ok(resp) = self.client.get(&url).send().await else {
            return vec![];
        };
        let Ok(tags) = resp.json::<TagsResponse>().await else {
            return vec![];
        };
        tags.models.into_iter().map(|m| m.name).collect()
    }
}

// ─── Wire protocol types ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage<'a>>,
    stream: bool,
    options: InferenceOptions,
}

#[derive(Serialize)]
struct OllamaMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct InferenceOptions {
    num_predict: u32,
    temperature: f32,
}

/// Used for the non-streaming (`stream: false`) response.
#[derive(Deserialize)]
struct ChatResponse {
    message: OllamaResponseMessage,
}

/// Used for each newline-delimited chunk in the streaming (`stream: true`) response.
#[derive(Deserialize)]
struct StreamingChunk {
    message: OllamaResponseMessage,
    done: bool,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    name: String,
}

// ─── Error response ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct OllamaError {
    error: String,
}

fn format_ollama_error(status: reqwest::StatusCode, body: &str, host: &str) -> String {
    // Try to parse a structured error first.
    let msg = serde_json::from_str::<OllamaError>(body)
        .map(|e| e.error)
        .unwrap_or_else(|_| body.to_owned());

    match status.as_u16() {
        404 => format!(
            "Ollama model not found (404): {msg}\n\
             Hint: pull the model first with `ollama pull <model>`\n\
             Or check available models with `ollama list`"
        ),
        _ if msg.contains("connection refused") || msg.contains("connect error") => format!(
            "Cannot reach Ollama at {host}. Is it running?\n\
             Start it with: ollama serve\n\
             Or install from: https://ollama.com"
        ),
        _ => format!("Ollama error {status}: {msg}"),
    }
}

// ─── Provider impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String> {
        let ollama_messages: Vec<OllamaMessage> = messages
            .iter()
            .map(|m| OllamaMessage {
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
            messages: ollama_messages,
            stream: false,
            options: InferenceOptions {
                num_predict: self.config.max_tokens,
                temperature: self.config.temperature,
            },
        };

        let url = format!("{}/api/chat", self.config.host);
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    anyhow::anyhow!(
                        "Cannot connect to Ollama at {}. Is it running?\n\
                         Start it with: ollama serve\n\
                         Or install from: https://ollama.com",
                        self.config.host
                    )
                } else {
                    anyhow::anyhow!("Ollama request failed: {e}")
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();

            if status.as_u16() == 404 {
                let available = self.available_models().await;
                let hint = if available.is_empty() {
                    "No models found locally. Run `ollama pull llama3.2` to get started.".to_owned()
                } else {
                    format!(
                        "Available models: {}\nPull '{}' with: ollama pull {}",
                        available.join(", "),
                        self.config.model,
                        self.config.model
                    )
                };
                anyhow::bail!("Ollama model '{}' not found.\n{hint}", self.config.model);
            }

            anyhow::bail!(
                "{}",
                format_ollama_error(status, &body_text, &self.config.host)
            );
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Ollama response: {e}"))?;

        Ok(parsed.message.content)
    }

    async fn stream(
        &self,
        messages: &[Message],
        tx: mpsc::Sender<StreamChunk>,
    ) -> anyhow::Result<()> {
        let ollama_messages: Vec<OllamaMessage> = messages
            .iter()
            .map(|m| OllamaMessage {
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
            messages: ollama_messages,
            stream: true,
            options: InferenceOptions {
                num_predict: self.config.max_tokens,
                temperature: self.config.temperature,
            },
        };

        let url = format!("{}/api/chat", self.config.host);
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Ollama stream request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            let msg = format_ollama_error(status, &body_text, &self.config.host);
            let _ = tx.send(StreamChunk::Error(msg)).await;
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

            // Ollama sends one JSON object per line.
            while let Some(newline) = line_buf.find('\n') {
                let line = line_buf[..newline].trim().to_owned();
                line_buf = line_buf[newline + 1..].to_owned();

                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<StreamingChunk>(&line) {
                    Ok(chunk) => {
                        if !chunk.message.content.is_empty()
                            && tx
                                .send(StreamChunk::Delta(chunk.message.content))
                                .await
                                .is_err()
                        {
                            return Ok(());
                        }
                        if chunk.done {
                            let _ = tx.send(StreamChunk::Done).await;
                            return Ok(());
                        }
                    }
                    Err(_) => {
                        // Skip malformed lines (e.g. status/error objects).
                    }
                }
            }
        }

        // Stream ended without a done:true packet — still signal completion.
        let _ = tx.send(StreamChunk::Done).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_host_and_model() {
        let cfg = OllamaConfig::default();
        assert_eq!(cfg.host, DEFAULT_HOST);
        assert_eq!(cfg.model, DEFAULT_MODEL);
    }

    #[test]
    fn provider_name() {
        let p = OllamaProvider::new(OllamaConfig::default());
        assert_eq!(p.name(), "ollama");
    }

    #[test]
    fn format_error_404_has_pull_hint() {
        let msg = format_ollama_error(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"error":"model 'mistral' not found"}"#,
            DEFAULT_HOST,
        );
        assert!(msg.contains("ollama pull"));
        assert!(msg.contains("404"));
    }

    #[test]
    fn format_error_connection_refused_has_serve_hint() {
        let msg = format_ollama_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "connection refused",
            DEFAULT_HOST,
        );
        assert!(msg.contains("ollama serve"));
    }

    #[test]
    fn format_error_generic_includes_status() {
        let msg = format_ollama_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"out of memory"}"#,
            DEFAULT_HOST,
        );
        assert!(msg.contains("500"));
        assert!(msg.contains("out of memory"));
    }

    #[test]
    fn default_max_tokens_and_temperature() {
        let cfg = OllamaConfig::default();
        assert_eq!(cfg.max_tokens, 2048);
        assert!((cfg.temperature - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn chat_request_serialises_stream_false() {
        let req = ChatRequest {
            model: "llama3.2",
            messages: vec![OllamaMessage {
                role: "user",
                content: "hello",
            }],
            stream: false,
            options: InferenceOptions {
                num_predict: 100,
                temperature: 0.5,
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["stream"], false);
        assert_eq!(json["model"], "llama3.2");
        assert_eq!(json["messages"][0]["role"], "user");
    }

    #[test]
    fn chat_response_deserialises() {
        let raw =
            r#"{"model":"llama3.2","message":{"role":"assistant","content":"Hello!"},"done":true}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.message.content, "Hello!");
    }

    #[test]
    fn tags_response_deserialises() {
        let raw = r#"{"models":[{"name":"llama3.2:latest"},{"name":"mistral:latest"}]}"#;
        let parsed: TagsResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.models.len(), 2);
        assert_eq!(parsed.models[0].name, "llama3.2:latest");
    }

    #[test]
    fn ollama_error_deserialises() {
        let raw = r#"{"error":"model 'xyz' not found, try pulling it first"}"#;
        let e: OllamaError = serde_json::from_str(raw).unwrap();
        assert!(e.error.contains("not found"));
    }
}
