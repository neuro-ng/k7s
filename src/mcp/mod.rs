//! MCP (Model Context Protocol) server — Phase 40.
//!
//! Exposes k7s cluster access as an MCP server using `rs-fast-mcp`, allowing any
//! MCP-compatible AI client (Claude Desktop, Cursor, Codeium, etc.) to interact
//! with the Kubernetes cluster through k7s's sanitizer security layer.
//!
//! Every tool response passes through the existing sanitizer pipeline — no secrets
//! or raw credentials ever leave the cluster boundary.
//!
//! # Transports
//!
//! - **`stdio`** (default) — for Claude Desktop and other process-hosted MCP clients.
//! - **`http`** — for remote agents, CI pipelines, and multi-client scenarios.
//!
//! # Quick start (Claude Desktop)
//!
//! Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:
//! ```json
//! {
//!   "mcpServers": {
//!     "k7s": {
//!       "command": "k7s",
//!       "args": ["mcp"]
//!     }
//!   }
//! }
//! ```

pub mod resources;
pub mod server;
pub mod tools;

use std::sync::Arc;

use rs_fast_mcp::error::FastMCPError;
use rs_fast_mcp::server::app::Server;

use crate::config::McpConfig;

pub use server::McpState;

/// Start the MCP server using the transport specified in `cfg`.
///
/// Blocks until the client disconnects (stdio EOF) or `SIGINT`.
pub async fn run(state: Arc<McpState>, cfg: &McpConfig) -> Result<(), FastMCPError> {
    let server = build_server(state, cfg)?;
    server.run().await
}

/// Build and configure the MCP server without running it.
///
/// Useful for testing or embedding in a larger application.
pub fn build_server(state: Arc<McpState>, cfg: &McpConfig) -> Result<Server, FastMCPError> {
    let server = match cfg.transport.as_str() {
        "http" => Server::builder("k7s", env!("CARGO_PKG_VERSION"))
            .http("0.0.0.0", cfg.port)
            .build(),
        _ => Server::builder("k7s", env!("CARGO_PKG_VERSION"))
            .stdio()
            .build(),
    };

    tools::register_all(&server.core, state.clone())?;
    resources::register_all(&server.core, state)?;

    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpConfig;

    #[test]
    fn default_transport_is_stdio() {
        let cfg = McpConfig::default();
        assert_eq!(cfg.transport, "stdio");
    }

    #[test]
    fn default_port_is_3000() {
        let cfg = McpConfig::default();
        assert_eq!(cfg.port, 3000);
    }

    #[test]
    fn mutations_disabled_by_default() {
        let cfg = McpConfig::default();
        assert!(!cfg.allow_mutations);
    }
}
