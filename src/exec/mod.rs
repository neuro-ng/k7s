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
    pub command: String,
}

/// Parameters for a `kubectl exec` invocation.
#[derive(Debug, Clone)]
pub struct ShellExec {
    /// Pod name.
    pub pod: String,
    /// Namespace.
    pub namespace: String,
    /// Container name. When `None`, kubectl uses the first container.
    pub container: Option<String>,
    /// Shell command to run inside the container.
    ///
    /// Defaults to trying `/bin/bash` then `/bin/sh`.
    pub shell: Option<String>,
    /// Optional kubeconfig context to pass through.
    pub context: Option<String>,
}

impl ShellExec {
    pub fn new(pod: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            pod: pod.into(),
            namespace: namespace.into(),
            container: None,
            shell: None,
            context: None,
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
            "-n".to_owned(),
            self.namespace.clone(),
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

        let status = Command::new("kubectl").args(&args).status();

        let exit_code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                tracing::error!(error = %e, "kubectl exec failed to launch");
                None
            }
        };

        ExecResult {
            exit_code,
            command: command_str,
        }
    }
}

// ─── ContainerAttach ─────────────────────────────────────────────────────────

/// Attach to a running container's stdin/stdout/stderr.
///
/// Equivalent to `kubectl attach -it <pod> -c <container>`.
///
/// Attaching differs from exec in that it connects to the *already-running*
/// process (PID 1 or the main entrypoint) rather than launching a new shell.
/// Useful for interactive processes and CLIs that don't spawn a shell.
///
/// Same TUI suspension precondition as `ShellExec::run()`.
///
/// # k9s Reference: `internal/view/exec.go` — `runAttach()`
pub struct ContainerAttach {
    /// Pod name.
    pub pod: String,
    /// Namespace.
    pub namespace: String,
    /// Container name.  Required when the pod has more than one container.
    pub container: Option<String>,
    /// Optional kubeconfig context.
    pub context: Option<String>,
}

impl ContainerAttach {
    pub fn new(pod: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            pod: pod.into(),
            namespace: namespace.into(),
            container: None,
            context: None,
        }
    }

    pub fn container(mut self, c: impl Into<String>) -> Self {
        self.container = Some(c.into());
        self
    }

    pub fn context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    /// Attach to the container.
    ///
    /// # Precondition
    ///
    /// TUI must have disabled raw mode and left the alternate screen first.
    pub fn run(&self) -> ExecResult {
        let mut args = vec![
            "attach".to_owned(),
            "-it".to_owned(),
            "-n".to_owned(),
            self.namespace.clone(),
        ];

        if let Some(ctx) = &self.context {
            args.extend(["--context".to_owned(), ctx.clone()]);
        }
        if let Some(c) = &self.container {
            args.extend(["-c".to_owned(), c.clone()]);
        }
        args.push(self.pod.clone());

        let command_str = format!("kubectl {}", args.join(" "));
        tracing::info!(command = %command_str, "attaching to container");

        let status = Command::new("kubectl").args(&args).status();
        let exit_code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                tracing::error!(error = %e, "kubectl attach failed to launch");
                None
            }
        };
        ExecResult {
            exit_code,
            command: command_str,
        }
    }
}

// ─── NodeShell ────────────────────────────────────────────────────────────────

/// Launch a debug shell pod on a Kubernetes node.
///
/// Equivalent to `kubectl debug node/<node> -it --image=<image>`.
///
/// This creates a temporary privileged pod that runs on the specified node,
/// giving access to the node's filesystem and processes.
///
/// Same TUI suspension precondition as `ShellExec::run()`.
///
/// # k9s Reference: `internal/config/shell_pod.go`
pub struct NodeShell {
    /// Node name to exec onto.
    pub node: String,
    /// Debug image to use.  Defaults to `busybox`.
    pub image: String,
    /// Optional kubeconfig context.
    pub context: Option<String>,
}

impl NodeShell {
    pub fn new(node: impl Into<String>) -> Self {
        Self {
            node: node.into(),
            image: "busybox".to_owned(),
            context: None,
        }
    }

    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    pub fn context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    /// Spawn the debug pod and attach to it.
    ///
    /// # Precondition
    ///
    /// TUI must have disabled raw mode and left the alternate screen first.
    pub fn run(&self) -> ExecResult {
        let mut args = vec![
            "debug".to_owned(),
            format!("node/{}", self.node),
            "-it".to_owned(),
            format!("--image={}", self.image),
        ];

        if let Some(ctx) = &self.context {
            args.extend(["--context".to_owned(), ctx.clone()]);
        }

        let command_str = format!("kubectl {}", args.join(" "));
        tracing::info!(command = %command_str, "launching node shell");

        let status = Command::new("kubectl").args(&args).status();
        let exit_code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                tracing::error!(error = %e, "kubectl debug node failed to launch");
                None
            }
        };
        ExecResult {
            exit_code,
            command: command_str,
        }
    }
}

// ─── ImageUpdate ─────────────────────────────────────────────────────────────

/// Update a container image on a workload.
///
/// Equivalent to `kubectl set image <resource>/<name> <container>=<image>`.
///
/// Works for Deployments, StatefulSets, DaemonSets, etc.
///
/// # k9s Reference: `internal/view/image_extender.go`
pub struct ImageUpdate {
    /// Resource kind, e.g. `"deployment"`.
    pub resource: String,
    /// Resource name.
    pub name: String,
    /// Namespace.
    pub namespace: String,
    /// Container name to update.
    pub container: String,
    /// New image reference.
    pub image: String,
    /// Optional kubeconfig context.
    pub context: Option<String>,
}

impl ImageUpdate {
    pub fn new(
        resource: impl Into<String>,
        name: impl Into<String>,
        namespace: impl Into<String>,
        container: impl Into<String>,
        image: impl Into<String>,
    ) -> Self {
        Self {
            resource: resource.into(),
            name: name.into(),
            namespace: namespace.into(),
            container: container.into(),
            image: image.into(),
            context: None,
        }
    }

    pub fn context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    /// Apply the image update synchronously.
    ///
    /// Unlike `ShellExec`, this does **not** require TUI suspension — kubectl
    /// completes without user interaction.
    pub fn run(&self) -> ExecResult {
        let mut args = vec![
            "set".to_owned(),
            "image".to_owned(),
            "-n".to_owned(),
            self.namespace.clone(),
        ];

        if let Some(ctx) = &self.context {
            args.extend(["--context".to_owned(), ctx.clone()]);
        }

        args.push(format!("{}/{}", self.resource, self.name));
        args.push(format!("{}={}", self.container, self.image));

        let command_str = format!("kubectl {}", args.join(" "));
        tracing::info!(command = %command_str, "updating container image");

        let status = Command::new("kubectl").args(&args).status();
        let exit_code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                tracing::error!(error = %e, "kubectl set image failed");
                None
            }
        };
        ExecResult {
            exit_code,
            command: command_str,
        }
    }
}

/// Open an editor on a Kubernetes resource manifest.
///
/// Uses `kubectl edit`, which handles fetching, launching `$EDITOR`, and
/// applying the changes.
///
/// Same TUI suspension precondition as `ShellExec::run()`.
pub struct KubectlEdit {
    pub resource: String,
    pub name: String,
    pub namespace: Option<String>,
    pub context: Option<String>,
}

impl KubectlEdit {
    pub fn new(resource: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            name: name.into(),
            namespace: None,
            context: None,
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

        ExecResult {
            exit_code,
            command: command_str,
        }
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

    #[test]
    fn container_attach_builder() {
        let attach = ContainerAttach::new("my-pod", "prod")
            .container("sidecar")
            .context("prod-ctx");

        assert_eq!(attach.pod, "my-pod");
        assert_eq!(attach.namespace, "prod");
        assert_eq!(attach.container.as_deref(), Some("sidecar"));
        assert_eq!(attach.context.as_deref(), Some("prod-ctx"));
    }

    #[test]
    fn node_shell_defaults_to_busybox() {
        let ns = NodeShell::new("node-1");
        assert_eq!(ns.node, "node-1");
        assert_eq!(ns.image, "busybox");
        assert!(ns.context.is_none());
    }

    #[test]
    fn node_shell_custom_image() {
        let ns = NodeShell::new("node-1").image("alpine:3.19");
        assert_eq!(ns.image, "alpine:3.19");
    }

    #[test]
    fn image_update_builder() {
        let iu =
            ImageUpdate::new("deployment", "api", "prod", "app", "nginx:1.27").context("prod-ctx");
        assert_eq!(iu.resource, "deployment");
        assert_eq!(iu.name, "api");
        assert_eq!(iu.namespace, "prod");
        assert_eq!(iu.container, "app");
        assert_eq!(iu.image, "nginx:1.27");
        assert_eq!(iu.context.as_deref(), Some("prod-ctx"));
    }
}
