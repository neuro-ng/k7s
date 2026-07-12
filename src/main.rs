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
mod dao;
mod exec;
mod health;
mod mcp;
mod meta;
mod metrics;
mod model;
mod portforward;
mod render;
mod sanitizer;
mod util;
mod vela;
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

    /// Manage the cluster metadata journal.
    ///
    /// k7s silently journals cluster snapshots, expert-scan issues, and
    /// operator interactions to `~/.local/state/k7s/cluster-metadata/<context>/`
    /// as dated JSON-lines files.  This subcommand lets you list, inspect,
    /// prune, and export that journal from the CLI.
    Meta {
        #[command(subcommand)]
        action: MetaAction,
    },

    /// Manage Helm releases.
    ///
    /// Read operations (`list`, `status`, `history`, `values`) call the `helm` CLI.
    /// Mutating operations (`install`, `upgrade`, `rollback`, `uninstall`) also
    /// require `helm` on PATH.
    Helm {
        #[command(subcommand)]
        action: HelmCliAction,
    },

    /// Interact with KubeVela Application Platform resources.
    ///
    /// Read operations query the Kubernetes API directly.
    /// Mutating operations (restart, resume, rollback, delete) delegate to the
    /// `vela` CLI binary, which must be installed and on PATH.
    Vela {
        #[command(subcommand)]
        action: VelaCliAction,
    },

    /// Start the MCP (Model Context Protocol) server.
    ///
    /// Exposes Kubernetes cluster access as MCP tools so any MCP-compatible AI
    /// client (Claude Desktop, Cursor, etc.) can interact with the cluster through
    /// k7s's sanitizer security layer.
    ///
    /// Default transport is stdio — suitable for Claude Desktop:
    /// ```json
    /// {
    ///   "mcpServers": {
    ///     "k7s": { "command": "k7s", "args": ["mcp"] }
    ///   }
    /// }
    /// ```
    Mcp {
        /// Transport: `stdio` (default) or `http`.
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// TCP port for the HTTP transport.
        #[arg(long, default_value_t = 3000)]
        port: u16,
        /// Enable mutating tools (scale, rollout-restart).
        #[arg(long)]
        allow_mutations: bool,
    },
}

#[derive(Subcommand, Debug)]
enum RolloutAction {
    /// Show the status of a rollout.
    Status { resource: String, name: String },
    /// Perform a rolling restart of a workload.
    Restart { resource: String, name: String },
    /// Roll back to the previous revision.
    Undo {
        resource: String,
        name: String,
        /// Revision to roll back to.
        #[arg(long)]
        to_revision: Option<u64>,
    },
    /// Pause a rollout.
    Pause { resource: String, name: String },
    /// Resume a paused rollout.
    Resume { resource: String, name: String },
    /// List rollout history.
    History { resource: String, name: String },
}

/// Subcommands for `k7s meta`.
#[derive(Subcommand, Debug)]
enum MetaAction {
    /// List available dates in the metadata journal.
    ///
    /// Shows dates for which daily files exist, with a record count for each.
    List {
        /// kubeconfig context name (defaults to the active context).
        #[arg(long, short = 'c')]
        context: Option<String>,
        /// Only show the last N days.
        #[arg(long, short = 'd', default_value = "30")]
        days: u8,
        /// Filter by record type: snapshot, issue, interaction.
        #[arg(long, short = 't')]
        r#type: Option<String>,
    },

    /// Show all records for a specific date.
    Show {
        /// Date in YYYY-MM-DD format.
        date: String,
        /// kubeconfig context name (defaults to the active context).
        #[arg(long, short = 'c')]
        context: Option<String>,
    },

    /// Delete old daily files from the metadata journal.
    ///
    /// Files strictly older than `--before` are removed.
    Prune {
        /// Delete files older than this date (YYYY-MM-DD).
        #[arg(long)]
        before: Option<String>,
        /// kubeconfig context name (defaults to the active context).
        #[arg(long, short = 'c')]
        context: Option<String>,
    },

    /// Export the journal as a Markdown incident-postmortem report.
    ///
    /// Writes to stdout; redirect to a file with `> report.md`.
    Export {
        /// kubeconfig context name (defaults to the active context).
        #[arg(long, short = 'c')]
        context: Option<String>,
        /// Number of days to include.
        #[arg(long, short = 'd', default_value = "7")]
        days: u8,
        /// Output format: `json` or `markdown` (default).
        #[arg(long, short = 'o', default_value = "markdown")]
        output: String,
    },
}

/// Subcommands for `k7s helm`.
#[derive(Subcommand, Debug)]
enum HelmCliAction {
    /// List all Helm releases.
    List {
        /// Limit to this namespace; omit for all namespaces.
        #[arg(long, short = 'n')]
        namespace: Option<String>,
        /// Output format: `table` (default), `json`, or `yaml`.
        #[arg(long, short = 'o', default_value = "table")]
        output: String,
    },
    /// Show the status and metadata of a release.
    Status {
        /// Release name.
        release: String,
        /// Namespace of the release.
        #[arg(long, short = 'n', default_value = "default")]
        namespace: String,
    },
    /// Show the revision history of a release.
    History {
        /// Release name.
        release: String,
        /// Namespace of the release.
        #[arg(long, short = 'n', default_value = "default")]
        namespace: String,
        /// Maximum number of revisions to show.
        #[arg(long, default_value_t = 10)]
        max: usize,
    },
    /// Show user-supplied values for a release (secrets redacted).
    Values {
        /// Release name.
        release: String,
        /// Namespace of the release.
        #[arg(long, short = 'n', default_value = "default")]
        namespace: String,
        /// Output format: `yaml` (default) or `json`.
        #[arg(long, short = 'o', default_value = "yaml")]
        output: String,
    },
    /// Install a new Helm release.
    Install {
        /// Release name.
        name: String,
        /// Chart reference (e.g. `stable/nginx` or `./charts/myapp`).
        chart: String,
        /// Namespace to install into.
        #[arg(long, short = 'n', default_value = "default")]
        namespace: String,
        /// Values file(s) to merge.
        #[arg(long, short = 'f')]
        values: Option<String>,
        /// Set individual values (`key=value`).
        #[arg(long)]
        set: Vec<String>,
        /// Preview without installing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Upgrade an existing release to a new chart version.
    Upgrade {
        /// Release name.
        release: String,
        /// Chart reference.
        chart: String,
        /// Namespace of the release.
        #[arg(long, short = 'n', default_value = "default")]
        namespace: String,
        /// Values file(s) to merge.
        #[arg(long, short = 'f')]
        values: Option<String>,
        /// Set individual values (`key=value`).
        #[arg(long)]
        set: Vec<String>,
        /// Install the release if it does not exist.
        #[arg(long)]
        install: bool,
        /// Preview without upgrading.
        #[arg(long)]
        dry_run: bool,
    },
    /// Rollback a release to a previous revision.
    Rollback {
        /// Release name.
        release: String,
        /// Target revision (omit for previous revision).
        revision: Option<u64>,
        /// Namespace of the release.
        #[arg(long, short = 'n', default_value = "default")]
        namespace: String,
        /// Preview without rolling back.
        #[arg(long)]
        dry_run: bool,
    },
    /// Uninstall a release.
    Uninstall {
        /// Release name.
        release: String,
        /// Namespace of the release.
        #[arg(long, short = 'n', default_value = "default")]
        namespace: String,
        /// Preview without uninstalling.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Subcommands for `k7s vela`.
#[derive(Subcommand, Debug)]
enum VelaCliAction {
    /// List all KubeVela Application CRs.
    List {
        /// Output format: `table` (default), `json`, or `yaml`.
        #[arg(short = 'o', long, default_value = "table")]
        output: String,
        /// Restrict to a specific namespace.
        #[arg(short = 'n', long)]
        namespace: Option<String>,
    },

    /// Show component health and workflow phase for an application.
    Status {
        /// Application name.
        app: String,
        /// Namespace of the application.
        #[arg(short = 'n', long, default_value = "default")]
        namespace: String,
    },

    /// Print a text-art component → trait hierarchy for an application.
    Tree {
        /// Application name.
        app: String,
        /// Namespace of the application.
        #[arg(short = 'n', long, default_value = "default")]
        namespace: String,
    },

    /// Show workflow steps and their phases for an application.
    Workflow {
        /// Application name.
        app: String,
        /// Namespace of the application.
        #[arg(short = 'n', long, default_value = "default")]
        namespace: String,
    },

    /// Restart the workflow of an application (`vela workflow restart`).
    Restart {
        /// Application name.
        app: String,
        /// Namespace of the application.
        #[arg(short = 'n', long, default_value = "default")]
        namespace: String,
    },

    /// Resume a suspended workflow (`vela workflow resume`).
    Resume {
        /// Application name.
        app: String,
        /// Namespace of the application.
        #[arg(short = 'n', long, default_value = "default")]
        namespace: String,
    },

    /// Rollback an application to a previous revision (`vela workflow rollback`).
    Rollback {
        /// Application name.
        app: String,
        /// Namespace of the application.
        #[arg(short = 'n', long, default_value = "default")]
        namespace: String,
        /// Target revision number (defaults to the previous revision).
        #[arg(long)]
        revision: Option<u64>,
    },

    /// Delete an application (`vela delete`).
    Delete {
        /// Application name.
        app: String,
        /// Namespace of the application.
        #[arg(short = 'n', long, default_value = "default")]
        namespace: String,
        /// Skip interactive confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// List capability definitions (ComponentDefinition, TraitDefinition, etc.).
    Defs {
        /// Filter by type: `component` (default), `trait`, `workflowstep`, `policy`.
        #[arg(long, short = 't', default_value = "component")]
        r#type: String,
        /// Output format: `table` (default), `json`.
        #[arg(short = 'o', long, default_value = "table")]
        output: String,
    },
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    // Both kube (aws-lc-rs) and reqwest (ring) pull in rustls; without an
    // explicit provider rustls panics on first TLS use.  Install aws-lc-rs
    // (the kube-preferred provider) before any network activity.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

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
    ui::run(cfg, cli.namespace.clone()).map_err(|e| {
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
            println!("k7s version: v{}", env!("CARGO_PKG_VERSION"));
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

        // ── Meta journal ─────────────────────────────────────────────────────
        Commands::Meta { action } => {
            run_meta(action, context)?;
        }

        // ── Helm ─────────────────────────────────────────────────────────────
        Commands::Helm { action } => {
            run_helm(action)?;
        }

        // ── KubeVela ─────────────────────────────────────────────────────────
        Commands::Vela { action } => {
            run_vela(action, context, namespace)?;
        }

        // ── MCP server ───────────────────────────────────────────────────────
        Commands::Mcp {
            transport,
            port,
            allow_mutations,
        } => {
            run_mcp(context, transport, port, allow_mutations)?;
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

    let status = std::process::Command::new("kubectl").args(&full).status();

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

    let status = std::process::Command::new("kubectl").args(&full).status();

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

// ─── `k7s meta` handler ──────────────────────────────────────────────────────

/// Resolve the kubeconfig context name to use for meta operations.
///
/// Priority: `--context` CLI flag → `KUBECONFIG` active context → `"default"`.
fn resolve_meta_context(override_ctx: &Option<String>) -> String {
    if let Some(ctx) = override_ctx {
        return ctx.clone();
    }
    // Try reading the active context from kubeconfig.
    let output = std::process::Command::new("kubectl")
        .args(["config", "current-context"])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    "default".to_owned()
}

fn run_meta(action: MetaAction, global_context: &Option<String>) -> anyhow::Result<()> {
    match action {
        // ── list ──────────────────────────────────────────────────────────────
        MetaAction::List {
            context,
            days,
            r#type: type_filter,
        } => {
            let ctx = resolve_meta_context(&context.or_else(|| global_context.clone()));
            let Some(store) = meta::MetadataStore::new(&ctx) else {
                eprintln!("k7s meta: cannot open metadata store for context '{ctx}'");
                process::exit(1);
            };

            let today = chrono::Utc::now().date_naive();
            println!("Cluster metadata journal — context: {ctx}");
            println!("{}", "─".repeat(60));
            println!("{:<12} {:>8}  TYPE BREAKDOWN", "DATE", "RECORDS");
            println!("{}", "─".repeat(60));

            let mut total = 0usize;
            for i in (0..days as i64).rev() {
                let date = today - chrono::Duration::days(i);
                let records = store.load_day(date);
                if records.is_empty() {
                    continue;
                }
                let filtered: Vec<_> = if let Some(ref tf) = type_filter {
                    records
                        .iter()
                        .filter(|r| r.type_label() == tf.as_str())
                        .collect()
                } else {
                    records.iter().collect()
                };
                if filtered.is_empty() {
                    continue;
                }
                let snapshots = filtered
                    .iter()
                    .filter(|r| r.type_label() == "snapshot")
                    .count();
                let issues = filtered
                    .iter()
                    .filter(|r| r.type_label() == "issue")
                    .count();
                let actions = filtered
                    .iter()
                    .filter(|r| r.type_label() == "interaction")
                    .count();
                println!(
                    "{:<12} {:>8}  snap={snapshots} issue={issues} action={actions}",
                    date.format("%Y-%m-%d"),
                    filtered.len()
                );
                total += filtered.len();
            }
            println!("{}", "─".repeat(60));
            println!("Total: {total} records over last {days} days");
        }

        // ── show ──────────────────────────────────────────────────────────────
        MetaAction::Show { date, context } => {
            let ctx = resolve_meta_context(&context.or_else(|| global_context.clone()));
            let Some(store) = meta::MetadataStore::new(&ctx) else {
                eprintln!("k7s meta: cannot open metadata store for context '{ctx}'");
                process::exit(1);
            };

            let parsed = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map_err(|_| anyhow::anyhow!("invalid date '{date}' — expected YYYY-MM-DD"))?;

            let records = store.load_day(parsed);
            if records.is_empty() {
                println!("No records for {date} in context '{ctx}'.");
                return Ok(());
            }

            println!("Cluster metadata — context: {ctx}  date: {date}");
            println!("{}", "─".repeat(60));
            for record in &records {
                let ts = record.timestamp().format("%H:%M:%S");
                println!(
                    "[{}] [{}] {}",
                    ts,
                    record.type_label().to_uppercase(),
                    record.summary()
                );
            }
            println!();
            println!("{} records.", records.len());
        }

        // ── prune ─────────────────────────────────────────────────────────────
        MetaAction::Prune { before, context } => {
            let ctx = resolve_meta_context(&context.or_else(|| global_context.clone()));
            let Some(store) = meta::MetadataStore::new(&ctx) else {
                eprintln!("k7s meta: cannot open metadata store for context '{ctx}'");
                process::exit(1);
            };

            let cutoff = if let Some(ref s) = before {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .map_err(|_| anyhow::anyhow!("invalid date '{s}' — expected YYYY-MM-DD"))?
            } else {
                // Default: keep last 90 days.
                chrono::Utc::now().date_naive() - chrono::Duration::days(90)
            };

            let deleted = store.prune(cutoff)?;
            if deleted == 0 {
                println!("k7s meta prune: nothing to delete.");
            } else {
                println!("k7s meta prune: deleted {deleted} daily file(s) older than {cutoff}.");
            }
        }

        // ── export ────────────────────────────────────────────────────────────
        MetaAction::Export {
            context,
            days,
            output,
        } => {
            let ctx = resolve_meta_context(&context.or_else(|| global_context.clone()));
            let Some(store) = meta::MetadataStore::new(&ctx) else {
                eprintln!("k7s meta: cannot open metadata store for context '{ctx}'");
                process::exit(1);
            };

            let records = store.load_recent(days);
            if records.is_empty() {
                eprintln!(
                    "k7s meta export: no records for context '{ctx}' in the last {days} days."
                );
                return Ok(());
            }

            match output.as_str() {
                "json" => {
                    let json = serde_json::to_string_pretty(&records)
                        .map_err(|e| anyhow::anyhow!("JSON serialization error: {e}"))?;
                    println!("{json}");
                }
                _ => {
                    // Markdown postmortem report.
                    let history = meta::summarise(&records, days, &ctx);
                    let today = chrono::Utc::now().date_naive();
                    println!("# Cluster Incident Report — {ctx}");
                    println!();
                    println!(
                        "**Generated:** {}  **Period:** last {days} days",
                        today.format("%Y-%m-%d")
                    );
                    println!();
                    println!("## Summary");
                    println!();
                    println!("{}", history.to_context_block());
                    println!();
                    println!("## Full Record Log");
                    println!();

                    let today_naive = chrono::Utc::now().date_naive();
                    for i in (0..days as i64).rev() {
                        let date = today_naive - chrono::Duration::days(i);
                        let day_records: Vec<_> = records
                            .iter()
                            .filter(|r| r.timestamp().date_naive() == date)
                            .collect();
                        if day_records.is_empty() {
                            continue;
                        }
                        println!("### {}", date.format("%Y-%m-%d"));
                        println!();
                        for record in day_records {
                            let ts = record.timestamp().format("%H:%M:%S UTC");
                            println!(
                                "- **[{}]** `{}` — {}",
                                record.type_label(),
                                ts,
                                record.summary()
                            );
                        }
                        println!();
                    }
                }
            }
        }
    }

    Ok(())
}

// ─── Helm CLI handler ─────────────────────────────────────────────────────────

fn run_helm(action: HelmCliAction) -> anyhow::Result<()> {
    use crate::dao::helm::HelmDao;

    let dao = HelmDao::new(None);

    match action {
        HelmCliAction::List { namespace, output } => {
            let releases = dao
                .list(namespace.as_deref())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            match output.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&releases)?),
                "yaml" => println!("{}", serde_yaml::to_string(&releases)?),
                _ => {
                    println!(
                        "{:<30} {:<20} {:<12} {:<10} {:<30} {:<12}",
                        "NAME", "NAMESPACE", "REVISION", "STATUS", "CHART", "APP VERSION"
                    );
                    println!("{}", "-".repeat(120));
                    for r in &releases {
                        println!(
                            "{:<30} {:<20} {:<12} {:<10} {:<30} {:<12}",
                            r.name, r.namespace, r.revision, r.status, r.chart, r.app_version
                        );
                    }
                    println!("\n{} release(s) found.", releases.len());
                }
            }
        }

        HelmCliAction::Status { release, namespace } => {
            let releases = dao
                .list(Some(&namespace))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let rel = releases
                .into_iter()
                .find(|r| r.name == release)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "release '{}' not found in namespace '{}'",
                        release,
                        namespace
                    )
                })?;
            println!("Name:        {}", rel.name);
            println!("Namespace:   {}", rel.namespace);
            println!("Chart:       {}", rel.chart);
            println!("App Version: {}", rel.app_version);
            println!("Status:      {}", rel.status);
            println!("Revision:    {}", rel.revision);
            println!("Updated:     {}", rel.updated);
        }

        HelmCliAction::History {
            release,
            namespace,
            max,
        } => {
            let history = dao
                .history(&release, &namespace)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "{:<10} {:<12} {:<30} {:<12} {:<30} DESCRIPTION",
                "REVISION", "STATUS", "CHART", "APP VERSION", "UPDATED"
            );
            println!("{}", "-".repeat(120));
            for e in history.iter().take(max) {
                println!(
                    "{:<10} {:<12} {:<30} {:<12} {:<30} {}",
                    e.revision, e.status, e.chart, e.app_version, e.updated, e.description
                );
            }
        }

        HelmCliAction::Values {
            release,
            namespace,
            output,
        } => {
            let raw = run_helm_bin_capture(&[
                "get", "values", &release, "-n", &namespace, "--output", &output,
            ]);
            // Sanitize secret lines before printing.
            let sanitized = raw
                .lines()
                .map(|line| {
                    let lower = line.to_lowercase();
                    let is_secret = crate::sanitizer::helm::SECRET_KEY_PATTERNS
                        .iter()
                        .any(|p| lower.contains(&format!("{p}:")));
                    if is_secret {
                        if let Some(pos) = line.find(':') {
                            format!("{}: [REDACTED]", &line[..pos])
                        } else {
                            "[REDACTED]".to_string()
                        }
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            println!("{sanitized}");
        }

        HelmCliAction::Install {
            name,
            chart,
            namespace,
            values,
            set,
            dry_run,
        } => {
            let mut args = vec!["install", &name, &chart, "-n", &namespace];
            let values_str = values.as_deref().unwrap_or("");
            if !values_str.is_empty() {
                args.extend(["-f", values_str]);
            }
            let set_strs: Vec<String> = set;
            for s in &set_strs {
                args.extend(["--set", s.as_str()]);
            }
            if dry_run {
                args.push("--dry-run");
            }
            run_helm_bin(&args);
        }

        HelmCliAction::Upgrade {
            release,
            chart,
            namespace,
            values,
            set,
            install,
            dry_run,
        } => {
            let mut args = vec!["upgrade", &release, &chart, "-n", &namespace];
            let values_str = values.as_deref().unwrap_or("");
            if !values_str.is_empty() {
                args.extend(["-f", values_str]);
            }
            let set_strs: Vec<String> = set;
            for s in &set_strs {
                args.extend(["--set", s.as_str()]);
            }
            if install {
                args.push("--install");
            }
            if dry_run {
                args.push("--dry-run");
            }
            run_helm_bin(&args);
        }

        HelmCliAction::Rollback {
            release,
            revision,
            namespace,
            dry_run,
        } => {
            let rev_str;
            let mut args = vec!["rollback", &release];
            if let Some(rev) = revision {
                rev_str = rev.to_string();
                args.push(&rev_str);
            }
            args.extend(["-n", &namespace]);
            if dry_run {
                args.push("--dry-run");
            }
            run_helm_bin(&args);
        }

        HelmCliAction::Uninstall {
            release,
            namespace,
            dry_run,
        } => {
            let mut args = vec!["uninstall", &release, "-n", &namespace];
            if dry_run {
                args.push("--dry-run");
            }
            run_helm_bin(&args);
        }
    }

    Ok(())
}

/// Run the `helm` binary with `args`, streaming stdout/stderr; exit on failure.
fn run_helm_bin(args: &[&str]) {
    let status = std::process::Command::new("helm").args(args).status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("error: helm CLI not found or failed to run: {e}");
            std::process::exit(1);
        }
    }
}

/// Run the `helm` binary and capture its stdout as a String.
fn run_helm_bin_capture(args: &[&str]) -> String {
    match std::process::Command::new("helm").args(args).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            eprintln!("helm error: {}", err.trim());
            String::new()
        }
        Err(e) => {
            eprintln!("error: helm CLI not found or failed to run: {e}");
            String::new()
        }
    }
}

// ─── KubeVela CLI handler ─────────────────────────────────────────────────────

fn run_vela(
    action: VelaCliAction,
    global_context: &Option<String>,
    global_namespace: &Option<String>,
) -> anyhow::Result<()> {
    match action {
        // ── list ──────────────────────────────────────────────────────────────
        VelaCliAction::List { output, namespace } => {
            let ns_opt = namespace.as_deref().or(global_namespace.as_deref());
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let apps = rt.block_on(async {
                let client = build_kube_client(global_context).await?;
                Ok::<_, anyhow::Error>(crate::vela::client::list_apps(&client, ns_opt).await)
            })?;

            match output.as_str() {
                "json" => {
                    let v: Vec<serde_json::Value> = apps
                        .iter()
                        .map(|a| {
                            serde_json::json!({
                                "name": a.name,
                                "namespace": a.namespace,
                                "status": a.status,
                                "workflowStatus": a.workflow_status,
                                "components": a.component_count,
                                "age": a.age_label()
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&v)?);
                }
                _ => {
                    println!(
                        "{:<30} {:<18} {:<14} {:<14} {:<6}",
                        "NAME", "NAMESPACE", "STATUS", "WORKFLOW", "AGE"
                    );
                    for a in &apps {
                        println!(
                            "{:<30} {:<18} {:<14} {:<14} {:<6}",
                            a.name,
                            a.namespace,
                            a.status,
                            a.workflow_status,
                            a.age_label()
                        );
                    }
                }
            }
        }

        // ── status ────────────────────────────────────────────────────────────
        VelaCliAction::Status { app, namespace } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let raw = rt.block_on(async {
                let client = build_kube_client(global_context).await?;
                fetch_vela_app_raw(&client, &app, &namespace).await
            })?;

            let components = crate::vela::parse_components(&raw);
            let steps = crate::vela::parse_workflow_steps(&raw);

            println!("Application: {}/{}", namespace, app);
            println!();
            println!(
                "{:<30} {:<14} {:<8} MESSAGE",
                "COMPONENT", "TYPE", "HEALTHY"
            );
            for c in &components {
                println!(
                    "{:<30} {:<14} {:<8} {}",
                    c.name, c.workload_type, c.healthy, c.message
                );
            }
            if !steps.is_empty() {
                println!();
                println!("{:<25} {:<18} PHASE", "STEP", "TYPE");
                for s in &steps {
                    println!("{:<25} {:<18} {}", s.name, s.step_type, s.phase);
                }
            }
        }

        // ── tree ──────────────────────────────────────────────────────────────
        VelaCliAction::Tree { app, namespace } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let raw = rt.block_on(async {
                let client = build_kube_client(global_context).await?;
                fetch_vela_app_raw(&client, &app, &namespace).await
            })?;

            let components = crate::vela::parse_components(&raw);
            println!("Application: {}/{}", namespace, app);
            for (i, comp) in components.iter().enumerate() {
                let is_last = i == components.len() - 1;
                let prefix = if is_last { "└── " } else { "├── " };
                let health = if comp.healthy { "✓" } else { "✗" };
                println!("{prefix}{} ({}) [{health}]", comp.name, comp.workload_type);
                let trait_prefix = if is_last { "    " } else { "│   " };
                for (j, t) in comp.traits.iter().enumerate() {
                    let t_last = j == comp.traits.len() - 1;
                    let t_pfx = if t_last { "└── " } else { "├── " };
                    let th = if t.healthy { "✓" } else { "✗" };
                    println!("{trait_prefix}{t_pfx}[trait] {} [{th}]", t.trait_type);
                }
            }
        }

        // ── workflow ──────────────────────────────────────────────────────────
        VelaCliAction::Workflow { app, namespace } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let raw = rt.block_on(async {
                let client = build_kube_client(global_context).await?;
                fetch_vela_app_raw(&client, &app, &namespace).await
            })?;

            let steps = crate::vela::parse_workflow_steps(&raw);
            println!("Workflow for {}/{}", namespace, app);
            println!("{:<25} {:<18} {:<14} MESSAGE", "STEP", "TYPE", "PHASE");
            for s in &steps {
                println!(
                    "{:<25} {:<18} {:<14} {}",
                    s.name, s.step_type, s.phase, s.message
                );
            }
        }

        // ── restart (vela CLI) ────────────────────────────────────────────────
        VelaCliAction::Restart { app, namespace } => {
            run_vela_bin(&["workflow", "restart", &app, "-n", &namespace]);
        }

        // ── resume (vela CLI) ─────────────────────────────────────────────────
        VelaCliAction::Resume { app, namespace } => {
            run_vela_bin(&["workflow", "resume", &app, "-n", &namespace]);
        }

        // ── rollback (vela CLI) ───────────────────────────────────────────────
        VelaCliAction::Rollback {
            app,
            namespace,
            revision,
        } => {
            let mut args = vec![
                "workflow".to_owned(),
                "rollback".to_owned(),
                app,
                "-n".to_owned(),
                namespace,
            ];
            if let Some(rev) = revision {
                args.extend(["--revision".to_owned(), rev.to_string()]);
            }
            run_vela_bin(&args.iter().map(String::as_str).collect::<Vec<_>>());
        }

        // ── delete (vela CLI) ─────────────────────────────────────────────────
        VelaCliAction::Delete {
            app,
            namespace,
            yes,
        } => {
            let mut args = vec!["delete", &app, "-n", &namespace];
            if yes {
                args.push("--yes");
            }
            run_vela_bin(&args);
        }

        // ── defs ──────────────────────────────────────────────────────────────
        VelaCliAction::Defs { r#type, output } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let defs = rt.block_on(async {
                let client = build_kube_client(global_context).await?;
                Ok::<_, anyhow::Error>(
                    crate::vela::client::list_definitions(&client, &r#type).await,
                )
            })?;

            match output.as_str() {
                "json" => {
                    let v: Vec<serde_json::Value> = defs
                        .iter()
                        .map(|d| {
                            serde_json::json!({
                                "name": d.name,
                                "type": d.def_type,
                                "description": d.description
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&v)?);
                }
                _ => {
                    println!("{:<35} {:<14} DESCRIPTION", "NAME", "TYPE");
                    for d in &defs {
                        println!("{:<35} {:<14} {}", d.name, d.def_type, d.description);
                    }
                }
            }
        }
    }

    Ok(())
}

// ─── `k7s mcp` handler ───────────────────────────────────────────────────────

/// Start the MCP server.
///
/// Connects to the Kubernetes cluster, builds the `McpState`, then runs the
/// appropriate transport (stdio or http) until EOF or SIGINT.
fn run_mcp(
    context: &Option<String>,
    transport: String,
    port: u16,
    allow_mutations: bool,
) -> anyhow::Result<()> {
    use crate::config::{ConfigDirs, McpConfig};
    use crate::mcp::server::McpState;
    use std::sync::Arc;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let client = build_kube_client(context).await?;

        let meta_context = context.clone().unwrap_or_else(|| "default".to_string());

        let dirs = ConfigDirs::resolve()?;
        let cfg = config::load(&dirs.config_file()).unwrap_or_default();

        let mut mcp_cfg = cfg.k7s.mcp.clone();
        // CLI flags override config values.
        mcp_cfg.transport = transport;
        mcp_cfg.port = port;
        if allow_mutations {
            mcp_cfg.allow_mutations = true;
        }

        let state = Arc::new(McpState::new(
            client,
            cfg.k7s.ai.sanitizer.clone(),
            mcp_cfg.allow_mutations,
            meta_context,
        ));

        tracing::info!(
            transport = %mcp_cfg.transport,
            port = mcp_cfg.port,
            allow_mutations = mcp_cfg.allow_mutations,
            "starting k7s MCP server"
        );

        crate::mcp::run(state, &mcp_cfg)
            .await
            .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))
    })
}

/// Build a kube::Client honoring the optional context override.
async fn build_kube_client(context: &Option<String>) -> anyhow::Result<kube::Client> {
    let config = if let Some(ctx) = context {
        kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
            context: Some(ctx.clone()),
            ..Default::default()
        })
        .await?
    } else {
        kube::Config::infer().await?
    };
    Ok(kube::Client::try_from(config)?)
}

/// Fetch the raw JSON of a single KubeVela Application CR.
async fn fetch_vela_app_raw(
    client: &kube::Client,
    app_name: &str,
    namespace: &str,
) -> anyhow::Result<serde_json::Value> {
    use kube::api::{ApiResource, DynamicObject, ListParams};
    use kube::Api;

    let ar = ApiResource {
        group: "core.oam.dev".into(),
        version: "v1beta1".into(),
        api_version: "core.oam.dev/v1beta1".into(),
        kind: "Application".into(),
        plural: "applications".into(),
    };
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ar);
    let lp = ListParams::default().fields(&format!("metadata.name={app_name}"));
    let list = api.list(&lp).await?;
    let obj =
        list.items.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("application '{app_name}' not found in '{namespace}'")
        })?;
    Ok(serde_json::to_value(obj)?)
}

/// Run the `vela` binary with `args`, printing stdout/stderr and exiting on failure.
fn run_vela_bin(args: &[&str]) {
    let status = std::process::Command::new("vela").args(args).status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            process::exit(s.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("error: vela CLI not found or failed to run: {e}");
            eprintln!("Install with: curl -fsSl https://kubevela.io/install.sh | bash");
            process::exit(1);
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
