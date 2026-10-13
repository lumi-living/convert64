//! Repository maintenance tasks: `cargo xtask <command>`.
//!
//! - `png2c64-baselines --png2c64 <path>`: regenerate every fixture's png2c64 baseline
//!   (`baseline.kla` or `baseline.art`, `baseline.png`) and the baseline section of its
//!   README.
//! - `regen-goldens [--report | --accept --reason "..." --pr N|pending | --set-pr N]`: list golden-test changes
//!   (report mode, works on a dirty tree); accept them only with a reason, recorded in
//!   `fixtures/CHANGELOG.md` (needs a clean tree).
//! - `vice-check [--x64sc <path>] [--skip-baselines] [--skip-exports]`: show each png2c64
//!   baseline, and every export of Convert64's own solve of each fixture, in VICE and
//!   compare the capture with it (local only).
//! - `dither-sheet [--input <png> --out <dir>] [--mode mc|hires] [--reason "..."]`: render
//!   every dither method and the comparison sheets (`docs/reference/dither-previews/`).
//! - `front-end-sheet`: the resampler A/B of #190 (the compositor vs png2c64's front
//!   end on every fixture, both into the reference solver).
//! - `png2c64-previews --png2c64 <path>`: regenerate png2c64's own renders and contact
//!   sheets in `docs/reference/dither-previews/`, the parity test's reference.
//! - `tokens [--check]`: generate the app theme (`crates/convert64-app/src/theme/generated.rs`)
//!   from the Figma variables in `design/tokens.json`; `--check` fails if it is stale.

mod baselines;
mod bluenoise;
mod font;
mod frontend;
mod goldens;
mod previews;
mod sheet;
mod tokens;
mod vice;
mod vice_exports;

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
    /// The resampler A/B (#190): every fixture at its baseline settings prepared by the
    /// compositor and by png2c64's front end, both solved by the reference solver, next
    /// to png2c64's baseline, in `docs/reference/dither-previews/convert64-sheets/`.
    FrontEndSheet,
    /// Regenerate png2c64's renders of its 30 dither methods and its contact sheets.
    ///
    /// Runs png2c64 on the dither-preview test card (strength 1.0) and on
    /// `fixtures/portrait/source.png` (png2c64's defaults) for every method id, writes
    /// `docs/reference/dither-previews/png2c64-card/`, `png2c64-portrait/` and the three
    /// sheets, and lists the renders that changed. Use the reference build
    /// (`docs/reference/png2c64-reference.md`): the parity test compares with these.
    Png2c64Previews {
        /// The png2c64 binary.
        #[arg(long)]
        png2c64: PathBuf,
    },
    /// Show every png2c64 baseline and every export of our own solves in VICE and
    /// compare each capture with the picture.
    ///
    /// Needs only VICE (`x64sc`): the display PRGs are built with
    /// `convert64_formats::prg`. Baselines: writes `fixtures/<name>/vice.png` and, for
    /// fixtures that pass (≥ 99.5 % of pixels agree and every cell shows only the
    /// baseline's colours), a line in the fixture README. Exports: solves each fixture
    /// in both modes and checks the Koala / Art Studio writer, each bitpair policy and
    /// the raw set (captures in `target/xtask/vice/exports/`) against the same bar, and
    /// records them in the fixture README. Local only: VICE is not on the CI runners.
    ViceCheck {
        /// The VICE C64 emulator binary.
        #[arg(long, default_value = "x64sc")]
        x64sc: PathBuf,
        /// Only these fixtures (default: all).
        #[arg(long)]
        only: Vec<String>,
        /// Do not check the png2c64 baselines.
        #[arg(long)]
        skip_baselines: bool,
        /// Do not check the exports of our own solves.
        #[arg(long)]
        skip_exports: bool,
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
    /// Render every dither method on the test card and the portrait, and the sheets.
    ///
    /// Without `--input`, regenerates the committed previews in
    /// `docs/reference/dither-previews/convert64-card/`, `convert64-portrait/` and
    /// `convert64-sheets/` (card, grey-ramp zoom, portrait, and ours next to png2c64's
    /// for the 30 shared ids with their OKLab MSE), and records `--reason` in that
    /// directory's CHANGELOG.md. With `--input <png> --out <dir>`, renders that picture
    /// instead.
    DitherSheet {
        /// A picture to render instead of the committed previews.
        #[arg(long, requires = "out")]
        input: Option<PathBuf>,
        /// Target mode.
        #[arg(long, value_enum, default_value = "mc")]
        mode: SheetMode,
        /// Where `--input`'s renders go.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Why the committed previews change (needed without `--input`).
        #[arg(long, conflicts_with = "input")]
        reason: Option<String>,
    },
    /// Generate the app theme from the Figma variables.
    ///
    /// Reads `design/tokens.json` and writes `crates/convert64-app/src/theme/generated.rs`
    /// (egui visuals for both themes, spacing, text styles, fonts, Convert64's semantic
    /// colours and layout constants). Fails on any variable whose Rust path egui 0.36
    /// or the app does not have.
    Tokens {
        /// Write nothing; fail if the committed file differs from the generated one.
        #[arg(long)]
        check: bool,
    },
}

/// `dither-sheet`'s target mode.
#[derive(Clone, Copy, clap::ValueEnum)]
enum SheetMode {
    /// Multicolor bitmap.
    Mc,
    /// Hi-res bitmap.
    Hires,
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
        Command::Png2c64Previews { png2c64 } => previews::run(&png2c64),
        Command::FrontEndSheet => frontend::run(),
        Command::ViceCheck {
            x64sc,
            only,
            skip_baselines,
            skip_exports,
        } => {
            let baselines = if skip_baselines {
                Ok(())
            } else {
                vice::run(&x64sc, &only)
            };
            let exports = if skip_exports {
                Ok(())
            } else {
                vice_exports::run(&x64sc, &only)
            };
            baselines.and(exports)
        }
        Command::DitherSheet {
            input,
            mode,
            out,
            reason,
        } => sheet::run(
            &repo_root(),
            input,
            match mode {
                SheetMode::Mc => convert64_core::mode::Mode::MulticolorBitmap,
                SheetMode::Hires => convert64_core::mode::Mode::HiresBitmap,
            },
            out,
            reason.as_deref(),
        ),
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
        Command::Tokens { check } => tokens::run(&repo_root(), check),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
