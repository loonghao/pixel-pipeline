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
use crate::palette::{
    build_source_palette, collect_weighted_pixels, distinct_colors, palette_from_weighted,
    posterize_lightness, quantize_with_lock, remap_to_palette, remap_to_palette_dithered,
};
use pixel_formats::color::parse_hex_color;
use pixel_formats::{Dithering, FeatureMap, Profile};
use std::path::Path;

/// Options controlling a conversion run.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub max_pixels: u64,
    /// Detect identity-critical features (face/eyes) with the heuristic
    /// provider and weight/protect them during reconstruction (PRD §7.5
    /// FR-RECON-004). Off by default; enable for character sprites.
    pub detect_features: bool,
    /// Palette shared across all cells of a sprite sheet. When set, per-sprite
    /// palette building is skipped and every frame maps to this palette, so
    /// animations never flicker between frame-local palettes (Aseprite: one
    /// palette from all frames plus a fixed lookup map). Built by
    /// `build_sheet_palette`.
    pub shared_palette: Option<Vec<[u8; 3]>>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            max_pixels: DEFAULT_MAX_PIXELS,
            detect_features: false,
            shared_palette: None,
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
    let prep = prepare_source(src, &input_sha256, profile, opts);
    let PreparedSource {
        mut src,
        fg,
        mask_source,
        features,
    } = prep;

    // Palette color budget. When internal outlines are enabled, reserve one
    // slot for the outline color so the recolored sprite never exceeds
    // `max_colors` (the internal-outline color is counted in the budget).
    let color_budget = palette_budget(profile);

    // Sheet-shared palette (Aseprite's "New Palette from Sprite"): built once
    // from every cell, mapped identically in every frame.
    let shared = opts.shared_palette.as_deref().filter(|p| !p.is_empty());

    // Quantize-then-snap order (PRD §14.5, spritefusion-style): posterize and
    // build the palette at *source* resolution — where small identity-critical
    // regions still have thousands of pixels and win palette slots — then
    // remap the source to that palette before downsampling. Pairs with
    // `sampling.mode = "mode"` so each cell votes among exact palette colors.
    let mut source_palette: Vec<[u8; 3]> = Vec::new();
    if profile.palette.quantize_source {
        source_palette = match shared {
            Some(p) => p.to_vec(),
            None => build_source_palette(
                &src,
                &fg,
                features.as_ref(),
                profile.features.saliency_weight,
                color_budget,
            ),
        };
        remap_to_palette(&mut src, &fg, &source_palette);
    }

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
    // Skipped when quantize_source already posterized the source.
    if profile.palette.posterize_levels >= 2 && !profile.palette.quantize_source {
        posterize_lightness(&mut body, &body_mask, profile.palette.posterize_levels);
    }

    // Lock identity-critical feature colors (face/eyes/...) so quantization
    // never merges them into a neighbour (FR-PALETTE-005). Only active when a
    // FeatureMap was supplied during reconstruction and the profile enables it.
    let lock = if profile.features.lock_feature_colors && feature_mask.count() > 0 {
        Some(&feature_mask)
    } else {
        None
    };
    // Keep the pre-quantization colors: the optimize pass refines the palette
    // against them (Lloyd iterations on already-snapped colors are a no-op).
    let reference = body.clone();
    let mut palette = if profile.palette.quantize_source {
        // The source was already quantized; snap any sampler round-off back to
        // the source palette instead of re-quantizing the tiny grid.
        remap_to_palette(&mut body, &body_mask, &source_palette);
        source_palette
    } else if let Some(p) = shared {
        // Sheet-shared palette on the legacy order: map the downsampled body
        // onto the palette built from every cell of the sheet.
        remap_to_palette(&mut body, &body_mask, p);
        p.to_vec()
    } else {
        quantize_with_lock(&mut body, &body_mask, lock, color_budget)
    };

    // Optional ordered dithering (opt-in): map the smooth pre-quantization
    // colors onto the final palette with a deterministic Bayer threshold, so
    // shading gradients become a stable stipple instead of a few flat bands.
    // It reads the `reference` (pre-quant) colors — dithering already-snapped
    // flat colors would be a no-op — and supersedes the convergence cleanup,
    // which would otherwise treat the stipple as orphan/jaggy noise. Hence the
    // optimize pass below is skipped whenever dithering is active.
    let dithered = profile.palette.dithering != Dithering::None;
    if dithered {
        body = reference.clone();
        remap_to_palette_dithered(
            &mut body,
            &body_mask,
            &palette,
            profile.palette.dithering,
            profile.palette.dither_strength,
        );
    }

    // Optional pixel-art convergence pass (PRD §14.5): Lloyd palette
    // refinement + orphan absorption + jaggy cleanup. Deterministic, palette
    // can only shrink, feature pixels are never rewritten. Per-frame Lloyd
    // refinement would drift a sheet-shared palette between frames, so it is
    // disabled when one is in use (Aseprite keeps one fixed lookup map per
    // palette); the pixel-level cleanup steps remain active.
    let mut opt_cfg = profile.optimize;
    if shared.is_some() {
        opt_cfg.palette_iterations = 0;
    }
    if !dithered
        && (opt_cfg.palette_iterations > 0 || opt_cfg.merge_orphans || opt_cfg.jaggy_cleanup)
    {
        crate::optimize::run_optimize(
            &mut body,
            &reference,
            &body_mask,
            lock,
            &mut palette,
            &opt_cfg,
        );
    }

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

/// A source image after the deterministic preparation steps shared by the
/// per-sprite pipeline and the sheet-level palette pre-pass: foreground mask,
/// detail preservation, feature detection and (for the quantize-then-snap
/// order) posterization. Both passes see identical pixels by construction.
struct PreparedSource {
    src: Bitmap,
    fg: Mask,
    mask_source: MaskSource,
    features: Option<FeatureMap>,
}

fn prepare_source(
    src: Bitmap,
    input_sha256: &str,
    profile: &Profile,
    opts: &ConvertOptions,
) -> PreparedSource {
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
        let (map, _prov) = provider.analyze_bitmap(&src, input_sha256);
        if map.is_empty() {
            None
        } else {
            Some(map)
        }
    } else {
        None
    };

    // For the quantize-then-snap order, posterize at source resolution so the
    // palette is built over the banded colors it will have to represent.
    if profile.palette.quantize_source && profile.palette.posterize_levels >= 2 {
        posterize_lightness(&mut src, &fg, profile.palette.posterize_levels);
    }

    PreparedSource {
        src,
        fg,
        mask_source,
        features,
    }
}

/// Palette color budget: when internal outlines are enabled, one slot is
/// reserved for the outline color so the recolored sprite never exceeds
/// `max_colors`.
fn palette_budget(profile: &Profile) -> u32 {
    let reserve_for_internal = if profile.outline.internal { 1 } else { 0 };
    profile
        .palette
        .max_colors
        .saturating_sub(reserve_for_internal)
        .max(1)
}

/// Build one palette shared by every cell of a sprite sheet (FR-PALETTE-002;
/// the design mirrors Aseprite's `create_palette_from_sprite`, which feeds
/// all frames into a single histogram before quantizing once). Each cell runs
/// the same deterministic source preparation as `convert_bitmap`, so the
/// aggregated pixels are exactly what the per-cell conversions will see.
/// Returns an empty palette when the cells have no foreground pixels; pass
/// the result via `ConvertOptions::shared_palette`.
pub fn build_sheet_palette(
    cells: &[&Bitmap],
    profile: &Profile,
    opts: &ConvertOptions,
) -> Vec<[u8; 3]> {
    let mut feature_px: Vec<([u8; 3], f32)> = Vec::new();
    let mut body_px: Vec<([u8; 3], f32)> = Vec::new();
    for cell in cells {
        let prep = prepare_source((*cell).clone(), "", profile, opts);
        collect_weighted_pixels(
            &prep.src,
            &prep.fg,
            prep.features.as_ref(),
            profile.features.saliency_weight,
            &mut feature_px,
            &mut body_px,
        );
    }
    palette_from_weighted(&feature_px, &body_px, palette_budget(profile))
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

    #[test]
    fn sheet_shared_palette_keeps_frames_consistent() {
        // Two cells with the same colors in different proportions: converted
        // independently they would each build a frame-local palette; with a
        // shared palette every output color must come from that one palette.
        let mut a = Bitmap::new(24, 24);
        let mut b = Bitmap::new(24, 24);
        for y in 4..20 {
            for x in 4..20 {
                let ca = if x < 12 {
                    [200, 40, 40, 255]
                } else {
                    [40, 40, 200, 255]
                };
                let cb = if y < 6 {
                    [40, 200, 40, 255]
                } else {
                    [200, 40, 40, 255]
                };
                a.set(x, y, ca);
                b.set(x, y, cb);
            }
        }
        let p = profile();
        let base_opts = ConvertOptions::default();
        let shared = build_sheet_palette(&[&a, &b], &p, &base_opts);
        assert!(!shared.is_empty());
        assert!(shared.len() as u32 <= p.palette.max_colors);

        let opts = ConvertOptions {
            shared_palette: Some(shared.clone()),
            ..base_opts
        };
        let oa = convert_bitmap(a, "a".into(), &p, &opts).unwrap();
        let ob = convert_bitmap(b, "b".into(), &p, &opts).unwrap();
        assert_eq!(oa.palette, ob.palette);
        for out in [&oa, &ob] {
            for y in 0..out.body.height {
                for x in 0..out.body.width {
                    if out.body_mask.get(x, y) {
                        let px = out.body.get(x, y);
                        assert!(shared.contains(&[px[0], px[1], px[2]]));
                    }
                }
            }
        }
    }

    #[test]
    fn build_sheet_palette_is_deterministic() {
        let mut a = Bitmap::new(16, 16);
        for y in 2..14 {
            for x in 2..14 {
                a.set(x, y, [10 * x as u8 + 30, 60, 200 - 5 * y as u8, 255]);
            }
        }
        let p = profile();
        let opts = ConvertOptions::default();
        let p1 = build_sheet_palette(&[&a], &p, &opts);
        let p2 = build_sheet_palette(&[&a], &p, &opts);
        assert_eq!(p1, p2);
        assert!(!p1.is_empty());
    }
}
