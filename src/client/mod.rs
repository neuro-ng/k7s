//! Kubernetes client, connection management, GVR abstractions.
//!
//! Phase 1 of the k7s roadmap.

pub mod config;
pub mod gvr;
pub mod metrics;
pub mod rbac;
pub mod retry;

pub use config::ClientConfig;
pub use gvr::Gvr;
