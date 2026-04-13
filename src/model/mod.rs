//! Business logic and application state models.
//!
//! Phase 2+ of the k7s roadmap.

pub mod history;
pub mod log;

pub use history::NavHistory;
pub use log::{merge_container_logs, LogItem, LogLevel, LogModel};
