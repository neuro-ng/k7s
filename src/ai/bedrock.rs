//! AWS Bedrock LLM provider using manual SigV4 signing.
//!
//! Authenticates via environment variables or `~/.aws/credentials` profile
//! files.  No `aws-sdk-*` dependency — signing is done with `sha2` + `hmac`.
//!
//! # Configuration
//!
//! ```yaml
//! k7s:
//!   ai:
//!     provider: bedrock
//!     awsRegion: us-east-1          # default: us-east-1
//!     awsProfile: default           # default: env vars, then "default" profile
//!     awsModel: us.anthropic.claude-sonnet-4-5  # default
//! ```
//!
//! # k9s Reference
//! No equivalent — k7s-unique.

use std::path::PathBuf;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ai::provider::{Message, Provider, Role};

// ─── Constants ────────────────────────────────────────────────────────────────

pub const DEFAULT_MODEL: &str = "us.anthropic.claude-sonnet-4-5";
pub const DEFAULT_REGION: &str = "us-east-1";

// ─── AWS Credentials ──────────────────────────────────────────────────────────

/// AWS credentials used for SigV4 signing.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl AwsCredentials {
    /// Load credentials from the standard environment variables:
    /// `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`.
    ///
    /// Returns `None` if `AWS_ACCESS_KEY_ID` or `AWS_SECRET_ACCESS_KEY` are absent.
    pub fn from_env() -> Option<Self> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").ok()?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").ok()?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        Some(Self {
            access_key_id,
            secret_access_key,
            session_token,
        })
    }

    /// Load credentials from a named profile in `~/.aws/credentials`.
    pub fn from_profile(profile: &str) -> anyhow::Result<Self> {
        let path = credentials_path()?;
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!(
                "cannot read AWS credentials from {}: {e}",
                path.display()
            )
        })?;
        parse_credentials_ini(&raw, profile)
    }
}

/// Return the path to the AWS shared credentials file.
///
/// Respects `$AWS_SHARED_CREDENTIALS_FILE`; falls back to `~/.aws/credentials`.
fn credentials_path() -> anyhow::Result<PathBuf> {
    if let Ok(p) = std::env::var("AWS_SHARED_CREDENTIALS_FILE") {
        return Ok(PathBuf::from(p));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".aws").join("credentials"))
}

/// Parse an INI-formatted credentials file and return the named profile.
fn parse_credentials_ini(ini: &str, profile: &str) -> anyhow::Result<AwsCredentials> {
    let section_header = format!("[{profile}]");
    let mut in_section = false;
    let mut access_key_id: Option<String> = None;
    let mut secret_access_key: Option<String> = None;
    let mut session_token: Option<String> = None;

    for line in ini.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            in_section = line == section_header;
            continue;
        }

        if !in_section || line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "aws_access_key_id" => access_key_id = Some(value.trim().to_owned()),
                "aws_secret_access_key" => secret_access_key = Some(value.trim().to_owned()),
                "aws_session_token" => session_token = Some(value.trim().to_owned()),
                _ => {}
            }
        }
    }

    let access_key_id = access_key_id
        .ok_or_else(|| anyhow::anyhow!("profile [{profile}] missing aws_access_key_id"))?;
    let secret_access_key = secret_access_key
        .ok_or_else(|| anyhow::anyhow!("profile [{profile}] missing aws_secret_access_key"))?;

    Ok(AwsCredentials {
        access_key_id,
        secret_access_key,
        session_token,
    })
}

/// Load credentials: env vars take precedence, then credentials file.
///
/// If `profile` is `None`, the `"default"` profile is used as fallback.
pub fn load_credentials(profile: Option<&str>) -> anyhow::Result<AwsCredentials> {
    if let Some(creds) = AwsCredentials::from_env() {
        return Ok(creds);
    }
    AwsCredentials::from_profile(profile.unwrap_or("default"))
}

// ─── SigV4 helpers ────────────────────────────────────────────────────────────

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    bytes_to_hex(&hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key)
        .expect("HMAC accepts keys of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Derive the SigV4 signing key.
///
/// `HMAC(HMAC(HMAC(HMAC("AWS4" + secret, date), region), service), "aws4_request")`
fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Return the sorted signed-headers string.
///
/// Alphabetical order: content-type < host < x-amz-date < x-amz-security-token.
fn signed_headers(has_session_token: bool) -> &'static str {
    if has_session_token {
        "content-type;host;x-amz-date;x-amz-security-token"
    } else {
        "content-type;host;x-amz-date"
    }
}

/// Build the full set of SigV4-signed request headers for a Bedrock Converse call.
fn build_auth_headers(
    creds: &AwsCredentials,
    region: &str,
    model_id: &str,
    body: &[u8],
) -> anyhow::Result<HeaderMap> {
    // 1. Timestamps
    let now = chrono::Utc::now();
    let datetime = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();

    // 2. Host
    let host = format!("bedrock-runtime.{region}.amazonaws.com");

    // 3. Canonical headers (alphabetically sorted, each line ends with \n)
    let has_token = creds.session_token.is_some();
    let canonical_headers = if has_token {
        format!(
            "content-type:application/json\nhost:{host}\nx-amz-date:{datetime}\nx-amz-security-token:{}\n",
            creds.session_token.as_deref().unwrap_or("")
        )
    } else {
        format!("content-type:application/json\nhost:{host}\nx-amz-date:{datetime}\n")
    };

    let sh = signed_headers(has_token);

    // 4. Canonical request
    let payload_hash = sha256_hex(body);
    let canonical_request = format!(
        "POST\n/model/{model_id}/converse\n\n{canonical_headers}\n{sh}\n{payload_hash}"
    );

    // 5. Credential scope
    let credential_scope = format!("{date}/{region}/bedrock-runtime/aws4_request");

    // 6. String to sign
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{datetime}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    // 7. Signature
    let key = signing_key(&creds.secret_access_key, &date, region, "bedrock-runtime");
    let signature = bytes_to_hex(&hmac_sha256(&key, string_to_sign.as_bytes()));

    // 8. Authorization header value
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope},SignedHeaders={sh},Signature={signature}",
        creds.access_key_id
    );

    // 9. Build HeaderMap
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "x-amz-date",
        HeaderValue::from_str(&datetime)
            .map_err(|e| anyhow::anyhow!("invalid x-amz-date header: {e}"))?,
    );
    headers.insert(
        reqwest::header::AUTHORIZATION,
        HeaderValue::from_str(&authorization)
            .map_err(|e| anyhow::anyhow!("invalid authorization header: {e}"))?,
    );
    if let Some(token) = &creds.session_token {
        headers.insert(
            "x-amz-security-token",
            HeaderValue::from_str(token)
                .map_err(|e| anyhow::anyhow!("invalid x-amz-security-token header: {e}"))?,
        );
    }

    Ok(headers)
}

// ─── Error helper ─────────────────────────────────────────────────────────────

fn format_bedrock_error(status: reqwest::StatusCode, body: &str) -> String {
    #[derive(Deserialize)]
    struct ErrBody {
        #[serde(alias = "Message")]
        message: Option<String>,
    }

    let msg = serde_json::from_str::<ErrBody>(body)
        .ok()
        .and_then(|e| e.message)
        .unwrap_or_else(|| body.to_owned());

    if status == reqwest::StatusCode::FORBIDDEN {
        format!(
            "Bedrock request forbidden ({status}): {msg}\n\
             Hint: check your IAM permissions for bedrock:InvokeModel on the target model."
        )
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        format!(
            "Bedrock throttling error ({status}): {msg}\n\
             Hint: you are being throttled by the Bedrock service — retry after a back-off."
        )
    } else {
        format!("Bedrock API error ({status}): {msg}")
    }
}

// ─── Wire types ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ConverseRequest<'a> {
    messages: Vec<BMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<SysBlock<'a>>,
    #[serde(rename = "inferenceConfig")]
    inference_config: InfConfig,
}

#[derive(Serialize)]
struct BMessage<'a> {
    role: &'a str,
    content: Vec<TextBlock<'a>>,
}

#[derive(Serialize)]
struct TextBlock<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct SysBlock<'a> {
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InfConfig {
    max_tokens: u32,
    temperature: f32,
}

#[derive(Deserialize)]
struct ConverseResponse {
    output: ConverseOutput,
    usage: Option<UsageBlock>,
}

#[derive(Deserialize)]
struct ConverseOutput {
    message: RMessage,
}

#[derive(Deserialize)]
struct RMessage {
    content: Vec<RTextBlock>,
}

#[derive(Deserialize)]
struct RTextBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct UsageBlock {
    #[serde(rename = "totalTokens")]
    total_tokens: Option<u32>,
}

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for [`BedrockProvider`].
#[derive(Debug, Clone)]
pub struct BedrockProviderConfig {
    /// AWS region where the Bedrock endpoint lives.
    pub region: String,
    /// AWS credentials profile name. `None` means use env vars first, then "default".
    pub profile: Option<String>,
    /// Bedrock model ID.
    pub model: String,
    /// Maximum tokens to generate per response.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–1.0).
    pub temperature: f32,
}

impl Default for BedrockProviderConfig {
    fn default() -> Self {
        Self {
            region: DEFAULT_REGION.to_owned(),
            profile: None,
            model: DEFAULT_MODEL.to_owned(),
            max_tokens: 2048,
            temperature: 0.3,
        }
    }
}

// ─── Provider ─────────────────────────────────────────────────────────────────

/// AWS Bedrock provider using manual SigV4 request signing.
///
/// Credentials are resolved at call time:
/// 1. `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` env vars (with optional `AWS_SESSION_TOKEN`)
/// 2. Profile in `~/.aws/credentials` (or `$AWS_SHARED_CREDENTIALS_FILE`)
pub struct BedrockProvider {
    config: BedrockProviderConfig,
    http: reqwest::Client,
}

impl BedrockProvider {
    pub fn new(config: BedrockProviderConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .build()
                .expect("HTTP client"),
        }
    }
}

#[async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String> {
        // Resolve credentials synchronously (file I/O is fast; avoids async complexity).
        let creds = load_credentials(self.config.profile.as_deref())?;

        // Separate system messages from user/assistant messages.
        let system_blocks: Vec<SysBlock> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| SysBlock {
                text: m.content.as_str(),
            })
            .collect();

        let b_messages: Vec<BMessage> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| BMessage {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant | Role::System => "assistant",
                },
                content: vec![TextBlock {
                    text: m.content.as_str(),
                }],
            })
            .collect();

        if b_messages.is_empty() {
            anyhow::bail!("no user/assistant messages to send to Bedrock");
        }

        let request = ConverseRequest {
            messages: b_messages,
            system: system_blocks,
            inference_config: InfConfig {
                max_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
            },
        };

        let body = serde_json::to_vec(&request)?;

        let url = format!(
            "https://bedrock-runtime.{region}.amazonaws.com/model/{model}/converse",
            region = self.config.region,
            model = self.config.model,
        );

        let headers = build_auth_headers(&creds, &self.config.region, &self.config.model, &body)?;

        tracing::debug!(url = %url, model = %self.config.model, "Bedrock Converse request");

        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .body(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("{}", format_bedrock_error(status, &text));
        }

        let parsed: ConverseResponse = resp.json().await?;

        if let Some(usage) = &parsed.usage {
            if let Some(total) = usage.total_tokens {
                tracing::debug!(total_tokens = total, "Bedrock token usage");
            }
        }

        parsed
            .output
            .message
            .content
            .into_iter()
            .find_map(|b| b.text)
            .ok_or_else(|| anyhow::anyhow!("Bedrock response contained no text content"))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::Message;

    #[test]
    fn default_config_region() {
        assert_eq!(BedrockProviderConfig::default().region, "us-east-1");
    }

    #[test]
    fn default_model_is_claude() {
        assert!(BedrockProviderConfig::default().model.starts_with("us.anthropic"));
    }

    #[test]
    fn provider_name() {
        let p = BedrockProvider::new(BedrockProviderConfig::default());
        assert_eq!(p.name(), "bedrock");
    }

    #[test]
    fn credentials_path_respects_env() {
        std::env::set_var("AWS_SHARED_CREDENTIALS_FILE", "/tmp/test_creds");
        let path = credentials_path().unwrap();
        assert_eq!(path.to_str().unwrap(), "/tmp/test_creds");
        std::env::remove_var("AWS_SHARED_CREDENTIALS_FILE");
    }

    #[test]
    fn parse_credentials_profile_basic() {
        let ini = "[default]\naws_access_key_id = AKIAIOSFODNN7EXAMPLE\naws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\n";
        let creds = parse_credentials_ini(ini, "default").unwrap();
        assert_eq!(creds.access_key_id, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(
            creds.secret_access_key,
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        );
        assert!(creds.session_token.is_none());
    }

    #[test]
    fn system_message_extracted() {
        let messages = [Message::system("be helpful")];
        let system_blocks: Vec<SysBlock> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| SysBlock {
                text: m.content.as_str(),
            })
            .collect();

        assert!(!system_blocks.is_empty());
    }

    #[tokio::test]
    async fn no_user_messages_errors() {
        let p = BedrockProvider::new(BedrockProviderConfig::default());
        let messages = vec![Message::system("system only")];
        let result = p.complete(&messages).await;
        assert!(result.is_err(), "expected Err when no user/assistant messages");
    }

    #[test]
    fn format_bedrock_error_403() {
        let msg = format_bedrock_error(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"message":"Access denied"}"#,
        );
        assert!(
            msg.contains("IAM"),
            "403 error should mention IAM, got: {msg}"
        );
    }

    #[test]
    fn format_bedrock_error_429() {
        let msg = format_bedrock_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            r#"{"message":"Rate exceeded"}"#,
        );
        assert!(
            msg.to_lowercase().contains("throttl"),
            "429 error should mention throttling, got: {msg}"
        );
    }

    #[test]
    fn bytes_to_hex_known() {
        assert_eq!(bytes_to_hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    #[test]
    fn sha256_hex_empty() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
