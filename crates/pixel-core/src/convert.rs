//! End-to-end deterministic conversion pipeline (PRD §6.1, §7).

use crate::bitmap::{Bitmap, Mask, DEFAULT_MAX_PIXELS};
use crate::compose::{compose_final, mask_to_bitmap, outline_to_bitmap};
use crate::error::CoreError;
use crate::grid::reconstruct;
use crate::internal_outline::compile_internal_outline;
use crate::mask::{
    alpha_coverage, count_components, foreground_from_alpha, foreground_from_corners,
    remove_small_components, MaskSource,
};
use crate::outline::compile_outline;
use crate::palette::{distinct_colors, posterize_lightness, quantize_with_lock};
use pixel_formats::color::parse_hex_color;
use pixel_formats::Profile;
use std::path::Path;

/// Options controlling a conversion run.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub max_pixels: u64,
    /// Detect identity-critical features (face/eyes) with the heuristic
    /// provider and weight/protect them during reconstruction (PRD §7.5
    /// FR-RECON-004). Off by default; enable for character sprites.
    pub detect_features: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            max_pixels: DEFAULT_MAX_PIXELS,
            detect_features: false,
        }
    }
}

/// All bitmaps and metrics produced by a conversion (before QA/report).
pub struct ConvertOutput {
    pub input_sha256: String,
    pub mask_source: MaskSource,
    pub final_png: Bitmap,
    pub body: Bitmap,
    pub body_mask_bitmap: Bitmap,
    pub outline_mask_bitmap: Bitmap,
    pub preview: Bitmap,
    pub body_mask: Mask,
    pub outline_mask: Mask,
    pub palette: Vec<[u8; 3]>,
    pub body_pixels: u32,
    pub outline_pixels: u32,
    pub internal_outline_pixels: u32,
    pub body_components: u32,
    pub palette_colors: u32,
    pub body_pixels_in_reserved_border: u32,
    pub alpha_binary: bool,
}

/// Run the full deterministic conversion for a single input file.
pub fn convert(
    input: &Path,
    profile: &Profile,
    opts: &ConvertOptions,
) -> Result<ConvertOutput, CoreError> {
    let bytes = std::fs::read(input).map_err(|e| CoreError::Io(e.to_string()))?;
    let input_sha256 = pixel_cache::sha256_hex(&bytes);
    let src = Bitmap::load(input, opts.max_pixels)?;
    convert_bitmap(src, input_sha256, profile, opts)
}

/// Run the full deterministic conversion for an already-decoded bitmap.
///
/// This is the in-memory entry point used by sprite-sheet slicing, where each
/// cell is a `Bitmap` rather than a file. `input_sha256` is the provenance
/// hash the caller wants recorded (e.g. sheet hash plus cell coordinates).
pub fn convert_bitmap(
    src: Bitmap,
    input_sha256: String,
    profile: &Profile,
    opts: &ConvertOptions,
) -> Result<ConvertOutput, CoreError> {
    // Foreground mask: prefer alpha, fall back to corner background (=> review).
    let (fg, mask_source) = if alpha_coverage(&src) > 0.01 {
        (
            foreground_from_alpha(&src, profile.alpha.threshold),
            MaskSource::Alpha,
        )
    } else {
        (
            foreground_from_corners(&src, profile.alpha.background_tolerance),
            MaskSource::CornerBackground,
        )
    };

    // Optional contrast-aware detail preservation (PRD §14.3). Runs on the
    // source *before* downsampling so small high-contrast features (hair
    // strands, eyes, weapon tips) survive into the low-resolution sprite. The
    // foreground mask is unchanged, so silhouette QA is unaffected.
    let mut src = src;
    if profile.detail.radius > 0 {
        crate::detail::preserve_details(
            &mut src,
            &fg,
            crate::detail::DetailConfig {
                radius: profile.detail.radius,
                iterations: profile.detail.iterations,
            },
        );
    }

    // Optional identity-critical feature detection (PRD §7.5 FR-RECON-004).
    // A Semantic Provider (here the deterministic heuristic fallback) marks
    // face/eye regions so the reconstructor weights them higher and the
    // palette stage can lock their colors. Providers only supply candidates;
    // they never bypass deterministic QA (DEC-006).
    let features = if opts.detect_features {
        let provider = crate::heuristic_provider::HeuristicProvider::default();
        let (map, _prov) = provider.analyze_bitmap(&src, &input_sha256);
        if map.is_empty() {
            None
        } else {
            Some(map)
        }
    } else {
        None
    };

    // Reconstruct onto the target grid, weighting identity-critical regions.
    let recon = reconstruct(&src, &fg, profile, features.as_ref());
    let mut body = recon.body;
    let mut body_mask = recon.body_mask;
    let feature_mask = recon.feature_mask;

    // Body-mask cleanup (PRD §14.4).
    remove_small_components(&mut body_mask, profile.cleanup.min_component_pixels);
    // Zero out body pixels dropped by cleanup so mask and body stay in sync.
    for y in 0..body.height {
        for x in 0..body.width {
            if !body_mask.get(x, y) {
                body.set(x, y, [0, 0, 0, 0]);
            }
        }
    }

    // Optional lightness posterization (cel-shading bands) before quantization.
    // We posterize only lightness (not chroma): hue separation is handled by the
    // Oklab median-cut quantizer, which keeps perceptually distinct hues (e.g.
    // white dress vs. skin) apart instead of collapsing them into one band.
    if profile.palette.posterize_levels >= 2 {
        posterize_lightness(&mut body, &body_mask, profile.palette.posterize_levels);
    }

    // Palette quantization. When internal outlines are enabled, reserve one
    // slot for the outline color so the recolored sprite never exceeds
    // `max_colors` (the internal-outline color is counted in the budget).
    let reserve_for_internal = if profile.outline.internal { 1 } else { 0 };
    let color_budget = profile
        .palette
        .max_colors
        .saturating_sub(reserve_for_internal)
        .max(1);
    // Lock identity-critical feature colors (face/eyes/...) so quantization
    // never merges them into a neighbour (FR-PALETTE-005). Only active when a
    // FeatureMap was supplied during reconstruction and the profile enables it.
    let lock = if profile.features.lock_feature_colors && feature_mask.count() > 0 {
        Some(&feature_mask)
    } else {
        None
    };
    let palette = quantize_with_lock(&mut body, &body_mask, lock, color_budget);

    // Outline compilation from the body mask.
    let outline_mask = compile_outline(&body_mask, profile);
    let outline_rgba = parse_hex_color(&profile.outline.color).map_err(CoreError::Invalid)?;

    // Optional internal outlines: recolor body pixels on perceptual boundaries
    // to the outline color. These stay inside the body mask, so they never
    // affect the external-outline QA gate (PRD §14.6).
    let mut internal_outline_pixels = 0;
    if profile.outline.internal {
        let edges = compile_internal_outline(&body, &body_mask, profile.outline.internal_threshold);
        for y in 0..body.height {
            for x in 0..body.width {
                if edges.get(x, y) {
                    body.set(x, y, outline_rgba);
                }
            }
        }
        internal_outline_pixels = edges.count();
    }

    // Count palette colors after any internal-outline recolor.
    let palette_colors = distinct_colors(&body, &body_mask);

    // Composition.
    let final_png = compose_final(&body, &outline_mask, outline_rgba);
    let body_mask_bitmap = mask_to_bitmap(&body_mask);
    let outline_mask_bitmap = outline_to_bitmap(&outline_mask, outline_rgba);
    let preview = final_png.upscale_nearest(preview_factor(profile));

    // Metrics.
    let body_pixels = body_mask.count();
    let outline_pixels = outline_mask.count();
    let body_components = count_components(&body_mask);
    let body_pixels_in_reserved_border = count_reserved_border_body(&body_mask, profile);
    let alpha_binary = is_alpha_binary(&final_png);

    Ok(ConvertOutput {
        input_sha256,
        mask_source,
        final_png,
        body,
        body_mask_bitmap,
        outline_mask_bitmap,
        preview,
        body_mask,
        outline_mask,
        palette,
        body_pixels,
        outline_pixels,
        internal_outline_pixels,
        body_components,
        palette_colors,
        body_pixels_in_reserved_border,
        alpha_binary,
    })
}

fn preview_factor(profile: &Profile) -> u32 {
    let max_dim = profile.target.width.max(profile.target.height).max(1);
    (256 / max_dim).max(1)
}

fn count_reserved_border_body(body_mask: &Mask, profile: &Profile) -> u32 {
    let reserved = profile.outline.width + profile.transparent_margin;
    if reserved == 0 {
        return 0;
    }
    let (w, h) = (body_mask.width, body_mask.height);
    let mut count = 0;
    for y in 0..h {
        for x in 0..w {
            let in_border = x < reserved || y < reserved || x >= w - reserved || y >= h - reserved;
            if in_border && body_mask.get(x, y) {
                count += 1;
            }
        }
    }
    count
}

fn is_alpha_binary(bmp: &Bitmap) -> bool {
    bmp.data.chunks_exact(4).all(|p| p[3] == 0 || p[3] == 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::from_toml(include_str!("../../../profiles/character-48.toml")).unwrap()
    }

    #[test]
    fn convert_produces_target_sized_binary_alpha_sprite() {
        // Write a small opaque square to a temp PNG, then convert it.
        let mut src = Bitmap::new(24, 24);
        for y in 4..20 {
            for x in 4..20 {
                src.set(x, y, [70, 130, 200, 255]);
            }
        }
        let path =
            std::env::temp_dir().join(format!("pixelpipe-convert-test-{}.png", std::process::id()));
        src.save_png(&path).unwrap();

        let p = profile();
        let out = convert(&path, &p, &ConvertOptions::default()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(out.final_png.width, p.target.width);
        assert_eq!(out.final_png.height, p.target.height);
        assert!(out.alpha_binary);
        assert!(out.body_pixels > 0);
        assert!(out.palette_colors <= p.palette.max_colors);
        assert_eq!(out.mask_source, MaskSource::Alpha);
        assert_eq!(out.body_pixels_in_reserved_border, 0);
    }
}
