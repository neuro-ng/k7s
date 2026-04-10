//! Shell exec and attach — Phase 8.1 / 8.2.
//!
//! Executing a shell into a pod requires temporarily suspending the TUI:
//! the terminal must be restored to cooked mode and the alternate screen
//! exited so the subprocess can own stdin/stdout.
//!
//! # Pattern
//!
//! 1. Caller tears down the TUI (raw mode off, leave alt screen).
//! 2. `ShellExec::run()` spawns `kubectl exec -it` and waits for it.
//! 3. Caller re-enters the TUI (raw mode on, enter alt screen, full redraw).
//!
//! This crate does not touch crossterm directly — that responsibility belongs
//! to the caller (the TUI event loop) so it can cleanly tear down and rebuild
//! the terminal in the right order.

use std::process::Command;

/// Result of a shell exec invocation.
#[derive(Debug)]
pub struct ExecResult {
    /// Exit code of the subprocess, or `None` if the process could not be started.
    pub exit_code: Option<i32>,
    /// Command that was run (for display / logging).
    pub command:   String,
}

/// Parameters for a `kubectl exec` invocation.
#[derive(Debug, Clone)]
pub struct ShellExec {
    /// Pod name.
    pub pod:       String,
    /// Namespace.
    pub namespace: String,
    /// Container name. When `None`, kubectl uses the first container.
    pub container: Option<String>,
    /// Shell command to run inside the container.
    ///
    /// Defaults to trying `/bin/bash` then `/bin/sh`.
    pub shell:     Option<String>,
    /// Optional kubeconfig context to pass through.
    pub context:   Option<String>,
}

impl ShellExec {
    pub fn new(pod: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            pod:       pod.into(),
            namespace: namespace.into(),
            container: None,
            shell:     None,
            context:   None,
        }
    }

    pub fn container(mut self, c: impl Into<String>) -> Self {
        self.container = Some(c.into());
        self
    }

    pub fn shell(mut self, s: impl Into<String>) -> Self {
        self.shell = Some(s.into());
        self
    }

    pub fn context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    /// Run the exec.
    ///
    /// # Precondition
    ///
    /// The TUI **must** have already disabled raw mode and left the alternate
    /// screen before calling this method.  If not, the subprocess output will
    /// corrupt the terminal state.
    pub fn run(&self) -> ExecResult {
        let shell = self.shell.as_deref().unwrap_or("/bin/sh");

        let mut args = vec![
            "exec".to_owned(),
            "-it".to_owned(),
            "-n".to_owned(), self.namespace.clone(),
        ];

        if let Some(ctx) = &self.context {
            args.extend(["--context".to_owned(), ctx.clone()]);
        }

        if let Some(c) = &self.container {
            args.extend(["-c".to_owned(), c.clone()]);
        }

        args.push(self.pod.clone());
        args.extend(["--".to_owned(), shell.to_owned()]);

        let command_str = format!("kubectl {}", args.join(" "));
        tracing::info!(command = %command_str, "launching shell exec");

        let status = Command::new("kubectl")
            .args(&args)
            .status();

        let exit_code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                tracing::error!(error = %e, "kubectl exec failed to launch");
                None
            }
        };

        ExecResult { exit_code, command: command_str }
    }
}

/// Open an editor on a Kubernetes resource manifest.
///
/// Uses `kubectl edit`, which handles fetching, launching `$EDITOR`, and
/// applying the changes.
///
/// Same TUI suspension precondition as `ShellExec::run()`.
pub struct KubectlEdit {
    pub resource:  String,
    pub name:      String,
    pub namespace: Option<String>,
    pub context:   Option<String>,
}

impl KubectlEdit {
    pub fn new(resource: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            resource:  resource.into(),
            name:      name.into(),
            namespace: None,
            context:   None,
        }
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    pub fn context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    pub fn run(&self) -> ExecResult {
        let mut args = vec!["edit".to_owned()];

        if let Some(ctx) = &self.context {
            args.extend(["--context".to_owned(), ctx.clone()]);
        }
        if let Some(ns) = &self.namespace {
            args.extend(["-n".to_owned(), ns.clone()]);
        }

        args.push(format!("{}/{}", self.resource, self.name));

        let command_str = format!("kubectl {}", args.join(" "));
        tracing::info!(command = %command_str, "launching kubectl edit");

        let status = Command::new("kubectl").args(&args).status();
        let exit_code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                tracing::error!(error = %e, "kubectl edit failed to launch");
                None
            }
        };

        ExecResult { exit_code, command: command_str }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_exec_builder() {
        let exec = ShellExec::new("my-pod", "default")
            .container("app")
            .shell("/bin/bash")
            .context("prod-ctx");

        assert_eq!(exec.pod, "my-pod");
        assert_eq!(exec.namespace, "default");
        assert_eq!(exec.container.as_deref(), Some("app"));
        assert_eq!(exec.shell.as_deref(), Some("/bin/bash"));
        assert_eq!(exec.context.as_deref(), Some("prod-ctx"));
    }

    #[test]
    fn kubectl_edit_builder() {
        let edit = KubectlEdit::new("deployment", "my-deploy")
            .namespace("staging")
            .context("dev");

        assert_eq!(edit.resource, "deployment");
        assert_eq!(edit.name, "my-deploy");
        assert_eq!(edit.namespace.as_deref(), Some("staging"));
    }
}
