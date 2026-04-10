// The project is under active development — many public items in sub-modules
// are implemented ahead of being wired into the main application loop.
// Suppress the resulting dead_code / unused_imports noise at the binary level.
#![allow(dead_code, unused_imports)]

mod config;
mod error;
mod ui;

// Future modules — declared here so the compiler resolves them
// as the project grows through the roadmap phases.
mod client;
mod dao;
mod exec;
mod model;
mod portforward;
mod render;
mod sanitizer;
mod ai;
mod view;
mod watch;
mod health;
mod util;

use clap::Parser;
use tracing_subscriber::EnvFilter;

/// k7s — Security-first Kubernetes TUI with AI-powered cluster analysis.
///
/// Connect to your cluster via the active kubeconfig context and browse
/// resources interactively. Use the built-in AI chat window (:chat) to
/// analyse logs, diagnose issues, and get efficiency recommendations —
/// without ever sending secrets to the LLM.
#[derive(Parser, Debug)]
#[command(
    name = "k7s",
    version,
    about,
    long_about = None
)]
struct Cli {
    /// Kubernetes context to connect to (defaults to active kubeconfig context).
    #[arg(short = 'c', long)]
    context: Option<String>,

    /// Namespace to use (defaults to all namespaces).
    #[arg(short = 'n', long)]
    namespace: Option<String>,

    /// Disable all mutating operations (delete, scale, edit, exec).
    #[arg(long)]
    readonly: bool,

    /// Log level filter (e.g. debug, info, warn, error).
    ///
    /// Also respects the RUST_LOG environment variable.
    #[arg(short = 'l', long, default_value = "info")]
    log_level: String,

    /// Path to a custom config file (overrides XDG default location).
    #[arg(long)]
    config: Option<std::path::PathBuf>,

    /// Write structured logs to this file instead of stderr.
    #[arg(long)]
    log_file: Option<std::path::PathBuf>,

    /// Print cluster info and exit without opening the TUI.
    #[arg(long)]
    headless: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    init_tracing(&cli.log_level, cli.log_file.as_deref())?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "k7s starting");

    // Resolve config directories and load config.
    let dirs = config::ConfigDirs::resolve().map_err(error::AppError::Config)?;
    let config_path = cli.config.unwrap_or_else(|| dirs.config_file());
    let mut cfg = config::load(&config_path).map_err(error::AppError::Config)?;

    // CLI flags override config file values.
    if cli.readonly {
        cfg.k7s.read_only = true;
    }

    tracing::debug!(
        config_file = %config_path.display(),
        readonly = cfg.k7s.read_only,
        "configuration loaded"
    );

    if cli.headless {
        // Headless mode: print resolved config and exit.
        println!("k7s v{}", env!("CARGO_PKG_VERSION"));
        println!("Config: {}", config_path.display());
        println!("Read-only: {}", cfg.k7s.read_only);
        return Ok(());
    }

    ui::run(cfg).map_err(|e| {
        tracing::error!(error = %e, "fatal error in TUI loop");
        e
    })
}

fn init_tracing(
    level: &str,
    log_file: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

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
