//! Reactive configuration watcher — Phase 9.10.
//!
//! Watches the k7s config file on disk and sends a reload signal whenever the
//! file is created or modified.  The signal is a unit value sent over a tokio
//! mpsc channel; the app calls [`crate::config::load`] to get the fresh config.
//!
//! The returned [`ConfigWatcher`] must be kept alive for as long as watching is
//! desired — dropping it stops the watcher.
//!
//! # Example
//!
//! ```no_run
//! use k7s::config::watcher::ConfigWatcher;
//! use std::path::Path;
//!
//! let (watcher, mut rx) = ConfigWatcher::new(Path::new("/path/to/config.yaml")).unwrap();
//! tokio::spawn(async move {
//!     while rx.recv().await.is_some() {
//!         // reload config
//!     }
//! });
//! ```

use std::path::{Path, PathBuf};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

/// A live config-file watcher.  Sending stops when this is dropped.
pub struct ConfigWatcher {
    // The underlying notify watcher — must stay alive to keep watching.
    _watcher: RecommendedWatcher,
}

impl ConfigWatcher {
    /// Start watching `path` for changes.
    ///
    /// Returns a handle that must be kept alive, and a receiver that fires `()`
    /// whenever the file is created or modified.
    ///
    /// If the path does not exist yet the watcher watches the **parent
    /// directory** and filters for the file name — this covers the case where
    /// the config file is written for the first time after startup.
    pub fn new(path: &Path) -> anyhow::Result<(Self, mpsc::Receiver<()>)> {
        let (tx, rx) = mpsc::channel::<()>(4);

        let watch_path: PathBuf;
        let file_name: Option<std::ffi::OsString>;

        if path.exists() {
            watch_path = path.to_owned();
            file_name = None;
        } else {
            // Watch the parent directory instead.
            watch_path = path.parent().unwrap_or(Path::new(".")).to_owned();
            file_name = path.file_name().map(|n| n.to_owned());
        }

        let target = path.to_owned();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(event) = res else { return };

            // Filter to create/modify events only.
            let relevant = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_));
            if !relevant {
                return;
            }

            // If watching a directory, restrict to the specific file.
            if let Some(ref name) = file_name {
                if !event.paths.iter().any(|p| p.file_name() == Some(name)) {
                    return;
                }
            }

            tracing::debug!(path = %target.display(), "config file changed — sending reload signal");
            let _ = tx.blocking_send(());
        })?;

        watcher.watch(&watch_path, RecursiveMode::NonRecursive)?;

        Ok((Self { _watcher: watcher }, rx))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[tokio::test]
    async fn watch_detects_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");

        // Create file before starting watcher.
        std::fs::write(&path, "k7s:\n  refreshRate: 2\n").unwrap();

        let (_watcher, mut rx) = ConfigWatcher::new(&path).unwrap();

        // Modify the file.
        {
            let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(b"k7s:\n  refreshRate: 5\n").unwrap();
            f.flush().unwrap();
        }

        // Allow the OS / notify a moment to propagate the event.
        tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for config-change event")
            .expect("channel closed unexpectedly");
    }
}
