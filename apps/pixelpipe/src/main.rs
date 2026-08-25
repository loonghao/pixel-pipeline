//! `pixelpipe` — Agent-first true-pixel asset compiler CLI (PRD §12).
//!
//! Contract (PRD §11.2): stdout is JSON/JSONL only, stderr carries human logs,
//! exit codes follow the pass/review/fail status model (§7.11).

mod atomic;
mod commands;
mod util;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "pixelpipe",
    version,
    about = "Agent-first Rust true-pixel asset compiler"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect an input image and suggest a profile/mode.
    Inspect(commands::inspect::Args),
    /// Convert a single image to a true-pixel asset at a target size.
    Convert(commands::convert::Args),
    /// Validate an existing sprite (+ optional body mask) against a profile.
    Validate(commands::validate::Args),
    /// Batch-convert tasks from a JSONL manifest.
    Batch(commands::batch::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Inspect(a) => commands::inspect::run(a),
        Command::Convert(a) => commands::convert::run(a),
        Command::Validate(a) => commands::validate::run(a),
        Command::Batch(a) => commands::batch::run(a),
    };
    match code {
        Ok(exit) => ExitCode::from(exit as u8),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}
