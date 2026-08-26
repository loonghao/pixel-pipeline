//! Input inspection (`pixelpipe inspect`, PRD §7.1).

use crate::bitmap::Bitmap;
use crate::mask::{alpha_coverage, count_components, foreground_from_alpha};
use crate::sheet::detect_grid;
use serde::Serialize;
use std::path::Path;

/// Suggested processing mode for an input (PRD FR-INSPECT-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SuggestedMode {
    /// Input already looks like pixel art; snap to grid.
    PixelSnap,
    /// Flat illustration; reconstruct to target grid.
    FlatReconstruct,
    /// Complex/photographic; needs a semantic provider.
    Semantic,
}

/// Structured inspection result emitted as JSON on stdout (PRD §7.1).
#[derive(Debug, Clone, Serialize)]
pub struct InspectResult {
    pub input: String,
    pub input_sha256: String,
    pub width: u32,
    pub height: u32,
    pub alpha_coverage: f32,
    pub has_usable_alpha: bool,
    pub foreground_ratio: f32,
    pub edge_foreground_ratio: f32,
    pub component_estimate: u32,
    pub touches_border: bool,
    /// True when the input looks like a multi-sprite sheet and should be sliced
    /// before conversion (PRD §0.2 pipeline entry).
    pub is_sprite_sheet: bool,
    /// Detected `[rows, cols]` grid when `is_sprite_sheet` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet_grid: Option<[u32; 2]>,
    pub suggested_mode: SuggestedMode,
    pub confidence: f32,
    pub warnings: Vec<String>,
}

/// Inspect an input image and produce structured metadata.
pub fn inspect(path: &Path, max_pixels: u64) -> Result<InspectResult, crate::error::CoreError> {
    let bytes = std::fs::read(path).map_err(|e| crate::error::CoreError::Io(e.to_string()))?;
    let input_sha256 = pixel_cache_sha(&bytes);
    let src = Bitmap::load(path, max_pixels)?;

    let cov = alpha_coverage(&src);
    let has_alpha = cov > 0.01;
    let fg = foreground_from_alpha(&src, 96);
    let fg_count = fg.count();
    let total = (src.width * src.height).max(1);
    let fg_ratio = fg_count as f32 / total as f32;

    let (edge_fg, edge_total) = edge_foreground(&src, &fg);
    let edge_ratio = if edge_total == 0 {
        0.0
    } else {
        edge_fg as f32 / edge_total as f32
    };
    let touches_border = edge_fg > 0;
    let components = count_components(&fg);

    // Sprite-sheet detection. Gutter analysis gives a best-effort grid; organic
    // art whose poses nearly touch may only yield one reliable axis. We treat a
    // detected grid OR a high component count as a sprite-sheet signal, but
    // always recommend an explicit `--grid` because auto-detection is
    // approximate for irregular silhouettes.
    let grid = detect_grid(&src);
    let many_components = components >= 6;
    let is_sprite_sheet = matches!(grid, Some((r, c)) if r * c >= 2) || many_components;
    let sheet_grid = grid.filter(|(r, c)| r * c >= 2).map(|(r, c)| [r, c]);

    let (mode, confidence) = if !has_alpha {
        (SuggestedMode::Semantic, 0.4)
    } else if edge_ratio > 0.1 {
        (SuggestedMode::FlatReconstruct, 0.5)
    } else {
        (SuggestedMode::FlatReconstruct, 0.8)
    };

    let mut warnings = Vec::new();
    if touches_border {
        warnings.push("foreground touches canvas border".into());
    }
    if let Some([r, c]) = sheet_grid {
        warnings.push(format!(
            "looks like a sprite sheet (~{r}x{c}); slice with `convert --grid ROWSxCOLS` (auto-detect is approximate for organic art)"
        ));
    } else if is_sprite_sheet {
        warnings.push(format!(
            "{components} components suggest a sprite sheet; slice with `convert --grid ROWSxCOLS` or `--cell WxH`"
        ));
    } else if components > 4 {
        warnings.push(format!("{components} foreground components detected"));
    }

    Ok(InspectResult {
        input: path.display().to_string(),
        input_sha256,
        width: src.width,
        height: src.height,
        alpha_coverage: cov,
        has_usable_alpha: has_alpha,
        foreground_ratio: fg_ratio,
        edge_foreground_ratio: edge_ratio,
        component_estimate: components,
        touches_border,
        is_sprite_sheet,
        sheet_grid,
        suggested_mode: mode,
        confidence,
        warnings,
    })
}

fn edge_foreground(src: &Bitmap, fg: &crate::bitmap::Mask) -> (u32, u32) {
    let (w, h) = (src.width, src.height);
    let mut count = 0;
    let mut total = 0;
    for x in 0..w {
        total += 2;
        if fg.get(x, 0) {
            count += 1;
        }
        if fg.get(x, h - 1) {
            count += 1;
        }
    }
    for y in 0..h {
        total += 2;
        if fg.get(0, y) {
            count += 1;
        }
        if fg.get(w - 1, y) {
            count += 1;
        }
    }
    (count, total)
}

fn pixel_cache_sha(bytes: &[u8]) -> String {
    pixel_cache::sha256_hex(bytes)
}
