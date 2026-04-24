// The project is under active development — many public items in sub-modules
// are implemented ahead of being wired into the main application loop.
// Suppress the resulting dead_code / unused_imports noise at the binary level.
#![allow(dead_code, unused_imports)]

// ── Optional jemalloc global allocator ───────────────────────────────────────
// Enable with: cargo build --features jemalloc
// Reduces heap fragmentation and enables memory profiling with jeprof.
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod config;
mod error;
mod history;
mod ui;

// Future modules — declared here so the compiler resolves them
// as the project grows through the roadmap phases.
mod ai;
mod bench;
mod client;
mod metrics;
mod dao;
mod exec;
mod health;
mod model;
mod portforward;
mod render;
mod sanitizer;
mod util;
mod view;
mod vul;
mod watch;

use std::process;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use history::{CommandHistory, HistorySource};

/// k7s — Security-first Kubernetes TUI with AI-powered cluster analysis.
///
/// Without a subcommand, opens the interactive TUI connected to the active
/// kubeconfig context.  With a subcommand, behaves as a `kubectl`-compatible
/// CLI while recording every command to a searchable, replayable history.
///
/// Run `k7s history` to review past commands and `k7s retry [N]` to
/// re-execute them.
#[derive(Parser, Debug)]
#[command(
    name = "k7s",
    version,
    about,
    long_about = None,
    // Allow `k7s get pods` without a subcommand being mandatory.
    subcommand_required = false,
    arg_required_else_help = false,
)]
struct Cli {
    /// Kubernetes context to connect to (defaults to active kubeconfig context).
    #[arg(short = 'c', long, global = true)]
    context: Option<String>,

    /// Namespace to use (defaults to all namespaces).
    #[arg(short = 'n', long, global = true)]
    namespace: Option<String>,

    /// Disable all mutating operations (delete, scale, edit, exec).
    #[arg(long, global = true)]
    readonly: bool,

    /// Log level filter (e.g. debug, info, warn, error).
    ///
    /// Also respects the RUST_LOG environment variable.
    #[arg(short = 'l', long, default_value = "info", global = true)]
    log_level: String,

    /// Path to a custom config file (overrides XDG default location).
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    /// Write structured logs to this file instead of stderr.
    #[arg(long, global = true)]
    log_file: Option<std::path::PathBuf>,

    /// Print cluster info and exit without opening the TUI.
    #[arg(long, global = true)]
    headless: bool,

    /// Enable expert mode: automatically analyze pod failures, performance
    /// issues, and log errors with AI-powered recommendations.
    #[arg(long, global = true)]
    expert: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

// ─── Subcommands (kubectl CLI parity) ────────────────────────────────────────

#[derive(Subcommand, Debug)]
enum Commands {
    /// Display one or many resources.
    ///
    /// Equivalent to `kubectl get`.
    Get {
        /// Resource type and optional name, e.g. `pods`, `pod my-pod`.
        #[arg(required = true)]
        resource: String,
        /// Optional resource name.
        name: Option<String>,
        /// Output format (wide, yaml, json, name).
        #[arg(short = 'o', long)]
        output: Option<String>,
        /// List across all namespaces.
        #[arg(short = 'A', long)]
        all_namespaces: bool,
        /// Label selector filter, e.g. `app=nginx`.
        #[arg(short = 'l', long, id = "label")]
        selector: Option<String>,
    },

    /// Show detailed information about a resource.
    ///
    /// Equivalent to `kubectl describe`.
    Describe {
        /// Resource type, e.g. `pod`.
        resource: String,
        /// Resource name.
        name: String,
    },

    /// Delete a resource.
    ///
    /// Equivalent to `kubectl delete`.
    Delete {
        /// Resource type, e.g. `pod`.
        resource: String,
        /// Resource name.
        name: String,
        /// Graceful termination period in seconds. 0 = force delete.
        #[arg(long)]
        grace_period: Option<u64>,
        /// Skip confirmation prompt and delete immediately.
        #[arg(long, short = 'f')]
        force: bool,
    },

    /// Apply configuration from a file or stdin.
    ///
    /// Equivalent to `kubectl apply`.
    Apply {
        /// Filename, directory, or URL to apply.
        #[arg(short = 'f', long, required = true)]
        filename: String,
        /// Dry run mode: `none`, `client`, or `server`.
        #[arg(long)]
        dry_run: Option<String>,
    },

    /// Print logs of a pod or container.
    ///
    /// Equivalent to `kubectl logs`.
    Logs {
        /// Pod name (or `deployment/name`, `job/name`, etc.).
        pod: String,
        /// Container name (required for multi-container pods).
        #[arg(short = 'c', long)]
        container: Option<String>,
        /// Follow log output (stream).
        #[arg(short = 'f', long)]
        follow: bool,
        /// Number of lines to show from the tail of the logs.
        #[arg(long)]
        tail: Option<i64>,
        /// Include timestamps in log output.
        #[arg(long)]
        timestamps: bool,
        /// Show logs for the previous container instance.
        #[arg(short = 'p', long)]
        previous: bool,
        /// Only return logs newer than a duration, e.g. `5s`, `2m`, `3h`.
        #[arg(long)]
        since: Option<String>,
    },

    /// Execute a command in a container.
    ///
    /// Equivalent to `kubectl exec`.
    Exec {
        /// Pod name.
        pod: String,
        /// Container name.
        #[arg(short = 'c', long)]
        container: Option<String>,
        /// Command and arguments to run inside the container.
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// Forward local ports to a pod.
    ///
    /// Equivalent to `kubectl port-forward`.
    PortForward {
        /// Pod, deployment, or service to forward to, e.g. `pod/my-pod`.
        resource: String,
        /// Port mapping(s), e.g. `8080:80` or `8080`.
        #[arg(required = true)]
        ports: Vec<String>,
    },

    /// Scale a workload to a given number of replicas.
    ///
    /// Equivalent to `kubectl scale`.
    Scale {
        /// Resource type, e.g. `deployment`.
        resource: String,
        /// Resource name.
        name: String,
        /// Desired number of replicas.
        #[arg(long, required = true)]
        replicas: u32,
    },

    /// Manage rollouts of workloads.
    ///
    /// Equivalent to `kubectl rollout`.
    Rollout {
        #[command(subcommand)]
        action: RolloutAction,
    },

    /// Display resource (CPU/memory) usage.
    ///
    /// Equivalent to `kubectl top`.
    Top {
        /// Resource type: `pods` or `nodes`.
        resource: String,
        /// Sort by: `cpu` or `memory`.
        #[arg(long)]
        sort_by: Option<String>,
    },

    /// Print client and server version information.
    ///
    /// Equivalent to `kubectl version`.
    Version {
        /// Print only the client version.
        #[arg(long)]
        client: bool,
        /// Output format (yaml, json).
        #[arg(short = 'o', long)]
        output: Option<String>,
    },

    /// Manage kubeconfig settings.
    ///
    /// Equivalent to `kubectl config`.  Passes arguments through to kubectl.
    Config {
        /// kubectl config subcommand and arguments (e.g. `get-contexts`).
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// List past k7s commands (CLI and TUI).
    ///
    /// Shows the unified history of commands entered from the CLI and
    /// actions taken inside the TUI, newest first.
    History {
        /// Maximum number of entries to display.
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
        /// Filter to CLI commands only.
        #[arg(long)]
        cli: bool,
        /// Filter to TUI actions only.
        #[arg(long)]
        tui: bool,
    },

    /// Re-execute a past command from history.
    ///
    /// `k7s retry` replays the most recent command.
    /// `k7s retry 3` replays the 3rd-most-recent command.
    Retry {
        /// Which entry to replay (1 = most recent, default).
        n: Option<usize>,
    },
}

#[derive(Subcommand, Debug)]
enum RolloutAction {
    /// Show the status of a rollout.
    Status {
        resource: String,
        name: String,
    },
    /// Perform a rolling restart of a workload.
    Restart {
        resource: String,
        name: String,
    },
    /// Roll back to the previous revision.
    Undo {
        resource: String,
        name: String,
        /// Revision to roll back to.
        #[arg(long)]
        to_revision: Option<u64>,
    },
    /// Pause a rollout.
    Pause {
        resource: String,
        name: String,
    },
    /// Resume a paused rollout.
    Resume {
        resource: String,
        name: String,
    },
    /// List rollout history.
    History {
        resource: String,
        name: String,
    },
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    init_tracing(&cli.log_level, cli.log_file.as_deref())?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "k7s starting");

    // Resolve config directories (needed for history and config loading).
    let dirs = config::ConfigDirs::resolve().map_err(error::AppError::Config)?;
    let config_path = cli.config.clone().unwrap_or_else(|| dirs.config_file());
    let mut cfg = config::load(&config_path).map_err(error::AppError::Config)?;

    // CLI flags override config file values.
    if cli.readonly {
        cfg.k7s.read_only = true;
    }
    if cli.expert {
        cfg.k7s.expert_mode = true;
    }

    tracing::debug!(
        config_file = %config_path.display(),
        readonly = cfg.k7s.read_only,
        "configuration loaded"
    );

    // ── Subcommand dispatch ──────────────────────────────────────────────────
    if let Some(cmd) = cli.command {
        let mut hist = CommandHistory::load(&dirs.state);
        return run_subcommand(cmd, &cli.context, &cli.namespace, &mut hist);
    }

    // ── Headless mode ────────────────────────────────────────────────────────
    if cli.headless {
        println!("k7s v{}", env!("CARGO_PKG_VERSION"));
        println!("Config: {}", config_path.display());
        println!("Read-only: {}", cfg.k7s.read_only);
        return Ok(());
    }

    // ── TUI mode (default) ───────────────────────────────────────────────────
    ui::run(cfg).map_err(|e| {
        tracing::error!(error = %e, "fatal error in TUI loop");
        e
    })
}

// ─── Subcommand runner ────────────────────────────────────────────────────────

/// Dispatch a parsed subcommand to the appropriate kubectl runner.
///
/// Every command is recorded in history before execution.  The function
/// returns an error only for pre-flight failures (bad arguments, history
/// load errors); kubectl failures are printed and the process exits with
/// the kubectl exit code.
fn run_subcommand(
    cmd: Commands,
    context: &Option<String>,
    namespace: &Option<String>,
    hist: &mut CommandHistory,
) -> anyhow::Result<()> {
    match cmd {
        Commands::Get {
            resource,
            name,
            output,
            all_namespaces,
            selector,
        } => {
            let mut args = vec!["get".to_owned(), resource.clone()];
            if let Some(n) = &name {
                args.push(n.clone());
            }
            if let Some(ns) = namespace {
                args.extend(["-n".into(), ns.clone()]);
            }
            if all_namespaces {
                args.push("-A".into());
            }
            if let Some(o) = &output {
                args.extend(["-o".into(), o.clone()]);
            }
            if let Some(sel) = &selector {
                args.extend(["-l".into(), sel.clone()]);
            }
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Describe { resource, name } => {
            let args = vec!["describe".to_owned(), resource, name];
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Delete {
            resource,
            name,
            grace_period,
            force,
        } => {
            let mut args = vec!["delete".to_owned(), resource, name];
            if let Some(gp) = grace_period {
                args.extend(["--grace-period".into(), gp.to_string()]);
            }
            if force {
                args.push("--force".into());
            }
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Apply { filename, dry_run } => {
            let mut args = vec!["apply".to_owned(), "-f".into(), filename];
            if let Some(dr) = dry_run {
                args.push(format!("--dry-run={dr}"));
            }
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Logs {
            pod,
            container,
            follow,
            tail,
            timestamps,
            previous,
            since,
        } => {
            let mut args = vec!["logs".to_owned(), pod];
            if let Some(c) = container {
                args.extend(["-c".into(), c]);
            }
            if follow {
                args.push("-f".into());
            }
            if let Some(t) = tail {
                args.extend(["--tail".into(), t.to_string()]);
            }
            if timestamps {
                args.push("--timestamps".into());
            }
            if previous {
                args.push("-p".into());
            }
            if let Some(s) = since {
                args.extend(["--since".into(), s]);
            }
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Exec {
            pod,
            container,
            command,
        } => {
            let mut args = vec!["exec".to_owned(), "-it".into(), pod];
            if let Some(c) = container {
                args.extend(["-c".into(), c]);
            }
            args.push("--".into());
            args.extend(command);
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::PortForward { resource, ports } => {
            let mut args = vec!["port-forward".to_owned(), resource];
            args.extend(ports);
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Scale {
            resource,
            name,
            replicas,
        } => {
            let args = vec![
                "scale".to_owned(),
                resource,
                name,
                format!("--replicas={replicas}"),
            ];
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Rollout { action } => {
            let args = build_rollout_args(action);
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Top { resource, sort_by } => {
            let mut args = vec!["top".to_owned(), resource];
            if let Some(s) = sort_by {
                args.extend(["--sort-by".into(), s]);
            }
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Version { client, output } => {
            let mut args = vec!["version".to_owned()];
            if client {
                args.push("--client".into());
            }
            if let Some(o) = output {
                args.extend(["-o".into(), o]);
            }
            run_kubectl(&args, context, namespace, hist);
        }

        Commands::Config { args } => {
            let mut full = vec!["config".to_owned()];
            full.extend(args);
            run_kubectl(&full, context, namespace, hist);
        }

        // ── History display ──────────────────────────────────────────────────
        Commands::History { limit, cli, tui } => {
            let entries = hist.recent(limit);
            if entries.is_empty() {
                println!("No history yet.");
                return Ok(());
            }

            println!("{:<6} {:<5} {:<25} COMMAND", "ID", "SRC", "WHEN");
            println!("{}", "─".repeat(70));
            for entry in entries {
                // Skip filtered sources.
                if cli && entry.source != HistorySource::Cli {
                    continue;
                }
                if tui && entry.source != HistorySource::Tui {
                    continue;
                }
                let when = entry.timestamp.format("%Y-%m-%d %H:%M:%S");
                let src = entry.source.to_string();
                let status = if entry.success { "" } else { " [FAILED]" };
                println!(
                    "{:<6} {:<5} {:<25} {}{}",
                    entry.id, src, when, entry.command, status
                );
            }
        }

        // ── Retry ────────────────────────────────────────────────────────────
        Commands::Retry { n } => {
            let n = n.unwrap_or(1);
            match hist.nth_last(n) {
                None => {
                    eprintln!(
                        "k7s: no history entry at position {n} \
                         (only {} entries recorded)",
                        hist.len()
                    );
                    process::exit(1);
                }
                Some(entry) => {
                    let command = entry.command.clone();
                    let source = entry.source.clone();

                    // TUI actions cannot be replayed from the CLI — they need the TUI
                    // to be running.  Warn and exit rather than silently doing nothing.
                    if source == HistorySource::Tui {
                        eprintln!(
                            "k7s: entry {n} is a TUI action (\"{command}\").\n\
                             Use `:retry {n}` inside the k7s TUI to replay it."
                        );
                        process::exit(1);
                    }

                    eprintln!("k7s retry: {command}");

                    // Reconstruct argv from the recorded command string and re-invoke
                    // kubectl directly (the history stores `"get pods -n default"` style).
                    let parts: Vec<&str> = command.split_whitespace().collect();
                    replay_kubectl(&parts, context, namespace, hist);
                }
            }
        }
    }

    Ok(())
}

// ─── kubectl helpers ──────────────────────────────────────────────────────────

/// Build the full `kubectl` argument list from `args` and optional global
/// flags (`--context`, `-n`), run the process, record it in history, and
/// exit with kubectl's exit code if it is non-zero.
fn run_kubectl(
    args: &[String],
    context: &Option<String>,
    namespace: &Option<String>,
    hist: &mut CommandHistory,
) {
    let mut full: Vec<String> = Vec::new();

    if let Some(ctx) = context {
        full.extend(["--context".into(), ctx.clone()]);
    }
    if let Some(ns) = namespace {
        // Only prepend -n when the subcommand doesn't already include it.
        // (Some callers like `logs` append it themselves.)
        let already_has_n = args.windows(2).any(|w| w[0] == "-n");
        if !already_has_n {
            full.extend(["-n".into(), ns.clone()]);
        }
    }

    full.extend_from_slice(args);

    let command_str = full.join(" ");
    tracing::info!(command = %command_str, "kubectl parity: running");

    let status = std::process::Command::new("kubectl")
        .args(&full)
        .status();

    let (exit_code, success) = match status {
        Ok(s) => (s.code().unwrap_or(1), s.success()),
        Err(e) => {
            eprintln!("k7s: could not run kubectl: {e}");
            (127, false)
        }
    };

    hist.push(
        HistorySource::Cli,
        &command_str,
        context.clone(),
        namespace.clone(),
        success,
    );

    if exit_code != 0 {
        process::exit(exit_code);
    }
}

/// Replay a raw command string (e.g. `"get pods -n default"`) by passing
/// its tokens directly to kubectl, then record the replay in history.
fn replay_kubectl(
    tokens: &[&str],
    context: &Option<String>,
    namespace: &Option<String>,
    hist: &mut CommandHistory,
) {
    let mut full: Vec<String> = Vec::new();
    if let Some(ctx) = context {
        full.extend(["--context".into(), ctx.clone()]);
    }
    full.extend(tokens.iter().map(|s| s.to_string()));

    let command_str = full.join(" ");
    tracing::info!(command = %command_str, "kubectl parity: replaying");

    let status = std::process::Command::new("kubectl")
        .args(&full)
        .status();

    let (exit_code, success) = match status {
        Ok(s) => (s.code().unwrap_or(1), s.success()),
        Err(e) => {
            eprintln!("k7s: could not run kubectl: {e}");
            (127, false)
        }
    };

    hist.push(
        HistorySource::Cli,
        format!("(retry) {command_str}"),
        context.clone(),
        namespace.clone(),
        success,
    );

    if exit_code != 0 {
        process::exit(exit_code);
    }
}

/// Build the `kubectl rollout <action> <resource> <name>` argument list.
fn build_rollout_args(action: RolloutAction) -> Vec<String> {
    match action {
        RolloutAction::Status { resource, name } => {
            vec!["rollout".into(), "status".into(), resource, name]
        }
        RolloutAction::Restart { resource, name } => {
            vec!["rollout".into(), "restart".into(), resource, name]
        }
        RolloutAction::Undo {
            resource,
            name,
            to_revision,
        } => {
            let mut args = vec!["rollout".into(), "undo".into(), resource, name];
            if let Some(rev) = to_revision {
                args.push(format!("--to-revision={rev}"));
            }
            args
        }
        RolloutAction::Pause { resource, name } => {
            vec!["rollout".into(), "pause".into(), resource, name]
        }
        RolloutAction::Resume { resource, name } => {
            vec!["rollout".into(), "resume".into(), resource, name]
        }
        RolloutAction::History { resource, name } => {
            vec!["rollout".into(), "history".into(), resource, name]
        }
    }
}

// ─── Tracing init ─────────────────────────────────────────────────────────────

fn init_tracing(level: &str, log_file: Option<&std::path::Path>) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    // The TUI owns the terminal, so log to a file or discard to avoid
    // corrupting the screen. Only write to stderr when no TUI is running
    // (e.g. headless mode or --log-file).
    if let Some(path) = log_file {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(file)
            .with_ansi(false)
            .init();
    } else {
        // In TUI mode we still initialise a subscriber but direct it to
        // /dev/null — structured logs are written to the state dir file
        // when an explicit --log-file is given.
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::sink)
            .init();
    }

    Ok(())
}
