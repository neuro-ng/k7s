//! Google Antigravity (Vertex AI Gemini) LLM provider.
//!
//! Authenticates via Application Default Credentials (ADC).
//! Run `gcloud auth application-default login` once to set up credentials, or
//! point `GOOGLE_APPLICATION_CREDENTIALS` at a service-account JSON file.
//!
//! # Configuration
//!
//! ```yaml
//! k7s:
//!   ai:
//!     provider: antigravity
//!     gcpProject: my-project-id        # required
//!     gcpRegion: us-central1           # default: us-central1
//!     model: gemini-1.5-pro            # default: gemini-1.5-pro
//! ```
//!
//! # k9s Reference
//! No equivalent — k7s-unique.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::ai::provider::{Message, Provider, Role};

// ─── Constants ────────────────────────────────────────────────────────────────

const OAUTH2_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const DEFAULT_MODEL: &str = "gemini-1.5-pro";
pub const DEFAULT_REGION: &str = "us-central1";
/// Refresh the token 60 s before it actually expires.
const REFRESH_HEADROOM: Duration = Duration::from_secs(60);

// ─── ADC credential file ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AdcFile {
    #[serde(rename = "type")]
    cred_type: String,
    // authorized_user (gcloud auth application-default login)
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
}

// ─── Cached access token ──────────────────────────────────────────────────────

#[derive(Clone)]
struct CachedToken {
    value: String,
    expires_at: Instant,
}

impl CachedToken {
    fn is_valid(&self) -> bool {
        Instant::now() + REFRESH_HEADROOM < self.expires_at
    }
}

// ─── OAuth2 wire types ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

// ─── ADC token fetcher ────────────────────────────────────────────────────────

struct AdcTokenFetcher {
    adc: AdcFile,
    cached: Mutex<Option<CachedToken>>,
    http: Client,
}

impl AdcTokenFetcher {
    async fn load() -> anyhow::Result<Self> {
        let path = adc_path()?;
        let raw = tokio::fs::read_to_string(&path).await.map_err(|e| {
            anyhow::anyhow!(
                "cannot read ADC credentials from {}: {e}\n\
                 Run: gcloud auth application-default login",
                path.display()
            )
        })?;
        let adc: AdcFile = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("malformed ADC file {}: {e}", path.display()))?;
        Ok(Self {
            adc,
            cached: Mutex::new(None),
            http: Client::new(),
        })
    }

    async fn get_token(&self) -> anyhow::Result<String> {
        let mut guard = self.cached.lock().await;
        if let Some(tok) = guard.as_ref() {
            if tok.is_valid() {
                return Ok(tok.value.clone());
            }
        }
        let fresh = self.refresh().await?;
        let value = fresh.value.clone();
        *guard = Some(fresh);
        Ok(value)
    }

    async fn refresh(&self) -> anyhow::Result<CachedToken> {
        match self.adc.cred_type.as_str() {
            "authorized_user" => self.exchange_refresh_token().await,
            other => anyhow::bail!(
                "ADC credential type '{other}' is not yet supported; \
                 use authorized_user credentials from `gcloud auth application-default login`"
            ),
        }
    }

    async fn exchange_refresh_token(&self) -> anyhow::Result<CachedToken> {
        let client_id = self
            .adc
            .client_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("ADC file missing client_id"))?;
        let client_secret = self
            .adc
            .client_secret
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("ADC file missing client_secret"))?;
        let refresh_token = self
            .adc
            .refresh_token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("ADC file missing refresh_token"))?;

        let resp = self
            .http
            .post(OAUTH2_TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OAuth2 token refresh failed ({status}): {body}");
        }

        let tok: TokenResponse = resp.json().await?;
        Ok(CachedToken {
            value: tok.access_token,
            expires_at: Instant::now() + Duration::from_secs(tok.expires_in),
        })
    }
}

fn adc_path() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(p) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        return Ok(std::path::PathBuf::from(p));
    }
    let base = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine user config directory"))?;
    Ok(base
        .join("gcloud")
        .join("application_default_credentials.json"))
}

// ─── Vertex AI Gemini wire types ──────────────────────────────────────────────

#[derive(Serialize)]
struct GenerateRequest<'a> {
    contents: Vec<GeminiContent<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction<'a>>,
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
struct GeminiContent<'a> {
    role: &'a str,
    parts: [TextPart<'a>; 1],
}

#[derive(Serialize)]
struct SystemInstruction<'a> {
    parts: [TextPart<'a>; 1],
}

#[derive(Serialize)]
struct TextPart<'a> {
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    max_output_tokens: u32,
    temperature: f32,
}

#[derive(Deserialize)]
struct GenerateResponse {
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    content: GeminiResponseContent,
}

#[derive(Deserialize)]
struct GeminiResponseContent {
    parts: Vec<ResponsePart>,
}

#[derive(Deserialize)]
struct ResponsePart {
    text: String,
}

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for [`AntigravityProvider`].
#[derive(Debug, Clone)]
pub struct AntigravityConfig {
    /// GCP project ID (required).
    pub project: String,
    /// Vertex AI region. Default: `"us-central1"`.
    pub region: String,
    /// Gemini model ID. Default: `"gemini-1.5-pro"`.
    pub model: String,
    /// Maximum tokens to generate per response.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0).
    pub temperature: f32,
}

impl Default for AntigravityConfig {
    fn default() -> Self {
        Self {
            project: String::new(),
            region: DEFAULT_REGION.to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            max_tokens: 2048,
            temperature: 0.3,
        }
    }
}

// ─── Provider ─────────────────────────────────────────────────────────────────

/// Google Vertex AI Gemini provider using Application Default Credentials.
///
/// The ADC token fetcher is initialised lazily on the first `complete()` call.
pub struct AntigravityProvider {
    config: AntigravityConfig,
    /// Lazily-loaded ADC fetcher so `new()` is synchronous.
    fetcher: Mutex<Option<Arc<AdcTokenFetcher>>>,
    http: Client,
}

impl AntigravityProvider {
    pub fn new(config: AntigravityConfig) -> Self {
        Self {
            config,
            fetcher: Mutex::new(None),
            http: Client::builder()
                .timeout(Duration::from_secs(90))
                .build()
                .expect("HTTP client"),
        }
    }

    async fn token(&self) -> anyhow::Result<String> {
        let mut guard = self.fetcher.lock().await;
        if guard.is_none() {
            *guard = Some(Arc::new(AdcTokenFetcher::load().await?));
        }
        guard.as_ref().unwrap().get_token().await
    }

    fn endpoint(&self) -> String {
        format!(
            "https://{r}-aiplatform.googleapis.com/v1/projects/{p}/locations/{r}/publishers/google/models/{m}:generateContent",
            r = self.config.region,
            p = self.config.project,
            m = self.config.model,
        )
    }
}

#[async_trait]
impl Provider for AntigravityProvider {
    fn name(&self) -> &str {
        "antigravity"
    }

    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String> {
        if self.config.project.is_empty() {
            anyhow::bail!(
                "GCP project is not configured. Set ai.gcpProject in config.yaml \
                 or the GOOGLE_CLOUD_PROJECT environment variable."
            );
        }

        let token = self.token().await?;

        let system_text: Option<&str> = messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.as_str());

        let contents: Vec<GeminiContent> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| GeminiContent {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant | Role::System => "model",
                },
                parts: [TextPart { text: &m.content }],
            })
            .collect();

        if contents.is_empty() {
            anyhow::bail!("no user/assistant messages to send");
        }

        let body = GenerateRequest {
            contents,
            system_instruction: system_text.map(|t| SystemInstruction {
                parts: [TextPart { text: t }],
            }),
            generation_config: GenerationConfig {
                max_output_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
            },
        };

        let url = self.endpoint();
        tracing::debug!(url = %url, model = %self.config.model, "Antigravity request");

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity API error {status}: {body}");
        }

        let parsed: GenerateResponse = resp.json().await?;
        parsed
            .candidates
            .into_iter()
            .next()
            .and_then(|c| c.content.parts.into_iter().next())
            .map(|p| p.text)
            .ok_or_else(|| anyhow::anyhow!("Antigravity response contained no candidates"))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_us_central1() {
        let cfg = AntigravityConfig::default();
        assert_eq!(cfg.region, "us-central1");
    }

    #[test]
    fn default_model_is_gemini() {
        let cfg = AntigravityConfig::default();
        assert!(cfg.model.starts_with("gemini"));
    }

    #[test]
    fn provider_name_is_antigravity() {
        let p = AntigravityProvider::new(AntigravityConfig::default());
        assert_eq!(p.name(), "antigravity");
    }

    #[test]
    fn endpoint_contains_project_region_model() {
        let p = AntigravityProvider::new(AntigravityConfig {
            project: "my-proj".into(),
            region: "europe-west4".into(),
            model: "gemini-1.5-flash".into(),
            ..Default::default()
        });
        let url = p.endpoint();
        assert!(url.contains("my-proj"));
        assert!(url.contains("europe-west4"));
        assert!(url.contains("gemini-1.5-flash"));
    }

    #[test]
    fn adc_path_respects_env_var() {
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", "/tmp/sa.json");
        let path = adc_path().unwrap();
        assert_eq!(path.to_str().unwrap(), "/tmp/sa.json");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
    }
}
