//! LLM provider abstraction.
//!
//! All AI backends implement `Provider`. The rest of the AI layer only
//! depends on this trait, keeping the LLM vendor details behind the boundary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A single message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// LLM provider abstraction.
///
/// Each backend implements this trait.  The session layer only calls
/// `complete()` and does not know which provider is in use.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name for display / logging.
    fn name(&self) -> &str;

    /// Send a sequence of messages and return the assistant's reply.
    ///
    /// The provider is responsible for authentication, retry on transient
    /// HTTP errors, and rate limiting.
    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String>;
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;

    /// A deterministic mock provider for unit tests.
    pub struct EchoProvider;

    #[async_trait]
    impl Provider for EchoProvider {
        fn name(&self) -> &str { "echo" }

        async fn complete(&self, messages: &[Message]) -> anyhow::Result<String> {
            // Echo back the last user message prefixed with "Echo: ".
            let last = messages.iter().rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.as_str())
                .unwrap_or("(no message)");
            Ok(format!("Echo: {last}"))
        }
    }
}
