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

pub mod config_map;
pub mod container;
pub mod context;
pub mod crd;
pub mod event;
pub mod cron_job;
pub mod daemon_set;
pub mod deployment;
pub mod generic;
pub mod helm;
pub mod job;
pub mod namespace;
pub mod node;
pub mod ops;
pub mod pod;
pub mod rbac;
pub mod registry;
pub mod replica_set;
pub mod secret;
pub mod service;
pub mod stateful_set;
pub mod traits;

pub use config_map::ConfigMapDao;
pub use container::{container_gvr, containers_from_pod, ContainerInfo, ContainerKind};
pub use context::{ContextDao, ContextEntry};
pub use crd::{discover_crds, CrdMeta};
pub use cron_job::CronJobDao;
pub use daemon_set::DaemonSetDao;
pub use helm::{HelmDao, HelmError, HelmHistoryEntry, HelmRelease};
pub use job::JobDao;
pub use namespace::NamespaceDao;
pub use node::NodeDao;
pub use rbac::{
    ClusterRoleBindingDao, ClusterRoleDao, PolicyRule, RoleBindingDao, RoleDao, Subject,
    SubjectKind,
};
pub use registry::Registry;
pub use replica_set::ReplicaSetDao;
pub use secret::SecretDao;
pub use service::ServiceDao;
pub use stateful_set::StatefulSetDao;
pub use traits::{Accessor, Describer, Loggable, Nuker, Restartable, Scalable};
