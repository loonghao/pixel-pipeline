//! `pixelpipe batch` (PRD §7.9, §11.2).
//!
//! Reads a JSONL manifest, converts each task (optionally in parallel), and
//! streams one report per line to stdout as JSONL. Exit code is the most
//! severe task status (fail > review > pass).

use crate::commands::convert::{run_conversion, ConvertParams};
use crate::util::artifact_paths;
use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use pixel_core::bitmap::DEFAULT_MAX_PIXELS;
use pixel_formats::{BatchTask, Report, Status};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

#[derive(ClapArgs)]
pub struct Args {
    /// JSONL manifest, one task per line.
    pub manifest: PathBuf,
    /// Default profile for tasks without their own `profile` field.
    #[arg(short, long, default_value = "character-48")]
    pub profile: String,
    /// Directory for outputs when a task omits `output` (uses `<id>.png`).
    #[arg(long, default_value = ".")]
    pub out_dir: PathBuf,
    /// Number of worker threads (default: all cores).
    #[arg(long)]
    pub jobs: Option<usize>,
    /// Skip tasks whose report already exists, re-emitting it as cached.
    #[arg(long)]
    pub resume: bool,
    /// Maximum decoded input pixels per task (safety limit).
    #[arg(long, default_value_t = DEFAULT_MAX_PIXELS)]
    pub max_pixels: u64,
}

/// One task outcome carried through to stdout emission.
enum Outcome {
    Report(Box<Report>),
    Error { id: String, error: String },
}

impl Outcome {
    fn status(&self) -> Status {
        match self {
            Outcome::Report(r) => r.status,
            Outcome::Error { .. } => Status::Fail,
        }
    }

    fn line(&self) -> String {
        match self {
            Outcome::Report(r) => r.to_json(),
            Outcome::Error { id, error } => serde_json::json!({
                "id": id,
                "status": "fail",
                "error": error,
            })
            .to_string(),
        }
    }
}

/// Resolve the output path for a task (explicit or `<out_dir>/<id>.png`).
fn task_output(task: &BatchTask, out_dir: &Path) -> PathBuf {
    match &task.output {
        Some(o) => PathBuf::from(o),
        None => out_dir.join(format!("{}.png", task.id)),
    }
}

/// Process one task into an outcome (never panics; errors become fail entries).
fn process(task: &BatchTask, args: &Args) -> Outcome {
    let output = task_output(task, &args.out_dir);

    if args.resume {
        let report_path = artifact_paths(&output).report;
        if let Some(mut r) = read_cached(&report_path) {
            r.cached = true;
            return Outcome::Report(Box::new(r));
        }
    }

    let params = ConvertParams {
        id: Some(task.id.clone()),
        input: PathBuf::from(&task.input),
        output,
        profile: task.profile.clone().unwrap_or_else(|| args.profile.clone()),
        size: task.size.clone(),
        max_colors: task.max_colors,
        outline_color: task.outline_color.clone(),
        max_pixels: args.max_pixels,
        write_sidecars: true,
    };
    match run_conversion(&params) {
        Ok(report) => Outcome::Report(Box::new(report)),
        Err(e) => Outcome::Error {
            id: task.id.clone(),
            error: format!("{e:#}"),
        },
    }
}

/// Read and parse an existing report sidecar for `--resume`.
fn read_cached(path: &Path) -> Option<Report> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Parse the manifest into tasks, failing on malformed lines.
fn read_tasks(path: &Path) -> Result<Vec<BatchTask>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    let mut tasks = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let task =
            BatchTask::from_line(line).with_context(|| format!("manifest line {}", i + 1))?;
        tasks.push(task);
    }
    Ok(tasks)
}

/// CLI entry point for `batch`.
pub fn run(args: Args) -> Result<i32> {
    let tasks = read_tasks(&args.manifest)?;

    let compute = || {
        tasks
            .par_iter()
            .map(|task| process(task, &args))
            .collect::<Vec<_>>()
    };
    let outcomes = match args.jobs {
        Some(n) if n > 0 => rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .context("building thread pool")?
            .install(compute),
        _ => compute(),
    };

    let mut worst = Status::Pass;
    for outcome in &outcomes {
        worst = worst.merge(outcome.status());
        println!("{}", outcome.line());
    }
    Ok(worst.exit_code())
}
