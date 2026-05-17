//! Cluster Metadata Store — Phase 37.
//!
//! Maintains a local, dated, per-cluster journal in
//! `~/.local/state/k7s/cluster-metadata/<context>/` as append-only
//! JSON-lines daily files.  Three record types are stored:
//!
//! - **Snapshot** — cluster node/workload counts at session start
//! - **Issue** — expert-scan alerts and event-watcher anomalies
//! - **Interaction** — operator actions (delete, scale, port-forward, AI chat, …)
//!
//! The `history` module aggregates recent records into a `ClusterHistory`
//! summary that the AI context builder injects as a system message.

pub mod history;
pub mod record;
pub mod store;

pub use history::{summarise, ClusterHistory};
pub use record::{
    InteractionAction, InteractionRecord, IssueRecord, MetadataRecord, NodeSummary, SnapshotRecord,
    WorkloadSummary,
};
pub use store::{IndexFile, MetadataStore};
