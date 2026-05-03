//! LLM provider abstraction.
//!
//! All AI backends implement `Provider`. The rest of the AI layer only
//! depends on this trait, keeping the LLM vendor details behind the boundary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// A single message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// A single token/chunk event emitted by a streaming LLM response.
///
/// Defined here (alongside `Provider`) so that the `stream()` method can
/// reference it without creating a circular dependency with `streaming.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamChunk {
    /// A text delta (one or more characters / tokens).
    Delta(String),
    /// The stream ended successfully.
    Done,
    /// The stream ended with an error.
    Error(String),
}

/// LLM provider abstraction.
///
/// Each backend implements this trait.  The session layer only calls
/// `complete()` or `stream()` and does not know which provider is in use.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name for display / logging.
    fn name(&self) -> &str;

    /// Send a sequence of messages and return the assistant's full reply.
    ///
    /// The provider is responsible for authentication, retry on transient
    /// HTTP errors, and rate limiting.
    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String>;

    /// Streaming variant of `complete()`.
    ///
    /// Sends `StreamChunk::Delta` events as tokens arrive, then a terminal
    /// `StreamChunk::Done` (or `StreamChunk::Error` on failure).
    ///
    /// The default implementation calls `complete()` and splits the result
    /// word-by-word so all providers get working (simulated) streaming for
    /// free.  Providers with native SSE / chunked HTTP should override this
    /// to emit real tokens as they arrive from the network.
    async fn stream(
        &self,
        messages: &[Message],
        tx: mpsc::Sender<StreamChunk>,
    ) -> anyhow::Result<()> {
        match self.complete(messages).await {
            Ok(text) => {
                for chunk in split_for_display(&text) {
                    if tx.send(StreamChunk::Delta(chunk)).await.is_err() {
                        return Ok(());
                    }
                }
                let _ = tx.send(StreamChunk::Done).await;
            }
            Err(e) => {
                let _ = tx.send(StreamChunk::Error(e.to_string())).await;
            }
        }
        Ok(())
    }
}

/// Split a complete response string into display-friendly chunks.
///
/// Each chunk is one "word" plus the trailing whitespace/newline that followed it,
/// so the rendered text remains readable as it streams into the chat widget.
pub(crate) fn split_for_display(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch == ' ' || ch == '\n' {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;

    /// A deterministic mock provider for unit tests.
    pub struct EchoProvider;

    #[async_trait]
    impl Provider for EchoProvider {
        fn name(&self) -> &str {
            "echo"
        }

        async fn complete(&self, messages: &[Message]) -> anyhow::Result<String> {
            let last = messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.as_str())
                .unwrap_or("(no message)");
            Ok(format!("Echo: {last}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_helpers::EchoProvider;

    #[test]
    fn split_for_display_on_spaces() {
        let chunks = split_for_display("hello world");
        assert_eq!(chunks, vec!["hello ", "world"]);
    }

    #[test]
    fn split_for_display_preserves_newlines() {
        let chunks = split_for_display("line1\nline2");
        assert!(chunks.iter().any(|c| c.contains('\n')));
    }

    #[test]
    fn split_for_display_empty() {
        assert!(split_for_display("").is_empty());
    }

    #[tokio::test]
    async fn default_stream_emits_done() {
        let p = EchoProvider;
        let (tx, mut rx) = mpsc::channel(64);
        p.stream(&[Message::user("hi")], tx).await.unwrap();

        let mut got_done = false;
        while let Ok(chunk) = rx.try_recv() {
            if chunk == StreamChunk::Done {
                got_done = true;
            }
        }
        assert!(got_done);
    }

    #[tokio::test]
    async fn default_stream_emits_content() {
        let p = EchoProvider;
        let (tx, mut rx) = mpsc::channel(64);
        p.stream(&[Message::user("hello")], tx).await.unwrap();

        let mut text = String::new();
        while let Ok(chunk) = rx.try_recv() {
            if let StreamChunk::Delta(d) = chunk {
                text.push_str(&d);
            }
        }
        assert!(text.contains("hello"), "got: {text}");
    }
}
