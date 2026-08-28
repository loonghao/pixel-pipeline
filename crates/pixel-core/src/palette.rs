//! Deterministic palette quantization (PRD §7.6, §14.5).
//!
//! Median-cut in Oklab space for representative colors, Oklab for nearest
//! match. No dithering by default (FR-PALETTE-003).

use crate::bitmap::{Bitmap, Mask};
use crate::oklab::{oklab_distance_sq, oklab_to_rgb, rgb_to_oklab, Oklab};
use pixel_formats::{Dithering, FeatureMap};

/// Snap each body pixel's Oklab lightness to one of `levels` evenly spaced
/// bands, preserving chroma (a, b). This posterizes smooth shading into flat
/// cel-shaded steps *before* quantization, which is what gives hand-drawn
/// pixel art its banded look rather than a downscaled-photo gradient. `levels`
/// below 2 is a no-op (PRD §14.5 shading-band note; stylization layer).
pub fn posterize_lightness(body: &mut Bitmap, mask: &Mask, levels: u32) {
    if levels < 2 {
        return;
    }
    let steps = (levels - 1) as f32;
    for y in 0..body.height {
        for x in 0..body.width {
            if !mask.get(x, y) {
                continue;
            }
            let p = body.get(x, y);
            let mut lab = rgb_to_oklab([p[0], p[1], p[2]]);
            lab.l = (lab.l * steps).round() / steps;
            let c = oklab_to_rgb(lab);
            body.set(x, y, [c[0], c[1], c[2], p[3]]);
        }
    }
}

/// Posterize lightness *and* collapse chroma towards each band's average hue.
///
/// `posterize_lightness` alone keeps the original a/b per pixel, so a single
/// lightness band can still contain many slightly-different hues (visually
/// "noisy"). To get the large, single-hue flat regions of hand-drawn pixel art
/// we additionally snap each pixel's chroma to the mean chroma of its
/// lightness band, so every band becomes one consistent hue at one lightness.
///
/// NOTE: kept for future use. Averaging chroma across a whole lightness band
/// can merge perceptually distinct colours that happen to share a lightness
/// (e.g. a white dress and skin), so the current pipeline uses
/// `posterize_lightness` and lets the Oklab quantizer keep hues apart.
/// `levels < 2` is a no-op.
#[allow(dead_code)]
pub fn posterize_to_flat_bands(body: &mut Bitmap, mask: &Mask, levels: u32) {
    if levels < 2 {
        return;
    }
    // Pass 1: snap lightness into bands (same as posterize_lightness).
    posterize_lightness(body, mask, levels);

    // Pass 2: average the chroma (a, b) of each lightness band, then snap.
    let steps = (levels - 1) as f32;
    let mut sum = vec![(0f32, 0f32, 0u32); levels as usize]; // (a, b, count)
    for y in 0..body.height {
        for x in 0..body.width {
            if !mask.get(x, y) {
                continue;
            }
            let p = body.get(x, y);
            let lab = rgb_to_oklab([p[0], p[1], p[2]]);
            let band = (lab.l * steps).round().clamp(0.0, steps) as usize;
            sum[band].0 += lab.a;
            sum[band].1 += lab.b;
            sum[band].2 += 1;
        }
    }
    let mut mean_ab = vec![(0f32, 0f32); levels as usize];
    for (i, (sa, sb, n)) in sum.iter().enumerate() {
        if *n > 0 {
            mean_ab[i] = (sa / *n as f32, sb / *n as f32);
        }
    }
    for y in 0..body.height {
        for x in 0..body.width {
            if !mask.get(x, y) {
                continue;
            }
            let p = body.get(x, y);
            let mut lab = rgb_to_oklab([p[0], p[1], p[2]]);
            let band = (lab.l * steps).round().clamp(0.0, steps) as usize;
            lab.a = mean_ab[band].0;
            lab.b = mean_ab[band].1;
            let c = oklab_to_rgb(lab);
            body.set(x, y, [c[0], c[1], c[2], p[3]]);
        }
    }
}

/// Quantize the body pixels (where `mask` is set) to at most `max_colors`.
/// Returns the palette (sorted deterministically) and mutates `body` in place.
pub fn quantize(body: &mut Bitmap, mask: &Mask, max_colors: u32) -> Vec<[u8; 3]> {
    let pixels = collect_pixels(body, mask, None, false);
    if pixels.is_empty() {
        return Vec::new();
    }
    let mut palette = median_cut(&pixels, max_colors.max(1) as usize);
    palette.sort_unstable();
    palette.dedup();
    remap_to_palette(body, mask, &palette);
    palette
}

/// Quantize, optionally protecting identity-critical feature pixels (`lock`).
///
/// Feature regions (face/eyes, FR-PALETTE-005) get a *dedicated share* of the
/// color budget so their hues survive quantization instead of being merged
/// into a dominant neighbour (e.g. skin merged into hair). We run median-cut
/// separately over feature and non-feature pixels, each with its own budget,
/// then merge the two palettes. The total never exceeds `max_colors`.
pub fn quantize_with_lock(
    body: &mut Bitmap,
    mask: &Mask,
    lock: Option<&Mask>,
    max_colors: u32,
) -> Vec<[u8; 3]> {
    let has_lock = lock.map(|lm| lm.count() > 0).unwrap_or(false);
    if !has_lock {
        return quantize(body, mask, max_colors);
    }
    let lm = lock.unwrap();
    let budget = max_colors.max(1) as usize;

    // Split the budget: feature pixels get a guaranteed share (at least 1/3,
    // at most budget-1), the rest go to the body.
    let feature_budget = (budget / 3).clamp(1, budget.saturating_sub(1).max(1));
    let body_budget = budget.saturating_sub(feature_budget).max(1);

    // Median-cut each group separately so feature hues are preserved.
    let feature_pixels = collect_pixels(body, mask, Some(lm), true);
    let body_pixels = collect_pixels(body, mask, Some(lm), false);

    let mut palette: Vec<[u8; 3]> = Vec::new();
    if !feature_pixels.is_empty() {
        palette.extend(median_cut(&feature_pixels, feature_budget));
    }
    if !body_pixels.is_empty() {
        palette.extend(median_cut(&body_pixels, body_budget));
    }
    if palette.is_empty() {
        return Vec::new();
    }
    palette.sort_unstable();
    palette.dedup();
    remap_to_palette(body, mask, &palette);
    palette
}

/// Build a palette from *source-resolution* foreground pixels (PRD §14.5
/// quantize-then-snap order). At source resolution small identity-critical
/// regions (eyes, sunglasses, skin) still have thousands of pixels, so they
/// keep palette slots they would lose after downsampling. Pixels are weighted
/// by alpha × saliency, and — when a FeatureMap is present — feature pixels
/// additionally get a dedicated budget share (same policy as
/// `quantize_with_lock`). Deterministic; the caller remaps with
/// `remap_to_palette`.
pub fn build_source_palette(
    src: &Bitmap,
    fg: &Mask,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
    max_colors: u32,
) -> Vec<[u8; 3]> {
    let mut feature_px: Vec<([u8; 3], f32)> = Vec::new();
    let mut body_px: Vec<([u8; 3], f32)> = Vec::new();
    collect_weighted_pixels(
        src,
        fg,
        features,
        saliency_weight,
        &mut feature_px,
        &mut body_px,
    );
    palette_from_weighted(&feature_px, &body_px, max_colors)
}

/// Accumulate weighted source pixels into feature/body buckets. Used by the
/// single-sprite palette build and by the sheet-level shared build, which
/// aggregates every cell into one pair of buckets (the same design as
/// Aseprite's `create_palette_from_sprite`, which feeds all frames into a
/// single histogram before quantizing once).
pub fn collect_weighted_pixels(
    src: &Bitmap,
    fg: &Mask,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
    feature_px: &mut Vec<([u8; 3], f32)>,
    body_px: &mut Vec<([u8; 3], f32)>,
) {
    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false);
    for y in 0..src.height {
        for x in 0..src.width {
            if !fg.get(x, y) {
                continue;
            }
            let p = src.get(x, y);
            let a = p[3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let (critical, saliency) = if use_features {
                let f = features.unwrap();
                (
                    f.is_critical(x, y),
                    1.0 + (saliency_weight - 1.0).max(0.0) * f.weight_at(x, y),
                )
            } else {
                (false, 1.0)
            };
            let entry = ([p[0], p[1], p[2]], a * saliency);
            if critical {
                feature_px.push(entry);
            } else {
                body_px.push(entry);
            }
        }
    }
}

/// Build a palette from pre-collected weighted pixels, giving feature pixels a
/// dedicated budget share (same split policy as `quantize_with_lock`).
pub fn palette_from_weighted(
    feature_px: &[([u8; 3], f32)],
    body_px: &[([u8; 3], f32)],
    max_colors: u32,
) -> Vec<[u8; 3]> {
    if feature_px.is_empty() && body_px.is_empty() {
        return Vec::new();
    }
    let budget = max_colors.max(1) as usize;

    // Exact-color path (Aseprite's "high precision" histogram): when the
    // distinct source colors already fit the budget, keep them verbatim with
    // zero quantization loss — important when the source is already pixel art.
    let mut distinct = std::collections::BTreeSet::new();
    for (c, _) in feature_px.iter().chain(body_px.iter()) {
        distinct.insert(*c);
        if distinct.len() > budget {
            break;
        }
    }
    if distinct.len() <= budget {
        return distinct.into_iter().collect();
    }

    let mut palette: Vec<[u8; 3]> = if feature_px.is_empty() {
        median_cut_weighted(body_px, budget)
    } else {
        // Same split policy as `quantize_with_lock`: features get a guaranteed
        // share so their hues survive regardless of area.
        let feature_budget = (budget / 3).clamp(1, budget.saturating_sub(1).max(1));
        let body_budget = budget.saturating_sub(feature_budget).max(1);
        let mut p = median_cut_weighted(feature_px, feature_budget);
        if !body_px.is_empty() {
            p.extend(median_cut_weighted(body_px, body_budget));
        }
        p
    };
    palette.sort_unstable();
    palette.dedup();
    palette
}

/// Collect body-pixel colors, restricted to inside (`only_lock = true`) or
/// outside (`false`) the `lock` mask.
fn collect_pixels(
    body: &Bitmap,
    mask: &Mask,
    lock: Option<&Mask>,
    only_lock: bool,
) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    for y in 0..body.height {
        for x in 0..body.width {
            if !mask.get(x, y) {
                continue;
            }
            let is_locked = lock.map(|lm| lm.get(x, y)).unwrap_or(false);
            if is_locked == only_lock {
                let p = body.get(x, y);
                out.push([p[0], p[1], p[2]]);
            }
        }
    }
    out
}

/// Map every masked body pixel to its nearest palette color in Oklab.
pub fn remap_to_palette(body: &mut Bitmap, mask: &Mask, palette: &[[u8; 3]]) {
    if palette.is_empty() {
        return;
    }
    let pal_lab: Vec<_> = palette.iter().map(|c| rgb_to_oklab(*c)).collect();
    for y in 0..body.height {
        for x in 0..body.width {
            if !mask.get(x, y) {
                continue;
            }
            let p = body.get(x, y);
            let lab = rgb_to_oklab([p[0], p[1], p[2]]);
            let mut best = 0usize;
            let mut best_d = f32::MAX;
            for (i, pl) in pal_lab.iter().enumerate() {
                let d = oklab_distance_sq(lab, *pl);
                if d < best_d {
                    best_d = d;
                    best = i;
                }
            }
            let c = palette[best];
            body.set(x, y, [c[0], c[1], c[2], 255]);
        }
    }
}

/// 4×4 Bayer ordered-dither threshold matrix (values `0..16`).
#[rustfmt::skip]
const BAYER4: [u16; 16] = [
     0,  8,  2, 10,
    12,  4, 14,  6,
     3, 11,  1,  9,
    15,  7, 13,  5,
];

/// 8×8 Bayer ordered-dither threshold matrix (values `0..64`).
#[rustfmt::skip]
const BAYER8: [u16; 64] = [
     0, 32,  8, 40,  2, 34, 10, 42,
    48, 16, 56, 24, 50, 18, 58, 26,
    12, 44,  4, 36, 14, 46,  6, 38,
    60, 28, 52, 20, 62, 30, 54, 22,
     3, 35, 11, 43,  1, 33,  9, 41,
    51, 19, 59, 27, 49, 17, 57, 25,
    15, 47,  7, 39, 13, 45,  5, 37,
    63, 31, 55, 23, 61, 29, 53, 21,
];

/// Map every masked body pixel to its nearest palette color in Oklab, applying
/// deterministic ordered (Bayer) dithering to lightness first.
///
/// Before the nearest-palette search each pixel's Oklab lightness is nudged by
/// a threshold read from a Bayer matrix indexed by the pixel's canvas
/// coordinates (`±strength/2` at most). Because the threshold depends only on
/// `(x, y)`, output stays byte-identical across runs. On a smooth gradient this
/// pushes neighbouring pixels to alternating palette entries, so a shading ramp
/// reads as a stable stipple instead of a few hard bands — the classic
/// low-palette dithered look. `Dithering::None` (or non-positive `strength`)
/// falls back to the plain nearest-color remap.
pub fn remap_to_palette_dithered(
    body: &mut Bitmap,
    mask: &Mask,
    palette: &[[u8; 3]],
    dithering: Dithering,
    strength: f32,
) {
    if palette.is_empty() {
        return;
    }
    let (n, matrix): (u32, &[u16]) = match dithering {
        Dithering::None => {
            remap_to_palette(body, mask, palette);
            return;
        }
        Dithering::Bayer4x4 => (4, &BAYER4),
        Dithering::Bayer8x8 => (8, &BAYER8),
    };
    if strength <= 0.0 {
        remap_to_palette(body, mask, palette);
        return;
    }
    let pal_lab: Vec<_> = palette.iter().map(|c| rgb_to_oklab(*c)).collect();
    let denom = (n * n) as f32;
    for y in 0..body.height {
        for x in 0..body.width {
            if !mask.get(x, y) {
                continue;
            }
            let p = body.get(x, y);
            let mut lab = rgb_to_oklab([p[0], p[1], p[2]]);
            let v = matrix[((y % n) * n + (x % n)) as usize] as f32;
            // Centered threshold in [-0.5, 0.5) scaled by strength.
            let t = (v + 0.5) / denom - 0.5;
            lab.l += t * strength;
            let mut best = 0usize;
            let mut best_d = f32::MAX;
            for (i, pl) in pal_lab.iter().enumerate() {
                let d = oklab_distance_sq(lab, *pl);
                if d < best_d {
                    best_d = d;
                    best = i;
                }
            }
            let c = palette[best];
            body.set(x, y, [c[0], c[1], c[2], 255]);
        }
    }
}

/// Median-cut quantization in Oklab space (deterministic, FR-PALETTE-002).
///
/// We split boxes along the perceptually widest Oklab channel and average in
/// Oklab, so colours that *look* similar are merged into one palette entry
/// instead of being kept apart by RGB-space distance. This is what collapses a
/// smooth gradient into a few large flat regions (the "hand-drawn" look).
fn median_cut(pixels: &[[u8; 3]], max_colors: usize) -> Vec<[u8; 3]> {
    let weighted: Vec<([u8; 3], f32)> = pixels.iter().map(|p| (*p, 1.0)).collect();
    median_cut_weighted(&weighted, max_colors)
}

/// Weighted median-cut (saliency-aware, PRD §14.3): each pixel carries a
/// weight, boxes split at the *weighted* median and average by weight, so
/// identity-critical pixels pull splits and representatives toward their hues
/// even when their area is small.
fn median_cut_weighted(pixels: &[([u8; 3], f32)], max_colors: usize) -> Vec<[u8; 3]> {
    // Work in Oklab for both the split decision and the representative colour.
    let labs: Vec<(Oklab, f32)> = pixels.iter().map(|(p, w)| (rgb_to_oklab(*p), *w)).collect();
    let mut boxes: Vec<Vec<(Oklab, f32)>> = vec![labs];
    while boxes.len() < max_colors {
        // Pick the box with the largest Oklab channel range to split.
        let mut target = None;
        let mut max_range = 0f32;
        for (i, b) in boxes.iter().enumerate() {
            if b.len() < 2 {
                continue;
            }
            let (_, range) = widest_channel(b);
            if range > max_range {
                max_range = range;
                target = Some(i);
            }
        }
        let Some(i) = target else { break };
        let mut b = boxes.swap_remove(i);
        let (ch, _) = widest_channel(&b);
        b.sort_unstable_by(|a, c| channel(a.0, ch).partial_cmp(&channel(c.0, ch)).unwrap());
        // Weighted median: split where cumulative weight crosses half the total.
        let total: f64 = b.iter().map(|(_, w)| *w as f64).sum();
        let mut mid = b.len() / 2;
        let mut acc = 0f64;
        for (idx, (_, w)) in b.iter().enumerate() {
            acc += *w as f64;
            if acc * 2.0 >= total {
                mid = idx + 1;
                break;
            }
        }
        let mid = mid.clamp(1, b.len() - 1); // keep both halves non-empty
        let hi = b.split_off(mid);
        boxes.push(b);
        boxes.push(hi);
    }
    boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| average_oklab(b))
        .collect()
}

/// Extract Oklab channel `ch` (0=L, 1=a, 2=b) for sorting/comparison.
fn channel(c: Oklab, ch: usize) -> f32 {
    match ch {
        0 => c.l,
        1 => c.a,
        _ => c.b,
    }
}

/// Widest Oklab channel in a box: returns (channel_index, perceptual range).
fn widest_channel(b: &[(Oklab, f32)]) -> (usize, f32) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for (p, _) in b {
        for c in 0..3 {
            let v = channel(*p, c);
            lo[c] = lo[c].min(v);
            hi[c] = hi[c].max(v);
        }
    }
    let mut ch = 0;
    let mut range = -1f32;
    for c in 0..3 {
        if hi[c] - lo[c] > range {
            range = hi[c] - lo[c];
            ch = c;
        }
    }
    (ch, range)
}

/// Weighted average of a box of Oklab colours, converted back to sRGB.
fn average_oklab(b: &[(Oklab, f32)]) -> [u8; 3] {
    let (mut l, mut a, mut bb, mut wsum) = (0f64, 0f64, 0f64, 0f64);
    for (p, w) in b {
        let w = *w as f64;
        l += p.l as f64 * w;
        a += p.a as f64 * w;
        bb += p.b as f64 * w;
        wsum += w;
    }
    if wsum <= 0.0 {
        wsum = b.len().max(1) as f64;
    }
    oklab_to_rgb(Oklab {
        l: (l / wsum) as f32,
        a: (a / wsum) as f32,
        b: (bb / wsum) as f32,
    })
}

/// Count distinct opaque colors in a bitmap where mask is set.
pub fn distinct_colors(body: &Bitmap, mask: &Mask) -> u32 {
    let mut set = std::collections::BTreeSet::new();
    for y in 0..body.height {
        for x in 0..body.width {
            if mask.get(x, y) {
                let p = body.get(x, y);
                set.insert([p[0], p[1], p[2]]);
            }
        }
    }
    set.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_from_weighted_keeps_exact_colors_when_they_fit() {
        // Aseprite-style exact-color path: distinct colors within budget are
        // preserved verbatim (zero quantization loss).
        let body_px: Vec<([u8; 3], f32)> = vec![
            ([200, 40, 40], 1.0),
            ([40, 200, 40], 1.0),
            ([200, 40, 40], 1.0),
            ([10, 20, 30], 0.5),
        ];
        let pal = palette_from_weighted(&[], &body_px, 8);
        assert_eq!(pal, vec![[10, 20, 30], [40, 200, 40], [200, 40, 40]]);
    }

    #[test]
    fn quantize_respects_color_budget() {
        let mut body = Bitmap::new(4, 1);
        let mut mask = Mask::new(4, 1);
        for (x, c) in [[200, 20, 20], [20, 200, 20], [20, 20, 200], [200, 200, 20]]
            .into_iter()
            .enumerate()
        {
            body.set(x as u32, 0, [c[0], c[1], c[2], 255]);
            mask.set(x as u32, 0, true);
        }
        let palette = quantize(&mut body, &mask, 2);
        assert!(palette.len() <= 2);
        assert!(distinct_colors(&body, &mask) <= 2);
    }

    #[test]
    fn empty_mask_yields_empty_palette() {
        let mut body = Bitmap::new(2, 1);
        let mask = Mask::new(2, 1);
        assert!(quantize(&mut body, &mask, 8).is_empty());
    }

    #[test]
    fn posterize_collapses_a_gradient_into_few_bands() {
        // A 16-step gray ramp should collapse to at most `levels` distinct colors.
        let mut body = Bitmap::new(16, 1);
        let mut mask = Mask::new(16, 1);
        for x in 0..16u32 {
            let v = (x * 17) as u8; // 0,17,...,255
            body.set(x, 0, [v, v, v, 255]);
            mask.set(x, 0, true);
        }
        posterize_lightness(&mut body, &mask, 3);
        assert!(distinct_colors(&body, &mask) <= 3);
    }

    #[test]
    fn weighted_median_cut_preserves_small_weighted_cluster() {
        // 90% near-white pixels vs 10% dark pixels with 8x weight: with a
        // 2-color budget the dark cluster must keep its own palette entry.
        let mut pixels: Vec<([u8; 3], f32)> = Vec::new();
        for _ in 0..90 {
            pixels.push(([240, 236, 230], 1.0));
        }
        for _ in 0..10 {
            pixels.push(([30, 25, 20], 8.0));
        }
        let palette = median_cut_weighted(&pixels, 2);
        assert_eq!(palette.len(), 2);
        assert!(palette.iter().any(|c| c[0] < 90));
        assert!(palette.iter().any(|c| c[0] > 180));
    }

    #[test]
    fn build_source_palette_reserves_feature_colors() {
        // A large cream body with a small skin-tone feature patch: the skin
        // hue must survive into the palette thanks to the feature budget.
        let mut src = Bitmap::new(32, 32);
        let mut fg = Mask::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                src.set(x, y, [235, 228, 214, 255]); // cream
                fg.set(x, y, true);
            }
        }
        for y in 4..8 {
            for x in 4..8 {
                src.set(x, y, [222, 170, 130, 255]); // skin
            }
        }
        let features = FeatureMap {
            width: 32,
            height: 32,
            regions: vec![pixel_formats::FeatureRegion {
                kind: pixel_formats::FeatureKind::Face,
                bbox: (4, 4, 8, 8),
                confidence: 0.9,
            }],
        };
        let palette = build_source_palette(&src, &fg, Some(&features), 2.0, 4);
        // A warm skin-like entry (red clearly above blue) must be present.
        assert!(palette
            .iter()
            .any(|c| c[0] as i32 - c[2] as i32 > 40 && c[0] > 180));
    }

    #[test]
    fn build_source_palette_is_deterministic() {
        let mut src = Bitmap::new(16, 16);
        let mut fg = Mask::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                src.set(x, y, [(x * 16) as u8, (y * 16) as u8, 128, 255]);
                fg.set(x, y, true);
            }
        }
        let a = build_source_palette(&src, &fg, None, 1.0, 6);
        let b = build_source_palette(&src, &fg, None, 1.0, 6);
        assert_eq!(a, b);
        assert!(!a.is_empty() && a.len() <= 6);
    }

    #[test]
    fn dithering_none_matches_plain_remap() {
        let mut a = Bitmap::new(8, 8);
        let mut mask = Mask::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                a.set(x, y, [(x * 30) as u8, 100, (y * 30) as u8, 255]);
                mask.set(x, y, true);
            }
        }
        let mut b = a.clone();
        let palette = vec![[10, 10, 10], [240, 240, 240], [200, 40, 40]];
        remap_to_palette(&mut a, &mask, &palette);
        remap_to_palette_dithered(&mut b, &mask, &palette, Dithering::None, 0.06);
        assert_eq!(a, b);
    }

    #[test]
    fn bayer_dithering_mixes_palette_on_flat_midtone() {
        // A constant mid-gray with a black/white palette collapses to one flat
        // color without dithering; Bayer thresholds split it into a stipple of
        // both palette colors. Output must stay within the palette.
        let mut plain = Bitmap::new(4, 4);
        let mut mask = Mask::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                plain.set(x, y, [128, 128, 128, 255]);
                mask.set(x, y, true);
            }
        }
        let mut dithered = plain.clone();
        let palette = vec![[0, 0, 0], [255, 255, 255]];
        remap_to_palette(&mut plain, &mask, &palette);
        remap_to_palette_dithered(&mut dithered, &mask, &palette, Dithering::Bayer4x4, 0.5);
        assert_eq!(distinct_colors(&plain, &mask), 1);
        assert_eq!(distinct_colors(&dithered, &mask), 2);
        for y in 0..4 {
            for x in 0..4 {
                let p = dithered.get(x, y);
                assert!(palette.contains(&[p[0], p[1], p[2]]));
            }
        }
    }

    #[test]
    fn bayer_dithering_is_deterministic() {
        let mut a = Bitmap::new(8, 8);
        let mut mask = Mask::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                a.set(x, y, [120, 120, 120, 255]);
                mask.set(x, y, true);
            }
        }
        let mut b = a.clone();
        let palette = vec![[0, 0, 0], [255, 255, 255]];
        remap_to_palette_dithered(&mut a, &mask, &palette, Dithering::Bayer8x8, 0.4);
        remap_to_palette_dithered(&mut b, &mask, &palette, Dithering::Bayer8x8, 0.4);
        assert_eq!(a, b);
    }

    #[test]
    fn posterize_below_two_levels_is_noop() {
        let mut body = Bitmap::new(2, 1);
        let mut mask = Mask::new(2, 1);
        body.set(0, 0, [10, 20, 30, 255]);
        body.set(1, 0, [200, 100, 50, 255]);
        mask.set(0, 0, true);
        mask.set(1, 0, true);
        let before = body.clone();
        posterize_lightness(&mut body, &mask, 1);
        assert_eq!(body, before);
    }
}
