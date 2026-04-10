//! build.rs — generate shell completion scripts at compile time.
//!
//! Completions are written to `$OUT_DIR/completions/` and can be installed
//! from there, or the release CI copies them to the dist archive.
//!
//! Generated files:
//!   k7s.bash        → /etc/bash_completion.d/k7s  or ~/.local/share/bash-completion/completions/
//!   k7s.fish        → ~/.config/fish/completions/
//!   _k7s (zsh)      → a directory on $fpath
//!   k7s.elv         → elvish (bonus)
//!   k7s.ps1         → PowerShell (bonus)

use std::env;
use std::path::PathBuf;

// Import clap and clap_complete.  We re-declare the CLI struct here (without
// the full business logic) so build.rs has no dependency on the rest of the
// crate.  The struct must stay in sync with src/main.rs.
use clap::{CommandFactory, Parser};
use clap_complete::{generate_to, Shell};

#[derive(Parser)]
#[command(name = "k7s", version)]
struct Cli {
    #[arg(short = 'c', long)]
    context: Option<String>,
    #[arg(short = 'n', long)]
    namespace: Option<String>,
    #[arg(long)]
    readonly: bool,
    #[arg(short = 'l', long, default_value = "info")]
    log_level: String,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    log_file: Option<PathBuf>,
    #[arg(long)]
    headless: bool,
}

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let completions_dir = out_dir.join("completions");
    std::fs::create_dir_all(&completions_dir).expect("create completions dir");

    let mut cmd = Cli::command();

    for shell in [Shell::Bash, Shell::Fish, Shell::Zsh, Shell::Elvish, Shell::PowerShell] {
        generate_to(shell, &mut cmd, "k7s", &completions_dir)
            .unwrap_or_else(|e| panic!("failed to generate {shell:?} completions: {e}"));
    }

    // Tell Cargo to re-run build.rs if the CLI definition changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/main.rs");
}
