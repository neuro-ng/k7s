//! Chat session management.
//!
//! A `ChatSession` maintains conversation history and enforces the security
//! invariant: only `SafeMetadata` (sanitizer output) is allowed in context.
//!
//! # Security
//!
//! The session accepts cluster context only as `SafeMetadata` values — raw
//! Kubernetes resources must NEVER be passed directly.  The `add_context()`
//! method is the enforced ingress point.

use crate::ai::provider::{Message, Provider};
use crate::ai::token_budget::{estimate_tokens, BudgetCheck, TokenBudget};
use crate::config::{SanitizerConfig, TokenBudgetConfig};
use crate::sanitizer::{Redactor, SafeMetadata};

/// A complete message exchange result.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub user_message:    String,
    pub assistant_reply: String,
    pub tokens_used:     u32,
}

/// Error types for session-level failures.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session token budget exhausted ({used}/{max} tokens used)")]
    BudgetExhausted { used: u32, max: u32 },

    #[error("query too large: {tokens} tokens exceeds per-query limit of {limit}")]
    QueryTooLarge { tokens: u32, limit: u32 },

    #[error("provider error: {0}")]
    Provider(#[from] anyhow::Error),
}

/// An active AI chat session with conversation history and budget tracking.
pub struct ChatSession {
    history:  Vec<Message>,
    budget:   TokenBudget,
    /// Accumulated sanitized cluster context injected into the system prompt.
    context:  Vec<SafeMetadata>,
    /// Redactor applied to user free-text input before it enters the LLM stream.
    ///
    /// # Security
    ///
    /// User input is NOT trusted. A user may paste raw `kubectl get secret -o yaml`
    /// output or copy-paste connection strings from other tools. All user messages
    /// pass through this redactor before being added to the message list.
    redactor: Redactor,
}

impl ChatSession {
    /// Create a new session.
    ///
    /// `sanitizer_cfg` is used to build the input redactor — pass
    /// `SanitizerConfig::default()` if no custom patterns are required.
    pub fn new(budget_cfg: &TokenBudgetConfig, sanitizer_cfg: &SanitizerConfig) -> Self {
        let redactor = Redactor::new(&sanitizer_cfg.custom_patterns)
            .expect("SanitizerConfig custom_patterns must contain valid regex");
        Self {
            history: Vec::new(),
            budget:  TokenBudget::from_config(budget_cfg),
            context: Vec::new(),
            redactor,
        }
    }

    /// Inject sanitized cluster context into the next query.
    ///
    /// This is the **only** way to add cluster data to a session.
    /// The data must have passed through `crate::sanitizer::sanitize()`.
    pub fn add_context(&mut self, metadata: SafeMetadata) {
        self.context.push(metadata);
    }

    /// Clear accumulated cluster context (after it has been sent).
    pub fn clear_context(&mut self) {
        self.context.clear();
    }

    /// Send a user message and receive an assistant reply.
    ///
    /// # Security
    ///
    /// This method builds the full message list including the system prompt
    /// with sanitized cluster context.  Raw resource data from the DAO layer
    /// must never reach this method directly.
    pub async fn send(
        &mut self,
        provider: &dyn Provider,
        user_message: impl Into<String>,
    ) -> Result<Exchange, SessionError> {
        let user_msg = user_message.into();

        // Build the full message list for this request.
        let messages = self.build_messages(&user_msg);
        let estimated = messages.iter()
            .map(|m| estimate_tokens(&m.content))
            .sum::<u32>();

        match self.budget.check(estimated) {
            BudgetCheck::Ok => {}
            BudgetCheck::Warning { used, max } => {
                tracing::warn!(used, max, "approaching token budget limit");
            }
            BudgetCheck::Exhausted => {
                return Err(SessionError::BudgetExhausted {
                    used: self.budget.used(),
                    max:  self.budget.max_session(),
                });
            }
            BudgetCheck::QueryTooLarge { tokens, limit } => {
                return Err(SessionError::QueryTooLarge { tokens, limit });
            }
        }

        let reply = provider.complete(&messages).await?;
        let reply_tokens = estimate_tokens(&reply);
        self.budget.record_usage(estimated + reply_tokens);

        // Persist the exchange in history for follow-up questions.
        self.history.push(Message::user(user_msg.clone()));
        self.history.push(Message::assistant(reply.clone()));

        // Clear context after use (incremental context strategy).
        self.clear_context();

        Ok(Exchange {
            user_message:    user_msg,
            assistant_reply: reply,
            tokens_used:     estimated + reply_tokens,
        })
    }

    pub fn history(&self) -> &[Message] { &self.history }
    pub fn budget(&self) -> &TokenBudget { &self.budget }
    pub fn context_len(&self) -> usize { self.context.len() }

    /// Build the full message list for a send — exposed for streaming.
    pub fn messages_for_send(&self, user_message: &str) -> Vec<Message> {
        self.build_messages(user_message)
    }

    fn build_messages(&self, user_message: &str) -> Vec<Message> {
        let mut msgs = Vec::new();

        // System prompt with k7s identity + sanitized cluster context.
        let mut system = concat!(
            "You are k7s, an AI assistant embedded in a Kubernetes terminal UI. ",
            "You help diagnose issues, recommend optimisations, and explain cluster state. ",
            "All cluster data you receive has been sanitised — secrets, tokens, and ",
            "credentials have been removed. Never speculate about secret values."
        ).to_owned();

        if !self.context.is_empty() {
            system.push_str("\n\n## Current cluster context (sanitised)\n\n");
            for meta in &self.context {
                if let Ok(json) = serde_json::to_string_pretty(&meta.fields) {
                    system.push_str(&format!("### {} {}/{}\n```json\n{}\n```\n\n",
                        meta.gvr,
                        meta.namespace.as_deref().unwrap_or(""),
                        meta.name,
                        json,
                    ));
                }
            }
        }

        msgs.push(Message::system(system));

        // Conversation history (prior turns).
        msgs.extend(self.history.iter().cloned());

        // Current user turn — redact any accidentally pasted secrets.
        let sanitized_input = self.redactor.redact_str(user_message);
        if sanitized_input != user_message {
            tracing::warn!(
                "user input contained secret-like patterns — content was redacted before \
                 being sent to the LLM"
            );
        }
        msgs.push(Message::user(sanitized_input));

        msgs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::test_helpers::EchoProvider;
    use crate::config::{SanitizerConfig, TokenBudgetConfig};
    use serde_json::json;

    fn session() -> ChatSession {
        ChatSession::new(&TokenBudgetConfig::default(), &SanitizerConfig::default())
    }

    #[tokio::test]
    async fn send_returns_echo_reply() {
        let mut session = session();
        let provider = EchoProvider;
        let exchange = session.send(&provider, "Hello").await.unwrap();
        assert!(exchange.assistant_reply.contains("Hello"));
    }

    #[tokio::test]
    async fn history_grows_with_exchanges() {
        let mut session = session();
        let provider = EchoProvider;
        session.send(&provider, "First").await.unwrap();
        session.send(&provider, "Second").await.unwrap();
        assert_eq!(session.history().len(), 4); // 2 user + 2 assistant
    }

    #[tokio::test]
    async fn context_is_cleared_after_send() {
        let mut session = session();
        session.add_context(SafeMetadata {
            gvr: "v1/pods".to_owned(),
            namespace: Some("default".to_owned()),
            name: "my-pod".to_owned(),
            fields: json!({}),
        });
        assert_eq!(session.context_len(), 1);
        let provider = EchoProvider;
        session.send(&provider, "What's wrong?").await.unwrap();
        assert_eq!(session.context_len(), 0);
    }

    #[test]
    fn budget_exhausted_error() {
        let mut session = ChatSession::new(
            &TokenBudgetConfig {
                max_per_session: 1,
                // max_per_query must be large enough that the system prompt itself
                // doesn't hit the per-query limit first.
                max_per_query: 100_000,
                warn_at: 1,
            },
            &SanitizerConfig::default(),
        );
        // Exhaust the session budget (used >= max).
        session.budget.record_usage(1);
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(session.send(&EchoProvider, "hi"));
        assert!(matches!(result, Err(SessionError::BudgetExhausted { .. })));
    }

    #[tokio::test]
    async fn user_input_secrets_are_redacted_before_send() {
        let mut session = session();
        let provider = EchoProvider;
        // Simulate a user pasting a kubectl secret output into the chat.
        let pasted = "my DB is at postgres://admin:hunter2@db:5432/prod";
        let exchange = session.send(&provider, pasted).await.unwrap();
        // The EchoProvider echoes the user message — if the connection string
        // made it through, the reply would contain "hunter2".
        assert!(
            !exchange.assistant_reply.contains("hunter2"),
            "secret value must not reach the LLM: {}",
            exchange.assistant_reply
        );
        assert!(
            exchange.assistant_reply.contains("[REDACTED]"),
            "redaction marker must appear in echoed reply: {}",
            exchange.assistant_reply
        );
    }

    #[tokio::test]
    async fn safe_user_input_passes_through_unchanged() {
        let mut session = session();
        let provider = EchoProvider;
        let safe_msg = "How many pods are running in the default namespace?";
        let exchange = session.send(&provider, safe_msg).await.unwrap();
        assert!(exchange.assistant_reply.contains("How many pods"));
    }
}
