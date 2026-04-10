//! Clipboard support — Phase 10.3.
//!
//! Copies text to the system clipboard by delegating to the available
//! platform command (`xclip`, `xsel`, `wl-copy`, `pbcopy`).  This avoids
//! pulling in a clipboard C-FFI crate and works inside containers where
//! native clipboard support may be absent.
//!
//! # k9s Reference: `internal/view/clipboard.go`

use std::io::Write;
use std::process::{Command, Stdio};

/// Which backend was used to copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardBackend {
    Xclip,
    Xsel,
    WlCopy,
    Pbcopy,
}

impl std::fmt::Display for ClipboardBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardBackend::Xclip  => write!(f, "xclip"),
            ClipboardBackend::Xsel   => write!(f, "xsel"),
            ClipboardBackend::WlCopy => write!(f, "wl-copy"),
            ClipboardBackend::Pbcopy => write!(f, "pbcopy"),
        }
    }
}

/// Error returned when no clipboard backend is available or copying fails.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("no clipboard backend found (install xclip, xsel, wl-clipboard, or pbcopy)")]
    NoBackend,
    #[error("clipboard backend '{backend}' failed: {message}")]
    BackendFailed {
        backend: ClipboardBackend,
        message: String,
    },
    #[error("I/O error writing to clipboard process: {0}")]
    Io(#[from] std::io::Error),
}

/// Copy `text` to the system clipboard.
///
/// Tries each backend in order and returns the one that succeeded.
pub fn copy(text: &str) -> Result<ClipboardBackend, ClipboardError> {
    let candidates: &[(&str, &[&str], ClipboardBackend)] = &[
        ("pbcopy",   &[],                            ClipboardBackend::Pbcopy),
        ("wl-copy",  &[],                            ClipboardBackend::WlCopy),
        ("xclip",    &["-selection", "clipboard"],   ClipboardBackend::Xclip),
        ("xsel",     &["--clipboard", "--input"],    ClipboardBackend::Xsel),
    ];

    for (bin, args, backend) in candidates {
        match try_copy(bin, args, text) {
            Ok(true)  => return Ok(backend.clone()),
            Ok(false) => {}          // binary present but command failed
            Err(_)    => {}          // binary not found — try next
        }
    }

    Err(ClipboardError::NoBackend)
}

/// Attempt to copy via a single backend.
///
/// Returns `Ok(true)` on success, `Ok(false)` if the process exited non-zero,
/// `Err` if the binary was not found.
fn try_copy(bin: &str, args: &[&str], text: &str) -> Result<bool, ClipboardError> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            // ENOENT means the binary is not installed — caller handles it.
            ClipboardError::Io(e)
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    Ok(status.success())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_display() {
        assert_eq!(ClipboardBackend::Xclip.to_string(),  "xclip");
        assert_eq!(ClipboardBackend::WlCopy.to_string(), "wl-copy");
        assert_eq!(ClipboardBackend::Pbcopy.to_string(), "pbcopy");
    }

    /// In CI/headless environments no clipboard backend is present.
    /// Verify we get `NoBackend` rather than a panic.
    #[test]
    fn no_backend_returns_error_in_headless() {
        // If a real backend IS present this test still passes — it just
        // succeeds instead of returning NoBackend.  The important thing is
        // the function does not panic or block.
        let result = copy("test");
        match result {
            Ok(_)  => {} // clipboard available — fine
            Err(ClipboardError::NoBackend) => {} // expected in CI
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
