//! Screen-level TUI views (pod list, deploy list, log viewer, chat).
//!
//! Phase 6 of the k7s roadmap.

pub mod actions;
pub mod browser;
pub mod describe;
pub mod dir;
pub mod expert;
pub mod help;
pub mod log;
pub mod metrics;
pub mod owner;
pub mod pulse;
pub mod reference;
pub mod registrar;
pub mod workload;
pub mod xray;

pub use browser::{
    alias_browser, browser_for_resource, container_browser, context_browser, BrowserView,
};
pub use describe::DescribeView;
pub use dir::{DirAction, DirEntry, DirView, EntryKind};
pub use help::{HelpAction, HelpView};
pub use log::{LogAction, LogView};
pub use metrics::{MetricsAction, MetricsView};
pub use owner::{controller_owner, gvr_for_kind, resolve_owners, OwnerRef};
pub use pulse::{PulseAction, PulseView};
pub use reference::{scan_pods_for_refs, ReferenceKind, UsageRef, UsedByView};
pub use registrar::{view_for, ViewEntry};
pub use workload::{build_workload_data, WorkloadAction, WorkloadEntry, WorkloadView};
pub use expert::{
    build_expert_prompt, ExpertAction, ExpertAlert, ExpertPanel, FailureDetector,
};
pub use xray::{build_xray_tree, demo_tree, NodeStatus, XRayAction, XRayNode, XRayView};
