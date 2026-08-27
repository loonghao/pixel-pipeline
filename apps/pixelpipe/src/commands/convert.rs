//! `pixelpipe convert` (PRD §7, §11.2).
//!
//! Converts one image to a true-pixel asset, writes artifacts atomically and
//! emits a report on stdout. Exit code follows the pass/review/fail status.

use crate::atomic::write_atomic;
use crate::util::{
    artifact_paths, build_report, metrics_from_output, resolve_profile, ArtifactPaths,
};
use anyhow::{anyhow, Context, Result};
use clap::Args as ClapArgs;
use pixel_core::bitmap::{Bitmap, DEFAULT_MAX_PIXELS};
use pixel_core::convert::{
    build_sheet_palette, convert, convert_bitmap, ConvertOptions, ConvertOutput,
};
use pixel_core::sheet::{detect_grid, slice, SheetSpec};
use pixel_formats::{parse_grid, parse_size, Artifacts, Canvas, Profile, Report, Status};
use std::collections::BTreeSet;
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
    /// Treat the input as a sprite sheet and slice it into a `ROWSxCOLS` grid,
    /// converting each cell separately. Outputs derive from the output stem as
    /// `stem_rRcC.png`.
    #[arg(long)]
    pub grid: Option<String>,
    /// Treat the input as a sprite sheet tiled by fixed `WxH` cells.
    #[arg(long, conflicts_with = "grid")]
    pub cell: Option<String>,
    /// Auto-detect a sprite-sheet grid from transparent gutters; falls back to
    /// single-sprite conversion when no grid is found.
    #[arg(long, conflicts_with_all = ["grid", "cell"])]
    pub auto_grid: bool,
    /// Skip writing sidecar artifacts (body/mask/preview); keep final + report.
    #[arg(long)]
    pub no_sidecars: bool,
    /// Pretty-print the report on stdout.
    #[arg(long)]
    pub pretty: bool,
    /// Detect identity-critical features (face/eyes) via the heuristic provider
    /// and preserve them during reconstruction + quantization (PRD §7.5).
    #[arg(long)]
    pub detect_features: bool,
    /// Also write the final palette as a GIMP/Aseprite `.gpl` sidecar
    /// (`<stem>.gpl`). Sheet mode writes one palette covering every cell.
    #[arg(long)]
    pub emit_palette: bool,
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
    /// Detect identity-critical features (face/eyes) and preserve them.
    pub detect_features: bool,
    /// Write a `.gpl` palette sidecar next to the output.
    pub emit_palette: bool,
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
        detect_features: params.detect_features,
        shared_palette: None,
    };
    let out = convert(&params.input, &profile, &opts)?;
    let palette_path = if params.emit_palette {
        let mut colors = BTreeSet::new();
        collect_final_colors(&out, &mut colors);
        Some(write_palette_gpl(&colors, &profile.name, &params.output)?)
    } else {
        None
    };
    finish_conversion(
        &out,
        &profile,
        profile_sha256,
        params.id.clone(),
        &params.input,
        &params.output,
        params.write_sidecars,
        palette_path.as_deref(),
    )
}

/// Accumulate the distinct opaque colors of a final PNG (body + outline).
fn collect_final_colors(out: &ConvertOutput, colors: &mut BTreeSet<[u8; 3]>) {
    let bmp = &out.final_png;
    for y in 0..bmp.height {
        for x in 0..bmp.width {
            let px = bmp.get(x, y);
            if px[3] > 0 {
                colors.insert([px[0], px[1], px[2]]);
            }
        }
    }
}

/// Write the palette as a GIMP `.gpl` file (loadable by Aseprite) next to the
/// output (`<stem>.gpl`), returning its path. Colors are already sorted by the
/// BTreeSet, so the file is deterministic.
fn write_palette_gpl(
    colors: &BTreeSet<[u8; 3]>,
    profile_name: &str,
    output: &Path,
) -> Result<PathBuf> {
    let dir = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "asset".to_string());
    let path = dir.join(format!("{stem}.gpl"));

    let mut text = String::from("GIMP Palette\n");
    text.push_str(&format!("Name: pixelpipe {profile_name} {stem}\n"));
    text.push_str("Columns: 8\n#\n");
    for c in colors {
        text.push_str(&format!(
            "{:3} {:3} {:3}\t#{:02x}{:02x}{:02x}\n",
            c[0], c[1], c[2], c[0], c[1], c[2]
        ));
    }
    write_atomic(&path, text.as_bytes())
        .with_context(|| format!("writing palette {}", path.display()))?;
    Ok(path)
}

/// Shared tail of a conversion: QA gate, artifact writing and report assembly.
/// Used by both the single-file and sprite-sheet-cell paths.
#[allow(clippy::too_many_arguments)]
fn finish_conversion(
    out: &ConvertOutput,
    profile: &Profile,
    profile_sha256: String,
    id: Option<String>,
    input: &Path,
    output: &Path,
    write_sidecars: bool,
    palette_path: Option<&Path>,
) -> Result<Report> {
    let metrics = metrics_from_output(out, profile);
    let qa_input = pixel_qa::QaInput {
        profile,
        metrics: metrics.clone(),
        mask_source: out.mask_source,
    };
    let (status, reasons) = pixel_qa::evaluate(&qa_input);

    let paths = artifact_paths(output);
    let mut artifacts = write_artifacts(out, &paths, write_sidecars)?;
    if let Some(p) = palette_path {
        artifacts.palette = Some(p.display().to_string());
    }

    let report = build_report(
        id,
        input,
        Some(output),
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

/// Slice a sprite sheet and convert every non-empty cell, returning one report
/// per cell. Cell outputs derive from the output stem as `stem_rRcC.png`.
pub fn run_sheet_conversion(params: &ConvertParams, spec: SheetSpec) -> Result<Vec<Report>> {
    let (mut profile, _) = resolve_profile(&params.profile)?;
    apply_overrides(&mut profile, params)?;
    let profile_sha256 = crate::util::sha256_str(&profile.to_toml()?);

    let sheet_bytes = std::fs::read(&params.input)
        .with_context(|| format!("reading {}", params.input.display()))?;
    let sheet_sha = pixel_cache::sha256_hex(&sheet_bytes);
    let sheet = Bitmap::load(&params.input, params.max_pixels)?;
    let cells = slice(&sheet, spec);
    if cells.is_empty() {
        return Err(anyhow!(
            "no non-empty cells found slicing {}",
            params.input.display()
        ));
    }

    let mut opts = ConvertOptions {
        max_pixels: params.max_pixels,
        detect_features: params.detect_features,
        shared_palette: None,
    };
    // Sheet-shared palette (Aseprite's "New Palette from Sprite"): build one
    // palette from every cell so animation frames never flicker between
    // frame-local palettes.
    if profile.palette.sheet_shared {
        let bitmaps: Vec<&pixel_core::bitmap::Bitmap> = cells.iter().map(|c| &c.bitmap).collect();
        let palette = build_sheet_palette(&bitmaps, &profile, &opts);
        if !palette.is_empty() {
            opts.shared_palette = Some(palette);
        }
    }
    // Convert every cell first so the optional `.gpl` sidecar can cover the
    // color union of all cells before the per-cell reports are finalized.
    let mut outputs: Vec<(u32, u32, ConvertOutput)> = Vec::with_capacity(cells.len());
    for cell in cells {
        let input_sha256 =
            crate::util::sha256_str(&format!("{sheet_sha}#r{}c{}", cell.row, cell.col));
        let out = convert_bitmap(cell.bitmap, input_sha256, &profile, &opts)?;
        outputs.push((cell.row, cell.col, out));
    }

    let palette_path = if params.emit_palette {
        let mut colors = BTreeSet::new();
        for (_, _, out) in &outputs {
            collect_final_colors(out, &mut colors);
        }
        Some(write_palette_gpl(&colors, &profile.name, &params.output)?)
    } else {
        None
    };

    let mut reports = Vec::with_capacity(outputs.len());
    for (row, col, out) in &outputs {
        let cell_out = cell_output_path(&params.output, *row, *col);
        let id = params
            .id
            .as_ref()
            .map(|base| format!("{base}_r{row}c{col}"));
        let report = finish_conversion(
            out,
            &profile,
            profile_sha256.clone(),
            id,
            &params.input,
            &cell_out,
            params.write_sidecars,
            palette_path.as_deref(),
        )?;
        reports.push(report);
    }
    Ok(reports)
}

/// Derive `stem_rRcC.ext` next to the requested output path.
fn cell_output_path(output: &Path, row: u32, col: u32) -> PathBuf {
    let dir = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "asset".to_string());
    let ext = output
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "png".to_string());
    dir.join(format!("{stem}_r{row}c{col}.{ext}"))
}

/// Resolve the sprite-sheet slicing spec from CLI flags, running auto-detection
/// when requested. Returns `None` when the input should be treated as a single
/// sprite.
fn resolve_sheet_spec(args: &Args) -> Result<Option<SheetSpec>> {
    if let Some(grid) = &args.grid {
        let (rows, cols) = parse_grid(grid)?;
        return Ok(Some(SheetSpec::Grid { rows, cols }));
    }
    if let Some(cell) = &args.cell {
        let (w, h) = parse_size(cell)?;
        return Ok(Some(SheetSpec::Cell { w, h }));
    }
    if args.auto_grid {
        let sheet = Bitmap::load(&args.input, args.max_pixels)?;
        if let Some((rows, cols)) = detect_grid(&sheet) {
            eprintln!("auto-grid: detected {rows}x{cols} grid");
            return Ok(Some(SheetSpec::Grid { rows, cols }));
        }
        eprintln!("auto-grid: no grid detected, converting as a single sprite");
    }
    Ok(None)
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
    let sheet_spec = resolve_sheet_spec(&args)?;
    let pretty = args.pretty;
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
        detect_features: args.detect_features,
        emit_palette: args.emit_palette,
    };

    // Sprite-sheet mode: one JSONL report line per cell, exit code = worst
    // status. Single-sprite mode: one report object.
    if let Some(spec) = sheet_spec {
        let reports = run_sheet_conversion(&params, spec)?;
        let mut worst = Status::Pass;
        for report in &reports {
            worst = worst.merge(report.status);
            println!("{}", report.to_json());
        }
        return Ok(worst.exit_code());
    }

    let report = run_conversion(&params)?;
    let json = if pretty {
        report.to_json_pretty()
    } else {
        report.to_json()
    };
    println!("{json}");
    Ok(report.status.exit_code())
}
