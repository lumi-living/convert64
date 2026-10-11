//! Repository maintenance tasks: `cargo xtask <command>`.
//!
//! - `png2c64-baselines --png2c64 <path>`: regenerate every fixture's png2c64 baseline
//!   (`baseline.kla` or `baseline.art`, `baseline.png`) and the baseline section of its
//!   README.

mod baselines;

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
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
