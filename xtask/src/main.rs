//! Repository maintenance tasks: `cargo xtask <command>`.
//!
//! - `png2c64-baselines --png2c64 <path>`: regenerate every fixture's png2c64 baseline
//!   (`baseline.kla` or `baseline.art`, `baseline.png`) and the baseline section of its
//!   README.
//! - `regen-goldens [--report | --accept --reason "..." --pr N|pending | --set-pr N]`: list golden-test changes
//!   (report mode, works on a dirty tree); accept them only with a reason, recorded in
//!   `fixtures/CHANGELOG.md` (needs a clean tree).
//! - `vice-check [--x64sc <path>]`: show each png2c64 baseline in VICE and compare the
//!   capture with it (local only).

mod baselines;
mod bluenoise;
mod goldens;
mod vice;

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
    /// Regenerate the solver's 64 × 64 blue-noise tile by void-and-cluster from its fixed
    /// seed (`crates/convert64-core/src/solve/blue_noise_64.bin`).
    BlueNoise,
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
    /// Show every png2c64 baseline in VICE and compare the capture with it.
    ///
    /// Needs only VICE (`x64sc`): the display PRGs are built from the committed baselines
    /// with `convert64_formats::prg`. Writes
    /// `fixtures/<name>/vice.png` and, for fixtures that pass (≥ 99.5 % of pixels agree
    /// and every cell shows only the baseline's colours), a line in the fixture README.
    /// Local only: VICE is not on the CI runners.
    ViceCheck {
        /// The VICE C64 emulator binary.
        #[arg(long, default_value = "x64sc")]
        x64sc: PathBuf,
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
    /// Accept mode (`--accept --reason "..." --pr N`): replaces the goldens and appends one
    /// line per fixture to `fixtures/CHANGELOG.md`, citing pull request N (open the PR
    /// first). Refuses a dirty git tree, so the goldens come from committed code.
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
        /// The pull request the new goldens go into, recorded in fixtures/CHANGELOG.md
        /// instead of a commit hash (squash merges make branch hashes unreachable): a
        /// number, or `pending` before the PR exists (then run `--set-pr N` once it does).
        #[arg(long, requires = "accept")]
        pr: Option<goldens::PrRef>,
        /// Rewrite the `#pending` entries in fixtures/CHANGELOG.md to this pull request
        /// number, after `--accept --pr pending` and opening the PR. Runs no tests.
        #[arg(long, value_name = "N", conflicts_with_all = ["accept", "report"])]
        set_pr: Option<u32>,
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
        Command::BlueNoise => bluenoise::run(&repo_root()),
        Command::Png2c64Baselines { png2c64, only } => baselines::run(&png2c64, &only),
        Command::ViceCheck { x64sc, only } => vice::run(&x64sc, &only),
        Command::RegenGoldens {
            report: _,
            accept,
            reason,
            pr,
            set_pr,
        } => match set_pr {
            Some(n) => goldens::set_pr(&repo_root(), n),
            None => goldens::run(accept, reason.as_deref(), pr),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
