//! `pixelpipe convert` (PRD §7, §11.2).
//!
//! Converts one image to a true-pixel asset, writes artifacts atomically and
//! emits a report on stdout. Exit code follows the pass/review/fail status.

use crate::atomic::write_atomic;
use crate::util::{
    artifact_paths, build_report, metrics_from_output, resolve_profile, ArtifactPaths,
};
use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use pixel_core::bitmap::DEFAULT_MAX_PIXELS;
use pixel_core::convert::{convert, ConvertOptions};
use pixel_formats::{parse_size, Artifacts, Canvas, Profile, Report};
use std::path::{Path, PathBuf};

#[derive(ClapArgs)]
pub struct Args {
    /// Input image path.
    pub input: PathBuf,
    /// Output final PNG path. Sidecar artifacts derive from this stem.
    #[arg(short, long)]
    pub output: PathBuf,
    /// Profile file path or built-in name (e.g. `character-48`).
    #[arg(short, long, default_value = "character-48")]
    pub profile: String,
    /// Override target size, e.g. `48x48`.
    #[arg(long)]
    pub size: Option<String>,
    /// Override palette color budget.
    #[arg(long)]
    pub max_colors: Option<u32>,
    /// Override outline color, e.g. `#2b1009`.
    #[arg(long)]
    pub outline_color: Option<String>,
    /// Maximum decoded input pixels (safety limit).
    #[arg(long, default_value_t = DEFAULT_MAX_PIXELS)]
    pub max_pixels: u64,
    /// Skip writing sidecar artifacts (body/mask/preview); keep final + report.
    #[arg(long)]
    pub no_sidecars: bool,
    /// Pretty-print the report on stdout.
    #[arg(long)]
    pub pretty: bool,
}

/// Parameters for a single conversion, shared by `convert` and `batch`.
pub struct ConvertParams {
    pub id: Option<String>,
    pub input: PathBuf,
    pub output: PathBuf,
    pub profile: String,
    pub size: Option<String>,
    pub max_colors: Option<u32>,
    pub outline_color: Option<String>,
    pub max_pixels: u64,
    pub write_sidecars: bool,
}

/// Apply inline overrides onto a resolved profile and re-validate.
fn apply_overrides(profile: &mut Profile, params: &ConvertParams) -> Result<()> {
    if let Some(size) = &params.size {
        let (w, h) = parse_size(size)?;
        profile.target.width = w;
        profile.target.height = h;
    }
    if let Some(mc) = params.max_colors {
        profile.palette.max_colors = mc;
    }
    if let Some(color) = &params.outline_color {
        profile.outline.color = color.clone();
    }
    profile
        .validate()
        .context("profile invalid after overrides")?;
    Ok(())
}

/// Run one conversion end to end, writing artifacts and returning its report.
pub fn run_conversion(params: &ConvertParams) -> Result<Report> {
    let (mut profile, _) = resolve_profile(&params.profile)?;
    apply_overrides(&mut profile, params)?;
    let profile_sha256 = crate::util::sha256_str(&profile.to_toml()?);

    let opts = ConvertOptions {
        max_pixels: params.max_pixels,
    };
    let out = convert(&params.input, &profile, &opts)?;

    let metrics = metrics_from_output(&out, &profile);
    let qa_input = pixel_qa::QaInput {
        profile: &profile,
        metrics: metrics.clone(),
        mask_source: out.mask_source,
    };
    let (status, reasons) = pixel_qa::evaluate(&qa_input);

    let paths = artifact_paths(&params.output);
    let artifacts = write_artifacts(&out, &paths, params.write_sidecars)?;

    let report = build_report(
        params.id.clone(),
        &params.input,
        Some(&params.output),
        &profile.name,
        profile_sha256,
        out.input_sha256.clone(),
        Canvas {
            width: out.final_png.width,
            height: out.final_png.height,
        },
        out.mask_source.as_str(),
        false,
        status,
        metrics,
        reasons,
        Vec::new(),
        artifacts,
    );

    let report_json = report.to_json_pretty();
    write_atomic(&paths.report, report_json.as_bytes())
        .with_context(|| format!("writing report {}", paths.report.display()))?;
    Ok(report)
}

/// Write final + optional sidecar PNGs atomically, returning their paths.
fn write_artifacts(
    out: &pixel_core::convert::ConvertOutput,
    paths: &ArtifactPaths,
    sidecars: bool,
) -> Result<Artifacts> {
    let write_png = |bmp: &pixel_core::bitmap::Bitmap, path: &Path| -> Result<String> {
        let bytes = bmp.to_png_bytes()?;
        write_atomic(path, &bytes).with_context(|| format!("writing {}", path.display()))?;
        Ok(path.display().to_string())
    };

    let mut artifacts = Artifacts {
        final_png: Some(write_png(&out.final_png, &paths.final_png)?),
        report: Some(paths.report.display().to_string()),
        ..Default::default()
    };
    if sidecars {
        artifacts.body = Some(write_png(&out.body, &paths.body)?);
        artifacts.body_mask = Some(write_png(&out.body_mask_bitmap, &paths.body_mask)?);
        artifacts.outline_mask = Some(write_png(&out.outline_mask_bitmap, &paths.outline_mask)?);
        artifacts.preview = Some(write_png(&out.preview, &paths.preview)?);
    }
    Ok(artifacts)
}

/// CLI entry point for `convert`.
pub fn run(args: Args) -> Result<i32> {
    let params = ConvertParams {
        id: None,
        input: args.input,
        output: args.output,
        profile: args.profile,
        size: args.size,
        max_colors: args.max_colors,
        outline_color: args.outline_color,
        max_pixels: args.max_pixels,
        write_sidecars: !args.no_sidecars,
    };
    let report = run_conversion(&params)?;
    let json = if args.pretty {
        report.to_json_pretty()
    } else {
        report.to_json()
    };
    println!("{json}");
    Ok(report.status.exit_code())
}
