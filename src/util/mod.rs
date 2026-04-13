//! Utility helpers — Phase 10.
//!
//! Modules in this crate are standalone utilities that do not depend on the
//! Kubernetes client or TUI layer.

pub mod clipboard;
pub mod fuzzy;
pub mod screen_dump;

pub use clipboard::{copy as clipboard_copy, ClipboardBackend, ClipboardError};
pub use fuzzy::{fuzzy_match, FuzzyMatch};
pub use screen_dump::{default_path, dump, DumpFormat, TableSnapshot};
