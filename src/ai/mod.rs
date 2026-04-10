//! AI integration layer — LLM providers, chat sessions, token budgeting.
//!
//! Phase 12 of the k7s roadmap.
//!
//! # Security
//!
//! All data entering this module MUST have already passed through the
//! sanitizer layer (`crate::sanitizer`). There are no exceptions.

pub mod api_client;
pub mod prompt;
pub mod provider;
pub mod session;
pub mod streaming;
pub mod token_budget;

pub use prompt::{build as build_prompt, PromptKind};
pub use provider::{Message, Provider, Role};
pub use session::ChatSession;
pub use streaming::{send_streaming, stream_complete, StreamChunk, StreamHandle, StreamingSend};
pub use token_budget::TokenBudget;
