//! Deterministic palette quantization (PRD §7.6, §14.5).
//!
//! Median-cut in Oklab space for representative colors, Oklab for nearest
//! match. No dithering by default (FR-PALETTE-003).

use crate::bitmap::{Bitmap, Mask};
use crate::oklab::{oklab_distance_sq, oklab_to_rgb, rgb_to_oklab, Oklab};

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
fn remap_to_palette(body: &mut Bitmap, mask: &Mask, palette: &[[u8; 3]]) {
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

/// Median-cut quantization in Oklab space (deterministic, FR-PALETTE-002).
///
/// We split boxes along the perceptually widest Oklab channel and average in
/// Oklab, so colours that *look* similar are merged into one palette entry
/// instead of being kept apart by RGB-space distance. This is what collapses a
/// smooth gradient into a few large flat regions (the "hand-drawn" look).
fn median_cut(pixels: &[[u8; 3]], max_colors: usize) -> Vec<[u8; 3]> {
    // Work in Oklab for both the split decision and the representative colour.
    let labs: Vec<Oklab> = pixels.iter().map(|p| rgb_to_oklab(*p)).collect();
    let mut boxes: Vec<Vec<Oklab>> = vec![labs];
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
        b.sort_unstable_by(|a, c| channel(*a, ch).partial_cmp(&channel(*c, ch)).unwrap());
        let mid = b.len() / 2;
        let hi = b.split_off(mid);
        boxes.push(b);
        boxes.push(hi);
    }
    boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(average_oklab)
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
fn widest_channel(b: &[Oklab]) -> (usize, f32) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in b {
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

/// Average a box of Oklab colours and convert the representative back to sRGB.
fn average_oklab(b: &Vec<Oklab>) -> [u8; 3] {
    let n = b.len() as f32;
    let (mut l, mut a, mut bb) = (0f32, 0f32, 0f32);
    for p in b {
        l += p.l;
        a += p.a;
        bb += p.b;
    }
    oklab_to_rgb(Oklab {
        l: l / n,
        a: a / n,
        b: bb / n,
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
