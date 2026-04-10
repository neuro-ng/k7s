//! Data Access Objects — the k7s interface to the Kubernetes API.
//!
//! Every resource type that k7s understands implements one or more of the
//! traits defined in this module. The trait set mirrors k9s's `Accessor`,
//! `Nuker`, `Describer`, `Loggable`, `Scalable`, and `Restartable`.
//!
//! # Design
//!
//! Traits are small and focused (single responsibility).  A DAO for a
//! read-only resource implements only `Accessor + Describer`.  A DAO for
//! a `Deployment` additionally implements `Scalable + Restartable`.
//!
//! All async operations take a `kube::Client` reference — DAOs themselves
//! are stateless and cheap to construct.

pub mod traits;
pub mod registry;
pub mod generic;
pub mod pod;
pub mod deployment;
pub mod stateful_set;
pub mod daemon_set;
pub mod replica_set;
pub mod job;
pub mod cron_job;
pub mod helm;

pub use traits::{Accessor, Describer, Loggable, Nuker, Restartable, Scalable};
pub use registry::Registry;
pub use stateful_set::StatefulSetDao;
pub use daemon_set::DaemonSetDao;
pub use replica_set::ReplicaSetDao;
pub use job::JobDao;
pub use cron_job::CronJobDao;
pub use helm::{HelmDao, HelmError, HelmHistoryEntry, HelmRelease};
