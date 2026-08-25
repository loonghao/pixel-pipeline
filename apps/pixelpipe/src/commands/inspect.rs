//! `pixelpipe inspect` (PRD §7.1, §11.2).
//!
//! Emits a single JSON object describing the input on stdout and returns exit
//! code 0. Inspection never fails the gate — it only informs.

use anyhow::Result;
use clap::Args as ClapArgs;
use pixel_core::bitmap::DEFAULT_MAX_PIXELS;
use std::path::PathBuf;

#[derive(ClapArgs)]
pub struct Args {
    /// Input image path.
    pub input: PathBuf,
    /// Maximum decoded input pixels (safety limit).
    #[arg(long, default_value_t = DEFAULT_MAX_PIXELS)]
    pub max_pixels: u64,
    /// Pretty-print the JSON output.
    #[arg(long)]
    pub pretty: bool,
}

/// Run `inspect`, printing the structured result as JSON to stdout.
pub fn run(args: Args) -> Result<i32> {
    let result = pixel_core::inspect(&args.input, args.max_pixels)?;
    let json = if args.pretty {
        serde_json::to_string_pretty(&result)?
    } else {
        serde_json::to_string(&result)?
    };
    println!("{json}");
    Ok(0)
}
