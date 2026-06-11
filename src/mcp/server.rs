//! MCP server state and handler factory helpers.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use rs_fast_mcp::error::FastMCPError;
use rs_fast_mcp::mcp::types::ContentBlock;
use rs_fast_mcp::server::context::Context;
use rs_fast_mcp::tools::tool::{ToolHandler, ToolResult};

use crate::config::SanitizerConfig;

// Re-export TextContent so tools.rs can use it without the full path.
pub use rs_fast_mcp::mcp::types::TextContent;

/// Shared state injected into every MCP tool and resource handler.
///
/// Wrapped in `Arc` so closures can cheaply clone it across async calls.
pub struct McpState {
    /// Live Kubernetes API client.
    pub client: kube::Client,
    /// Sanitizer configuration (custom regex patterns, strict mode flag).
    pub sanitizer_cfg: SanitizerConfig,
    /// Whether mutating tools (scale, rollout-restart) are enabled.
    pub allow_mutations: bool,
    /// Cluster context name used to locate the metadata journal.
    pub meta_context: String,
}

impl McpState {
    pub fn new(
        client: kube::Client,
        sanitizer_cfg: SanitizerConfig,
        allow_mutations: bool,
        meta_context: String,
    ) -> Self {
        Self {
            client,
            sanitizer_cfg,
            allow_mutations,
            meta_context,
        }
    }
}

// ─── Handler factory ─────────────────────────────────────────────────────────

/// Wrap an async closure that borrows `Arc<McpState>` into a `ToolHandler`.
///
/// The closure receives the shared state and the raw JSON arguments; it returns
/// a `ToolResult` or a `FastMCPError`.  The state is cloned once per call, so
/// the closure only needs `Fn` (not `FnOnce`).
pub fn make_handler<F, Fut>(state: Arc<McpState>, f: F) -> ToolHandler
where
    F: Fn(Arc<McpState>, serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<ToolResult, FastMCPError>> + Send + 'static,
{
    Box::new(move |_ctx: Context, args: serde_json::Value| {
        let state = Arc::clone(&state);
        let fut = f(Arc::clone(&state), args);
        Box::pin(fut) as Pin<Box<dyn Future<Output = Result<ToolResult, FastMCPError>> + Send>>
    })
}

// ─── Result helpers ───────────────────────────────────────────────────────────

/// Build a `ToolResult` containing a single plain-text block.
pub fn text_result(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: vec![ContentBlock::Text(TextContent {
            type_: "text".to_string(),
            text: text.into(),
            annotations: None,
        })],
        structured_content: None,
    }
}

/// Build a `ToolResult` with a JSON body (text + structured_content).
pub fn json_result(value: serde_json::Value) -> ToolResult {
    ToolResult {
        content: vec![ContentBlock::Text(TextContent {
            type_: "text".to_string(),
            text: serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            annotations: None,
        })],
        structured_content: Some(value),
    }
}

/// Convert an `anyhow::Error` into a `FastMCPError`.
pub fn anyhow_to_mcp(e: anyhow::Error) -> FastMCPError {
    FastMCPError::new(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_result_has_one_content_block() {
        let r = text_result("hello");
        assert_eq!(r.content.len(), 1);
        match &r.content[0] {
            ContentBlock::Text(t) => assert_eq!(t.text, "hello"),
            _ => panic!("expected Text block"),
        }
    }

    #[test]
    fn json_result_sets_structured_content() {
        let v = serde_json::json!({"key": "value"});
        let r = json_result(v.clone());
        assert_eq!(r.structured_content, Some(v));
    }
}
