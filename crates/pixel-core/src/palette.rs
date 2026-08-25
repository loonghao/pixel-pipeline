//! Deterministic palette quantization (PRD §7.6, §14.5).
//!
//! Median-cut in linear sRGB for representative colors, Oklab for nearest
//! match. No dithering by default (FR-PALETTE-003).

use crate::bitmap::{Bitmap, Mask};
use crate::oklab::{linear_to_srgb, oklab_distance_sq, rgb_to_oklab, srgb_to_linear};

/// Quantize the body pixels (where `mask` is set) to at most `max_colors`.
/// Returns the palette (sorted deterministically) and mutates `body` in place.
pub fn quantize(body: &mut Bitmap, mask: &Mask, max_colors: u32) -> Vec<[u8; 3]> {
    let mut pixels: Vec<[u8; 3]> = Vec::new();
    for y in 0..body.height {
        for x in 0..body.width {
            if mask.get(x, y) {
                let p = body.get(x, y);
                pixels.push([p[0], p[1], p[2]]);
            }
        }
    }
    if pixels.is_empty() {
        return Vec::new();
    }

    let mut palette = median_cut(&pixels, max_colors.max(1) as usize);
    // Deterministic order for stable palette hashes.
    palette.sort_unstable();

    // Map every body pixel to its nearest palette color in Oklab.
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
    palette
}

/// Median-cut quantization in linear sRGB space (deterministic).
fn median_cut(pixels: &[[u8; 3]], max_colors: usize) -> Vec<[u8; 3]> {
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![pixels.to_vec()];
    while boxes.len() < max_colors {
        // Pick the box with the largest channel range to split.
        let mut target = None;
        let mut max_range = 0i32;
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
        b.sort_unstable_by_key(|p| p[ch]);
        let mid = b.len() / 2;
        let hi = b.split_off(mid);
        boxes.push(b);
        boxes.push(hi);
    }
    boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(average_linear)
        .collect()
}

fn widest_channel(b: &[[u8; 3]]) -> (usize, i32) {
    let mut lo = [255i32; 3];
    let mut hi = [0i32; 3];
    for p in b {
        for c in 0..3 {
            lo[c] = lo[c].min(p[c] as i32);
            hi[c] = hi[c].max(p[c] as i32);
        }
    }
    let mut ch = 0;
    let mut range = -1;
    for c in 0..3 {
        if hi[c] - lo[c] > range {
            range = hi[c] - lo[c];
            ch = c;
        }
    }
    (ch, range)
}

fn average_linear(b: &Vec<[u8; 3]>) -> [u8; 3] {
    let mut acc = [0f64; 3];
    for p in b {
        for c in 0..3 {
            acc[c] += srgb_to_linear(p[c]) as f64;
        }
    }
    let n = b.len() as f64;
    [
        linear_to_srgb((acc[0] / n) as f32),
        linear_to_srgb((acc[1] / n) as f32),
        linear_to_srgb((acc[2] / n) as f32),
    ]
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
}
