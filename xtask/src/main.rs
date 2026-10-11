//! Repository maintenance tasks: `cargo xtask <command>`.
//!
//! - `png2c64-baselines --png2c64 <path>`: regenerate every fixture's png2c64 baseline
//!   (`baseline.kla` or `baseline.art`, `baseline.png`) and the baseline section of its
//!   README.
//! - `regen-goldens [--report | --accept --reason "..."]`: list golden-test changes
//!   (report mode, works on a dirty tree); accept them only with a reason, recorded in
//!   `fixtures/CHANGELOG.md` (needs a clean tree).

mod baselines;
mod goldens;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "cargo xtask", about = "Convert64 repository maintenance tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate the png2c64 baselines of every fixture.
    ///
    /// Runs png2c64 on each `fixtures/<name>/source.png` with the settings recorded in
    /// this tool, cuts the Koala or Art Studio data out of the display PRG png2c64
    /// writes, checks it pixel for pixel against png2c64's own PNG, writes
    /// `baseline.kla`/`.art` and `baseline.png`, runs `convert64 validate` on the
    /// result and records everything in the fixture's README.
    Png2c64Baselines {
        /// The png2c64 binary, built from <https://github.com/tinic/png2c64>.
        #[arg(long)]
        png2c64: PathBuf,
        /// Only these fixtures (default: all).
        #[arg(long)]
        only: Vec<String>,
    },
    /// List golden-test changes; accept them only with a reason.
    ///
    /// Report mode (`--report`, the default): runs the golden harness, compares its
    /// candidates with the committed goldens and lists what differs (exit 1 if anything
    /// does). It changes no committed file and works on a dirty tree.
    ///
    /// Accept mode (`--accept --reason "..."`): replaces the goldens and appends one line
    /// per fixture to `fixtures/CHANGELOG.md`. Refuses a dirty git tree, because the
    /// changelog records the commit that produced the goldens.
    RegenGoldens {
        /// Only list what differs (the default); allowed on a dirty tree.
        #[arg(long, conflicts_with = "accept")]
        report: bool,
        /// Replace the goldens with the current output.
        #[arg(long, requires = "reason")]
        accept: bool,
        /// Why the goldens change; recorded in fixtures/CHANGELOG.md.
        #[arg(long, requires = "accept")]
        reason: Option<String>,
    },
}

/// The repository root (the parent of this crate's directory).
pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Png2c64Baselines { png2c64, only } => baselines::run(&png2c64, &only),
        Command::RegenGoldens {
            report: _,
            accept,
            reason,
        } => goldens::run(accept, reason.as_deref()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
