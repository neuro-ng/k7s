//! Streaming response display — Phase 12.15.
//!
//! Wraps the `Provider::complete()` call in a streaming abstraction so the
//! TUI can update the chat window token-by-token (or chunk-by-chunk) as the
//! LLM produces output, rather than waiting for the full response before
//! rendering.
//!
//! # Design
//!
//! Real streaming (SSE / chunked HTTP) requires provider-specific code.
//! This module provides:
//!
//! 1. `StreamHandle` — a `tokio::sync::mpsc::Receiver<StreamChunk>` wrapped
//!    in a newtype so the TUI can poll it without knowing the provider details.
//! 2. `StreamChunk` — either a text delta or a terminal `Done` / `Error`.
//! 3. `stream_complete()` — a shim that calls `Provider::complete()` and
//!    re-broadcasts the full response word-by-word so the TUI sees a stream
//!    even when the provider does not support true streaming.
//! 4. `StreamingSession` — a thin wrapper around `ChatSession` that sends
//!    messages through a `StreamHandle`.
//!
//! When a future provider adds native SSE streaming, only step 3 needs to be
//! replaced; the TUI and session layers remain unchanged.

use tokio::sync::mpsc;

use crate::ai::provider::{Message, Provider};
use crate::ai::session::{ChatSession, SessionError};
use crate::ai::token_budget::estimate_tokens;

// ─── StreamChunk ─────────────────────────────────────────────────────────────

/// A single event emitted by a streaming LLM response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamChunk {
    /// A text delta (one or more characters / tokens).
    Delta(String),
    /// The stream ended successfully.
    Done,
    /// The stream ended with an error.
    Error(String),
}

// ─── StreamHandle ─────────────────────────────────────────────────────────────

/// A handle to an in-flight streaming LLM response.
///
/// Call `next_chunk()` from the TUI render loop (non-blocking) to drain
/// available chunks without blocking the UI thread.
pub struct StreamHandle {
    rx: mpsc::Receiver<StreamChunk>,
}

impl StreamHandle {
    fn new(rx: mpsc::Receiver<StreamChunk>) -> Self {
        Self { rx }
    }

    /// Non-blocking drain: collect all chunks currently in the channel buffer.
    ///
    /// Returns `true` when a `Done` or `Error` chunk is included (stream ended).
    pub fn drain(&mut self) -> (Vec<StreamChunk>, bool) {
        let mut chunks = Vec::new();
        let mut finished = false;
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => {
                    let is_terminal = matches!(chunk, StreamChunk::Done | StreamChunk::Error(_));
                    chunks.push(chunk);
                    if is_terminal {
                        finished = true;
                        break;
                    }
                }
                Err(mpsc::error::TryRecvError::Empty)        => break,
                Err(mpsc::error::TryRecvError::Disconnected) => { finished = true; break; }
            }
        }
        (chunks, finished)
    }
}

// ─── stream_complete() ────────────────────────────────────────────────────────

/// Kick off a `Provider::complete()` call and return a `StreamHandle`.
///
/// The response is broken into word-sized chunks and fed into the channel so
/// the TUI sees incremental updates even with non-streaming providers.
///
/// `channel_size` is the mpsc buffer depth; 256 is fine for UI polling.
pub fn stream_complete(
    provider: std::sync::Arc<dyn Provider>,
    messages: Vec<Message>,
    channel_size: usize,
) -> StreamHandle {
    let (tx, rx) = mpsc::channel(channel_size);

    tokio::spawn(async move {
        match provider.complete(&messages).await {
            Ok(response) => {
                // Emit the response word-by-word so the TUI updates progressively.
                for word in tokenise_response(&response) {
                    if tx.send(StreamChunk::Delta(word)).await.is_err() {
                        return; // Receiver dropped (e.g. user closed the chat window).
                    }
                }
                let _ = tx.send(StreamChunk::Done).await;
            }
            Err(e) => {
                let _ = tx.send(StreamChunk::Error(e.to_string())).await;
            }
        }
    });

    StreamHandle::new(rx)
}

/// Split a response string into display-friendly chunks.
///
/// Each chunk is one "word" plus the trailing whitespace that followed it,
/// so the rendered text stays readable as it arrives.
fn tokenise_response(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if (ch == ' ' || ch == '\n') && !current.is_empty() {
            chunks.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

// ─── StreamingSession ─────────────────────────────────────────────────────────

/// The result of a streaming send: accumulates the full text for history,
/// plus a handle to consume the stream from the TUI.
pub struct StreamingSend {
    /// Live stream handle — poll this from the render loop.
    pub handle: StreamHandle,
    /// Estimated token count for budget tracking.
    pub estimated_tokens: u32,
}

/// Streaming variant of `ChatSession::send`.
///
/// Kicks off a background task and returns immediately with a `StreamHandle`.
/// The caller is responsible for:
/// 1. Draining the handle each render frame.
/// 2. Calling `session.record_streaming_exchange()` when `Done` is received.
pub fn send_streaming(
    session: &ChatSession,
    provider: std::sync::Arc<dyn Provider>,
    user_message: impl Into<String>,
) -> Result<StreamingSend, SessionError> {
    let user_msg = user_message.into();
    let messages = session.messages_for_send(&user_msg);

    let estimated: u32 = messages.iter()
        .map(|m| estimate_tokens(&m.content))
        .sum();

    // Budget check (delegates to the session's budget).
    use crate::ai::token_budget::BudgetCheck;
    match session.budget().check(estimated) {
        BudgetCheck::Ok | BudgetCheck::Warning { .. } => {}
        BudgetCheck::Exhausted => {
            return Err(SessionError::BudgetExhausted {
                used: session.budget().used(),
                max:  session.budget().max_session(),
            });
        }
        BudgetCheck::QueryTooLarge { tokens, limit } => {
            return Err(SessionError::QueryTooLarge { tokens, limit });
        }
    }

    let handle = stream_complete(provider, messages, 256);
    Ok(StreamingSend { handle, estimated_tokens: estimated })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::test_helpers::EchoProvider;
    use std::sync::Arc;

    #[tokio::test]
    async fn stream_complete_emits_done() {
        let provider = Arc::new(EchoProvider);
        let messages = vec![Message::user("hello world")];
        let mut handle = stream_complete(provider, messages, 64);

        // Collect until done.
        let mut text = String::new();
        let mut finished = false;
        while !finished {
            tokio::task::yield_now().await;
            let (chunks, done) = handle.drain();
            for chunk in chunks {
                match chunk {
                    StreamChunk::Delta(d) => text.push_str(&d),
                    StreamChunk::Done     => {}
                    StreamChunk::Error(e) => panic!("unexpected error: {e}"),
                }
            }
            if done { finished = true; }
        }

        assert!(text.contains("hello world"), "got: {text}");
    }

    #[tokio::test]
    async fn error_provider_emits_error_chunk() {
        use async_trait::async_trait;
        use crate::ai::provider::Provider;

        struct FailProvider;
        #[async_trait]
        impl Provider for FailProvider {
            fn name(&self) -> &str { "fail" }
            async fn complete(&self, _: &[Message]) -> anyhow::Result<String> {
                anyhow::bail!("simulated failure")
            }
        }

        let provider = Arc::new(FailProvider);
        let mut handle = stream_complete(provider, vec![Message::user("hi")], 8);

        let mut got_error = false;
        for _ in 0..50 {
            tokio::task::yield_now().await;
            let (chunks, done) = handle.drain();
            for chunk in chunks {
                if matches!(chunk, StreamChunk::Error(_)) {
                    got_error = true;
                }
            }
            if done { break; }
        }
        assert!(got_error);
    }

    #[test]
    fn tokenise_splits_on_spaces() {
        let chunks = tokenise_response("hello world");
        assert_eq!(chunks, vec!["hello ", "world"]);
    }

    #[test]
    fn tokenise_preserves_newlines() {
        let chunks = tokenise_response("line1\nline2");
        assert!(chunks.iter().any(|c| c.contains('\n')));
    }

    #[test]
    fn tokenise_empty_string() {
        let chunks = tokenise_response("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn drain_returns_finished_on_disconnect() {
        let (tx, rx) = mpsc::channel(4);
        let mut handle = StreamHandle::new(rx);
        drop(tx); // disconnect immediately
        let (chunks, finished) = handle.drain();
        assert!(chunks.is_empty());
        assert!(finished);
    }
}
