//! Screen-level TUI views (pod list, deploy list, log viewer, chat).
//!
//! Phase 6 of the k7s roadmap.

pub mod actions;
pub mod browser;
pub mod describe;
pub mod log;
pub mod owner;
pub mod registrar;

pub use browser::{browser_for_resource, BrowserView};
pub use describe::DescribeView;
pub use log::{LogAction, LogView};
pub use owner::{controller_owner, gvr_for_kind, resolve_owners, OwnerRef};
pub use registrar::{view_for, ViewEntry};
