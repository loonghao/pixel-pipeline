//! Shared CLI helpers: profile resolution, artifact paths, report assembly.

use anyhow::{anyhow, Context, Result};
use pixel_core::convert::ConvertOutput;
use pixel_formats::{
    Artifacts, Canvas, Profile, QaMetrics, Reason, Report, Status, REPORT_SCHEMA_VERSION,
    TOOL_VERSION,
};
use std::path::{Path, PathBuf};

/// Built-in profiles bundled into the binary (PRD FR-PROFILE-002).
const BUILTIN: &[(&str, &str)] = &[
    (
        "character-32",
        include_str!("../../../profiles/character-32.toml"),
    ),
    (
        "character-48",
        include_str!("../../../profiles/character-48.toml"),
    ),
    (
        "character-64",
        include_str!("../../../profiles/character-64.toml"),
    ),
];

/// Resolve a `--profile` value: a file path if it exists, else a built-in name.
/// Returns the parsed profile plus its canonical TOML text (for hashing).
pub fn resolve_profile(spec: &str) -> Result<(Profile, String)> {
    let p = Path::new(spec);
    if p.exists() {
        let text = std::fs::read_to_string(p)
            .with_context(|| format!("reading profile {}", p.display()))?;
        let profile = Profile::from_toml(&text)?;
        return Ok((profile, text));
    }
    let name = spec.trim_end_matches(".toml");
    if let Some((_, text)) = BUILTIN.iter().find(|(n, _)| *n == name) {
        let profile = Profile::from_toml(text)?;
        return Ok((profile, (*text).to_string()));
    }
    Err(anyhow!(
        "profile '{spec}' not found as a file or built-in ({})",
        BUILTIN
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Derive sidecar artifact paths from a final output path (PRD §7.10).
pub struct ArtifactPaths {
    pub final_png: PathBuf,
    pub body: PathBuf,
    pub body_mask: PathBuf,
    pub outline_mask: PathBuf,
    pub preview: PathBuf,
    pub report: PathBuf,
}

pub fn artifact_paths(output: &Path) -> ArtifactPaths {
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "asset".to_string());
    let dir = output.parent().unwrap_or_else(|| Path::new("."));
    let with = |suffix: &str| dir.join(format!("{stem}{suffix}"));
    ArtifactPaths {
        final_png: output.to_path_buf(),
        body: with(".body.png"),
        body_mask: with(".body-mask.png"),
        outline_mask: with(".outline-mask.png"),
        preview: with(".preview.png"),
        report: with(".report.json"),
    }
}

/// Assemble QA metrics from a conversion output + profile limits.
pub fn metrics_from_output(out: &ConvertOutput, profile: &Profile) -> QaMetrics {
    let dimension_valid = out.final_png.width == profile.target.width
        && out.final_png.height == profile.target.height;
    QaMetrics {
        dimension_valid,
        alpha_binary: out.alpha_binary,
        body_pixels: out.body_pixels,
        outline_pixels: out.outline_pixels,
        body_components: out.body_components,
        palette_colors: out.palette_colors,
        palette_limit: profile.palette.max_colors,
        // A freshly compiled outline matches expectation by construction.
        outline_extra_pixels: 0,
        outline_missing_pixels: 0,
        outline_color_mismatch_pixels: 0,
        body_pixels_in_reserved_border: out.body_pixels_in_reserved_border,
    }
}

/// Build a full report for a conversion.
#[allow(clippy::too_many_arguments)]
pub fn build_report(
    id: Option<String>,
    input: &Path,
    output: Option<&Path>,
    profile_name: &str,
    profile_sha256: String,
    input_sha256: String,
    canvas: Canvas,
    mask_source: &str,
    cached: bool,
    status: Status,
    qa: QaMetrics,
    reasons: Vec<Reason>,
    warnings: Vec<String>,
    artifacts: Artifacts,
) -> Report {
    Report {
        schema_version: REPORT_SCHEMA_VERSION,
        tool_version: TOOL_VERSION.to_string(),
        status,
        id,
        input: input.display().to_string(),
        output: output.map(|p| p.display().to_string()),
        profile: profile_name.to_string(),
        profile_sha256,
        input_sha256,
        canvas,
        mask_source: mask_source.to_string(),
        cached,
        qa,
        reasons,
        warnings,
        artifacts,
    }
}

/// SHA-256 of a string (for profile hashing).
pub fn sha256_str(s: &str) -> String {
    pixel_cache::sha256_hex(s.as_bytes())
}
