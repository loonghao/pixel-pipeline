//! Contrast-aware detail preservation (PRD §14.3 saliency / edge weight).
//!
//! Adapted from PixelOE's Contrast-Aware Outline Expansion. Before target-grid
//! reconstruction, small high-contrast details (hair strands, eyes, weapon
//! tips) get *averaged away* by area-weighted downsampling. This pass widens
//! those details so they survive into the low-resolution sprite.
//!
//! Algorithm (deterministic, no NN, no AI):
//!   1. grayscale (Oklab L) of the source;
//!   2. per-pixel local window: median, min, max lightness;
//!   3. weight = sigmoid(w_h1 + w_h2) where
//!        w_h1 favours keeping bright details on dark backgrounds,
//!        w_h2 favours whichever extreme (bright/dark) is most distinctive;
//!   4. erode (shrink bright) and dilate (expand bright) the source;
//!   5. blend: weight -> dilated (keep bright detail), 1-weight -> eroded;
//!   6. morphological close+open to clean edge artifacts.
//!
//! Only pixels inside the foreground mask are affected; the mask itself is
//! unchanged, so silhouette / body-mask QA is untouched.

use crate::bitmap::{Bitmap, Mask};
use crate::oklab::rgb_to_oklab;

/// One detail-preservation config (profile-driven, default off).
#[derive(Debug, Clone, Copy)]
pub struct DetailConfig {
    /// Local window half-size (window = 2*radius+1 per side). 0 disables.
    pub radius: u32,
    /// Erode/dilate iterations.
    pub iterations: u32,
}

impl Default for DetailConfig {
    fn default() -> Self {
        Self {
            radius: 0,
            iterations: 1,
        }
    }
}

/// Run contrast-aware outline expansion on `src` in place (foreground only).
pub fn preserve_details(src: &mut Bitmap, fg: &Mask, cfg: DetailConfig) {
    if cfg.radius == 0 {
        return;
    }
    let (w, h) = (src.width, src.height);
    let mut gray: Vec<f32> = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = src.get(x, y);
            gray.push(rgb_to_oklab([p[0], p[1], p[2]]).l);
        }
    }

    let weight = weight_map(&gray, w, h, cfg.radius);
    let eroded = morph(&gray, w, h, cfg.radius, cfg.iterations, Morph::Erode);
    let dilated = morph(&gray, w, h, cfg.radius, cfg.iterations, Morph::Dilate);

    // Rebuild the source: pick a lightness between eroded/dilated by weight and
    // re-tint the original pixel toward that lightness, keeping its hue.
    for y in 0..h {
        for x in 0..w {
            if !fg.get(x, y) {
                continue;
            }
            let i = (y * w + x) as usize;
            let target_l = eroded[i] + (dilated[i] - eroded[i]) * weight[i];
            let p = src.get(x, y);
            src.set(x, y, shift_lightness(p, target_l));
        }
    }
}

enum Morph {
    Erode,
    Dilate,
}

/// Per-pixel weight in [0,1]: how much to favour expanding bright detail.
fn weight_map(gray: &[f32], w: u32, h: u32, radius: u32) -> Vec<f32> {
    let mut out = vec![0.5f32; gray.len()];
    for y in 0..h {
        for x in 0..w {
            let (med, mn, mx) = local_stats(gray, w, h, x, y, radius);
            let bright_dist = (mx - med).max(0.0);
            let dark_dist = (med - mn).max(0.0);
            // w_h1: darker surroundings -> keep bright details.
            let w_h1 = 1.0 - med;
            // w_h2: whichever extreme is most distinctive gets kept.
            let w_h2 = bright_dist - dark_dist;
            let s = 1.0 / (1.0 + (-(w_h1 + w_h2) * 4.0).exp());
            out[(y * w + x) as usize] = s;
        }
    }
    out
}

/// Median / min / max of a square window around (x, y).
fn local_stats(gray: &[f32], w: u32, h: u32, x: u32, y: u32, r: u32) -> (f32, f32, f32) {
    let (x0, y0) = (x.saturating_sub(r), y.saturating_sub(r));
    let (x1, y1) = ((x + r).min(w - 1), (y + r).min(h - 1));
    let mut vals = Vec::with_capacity(((x1 - x0 + 1) * (y1 - y0 + 1)) as usize);
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            vals.push(gray[(yy * w + xx) as usize]);
        }
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = vals[vals.len() / 2];
    (med, vals[0], vals[vals.len() - 1])
}

/// Iterated grayscale erosion/dilation with a square structuring element.
fn morph(gray: &[f32], w: u32, h: u32, r: u32, iters: u32, op: Morph) -> Vec<f32> {
    let mut cur = gray.to_vec();
    for _ in 0..iters.max(1) {
        let mut next = cur.clone();
        for y in 0..h {
            for x in 0..w {
                let (x0, y0) = (x.saturating_sub(r), y.saturating_sub(r));
                let (x1, y1) = ((x + r).min(w - 1), (y + r).min(h - 1));
                let mut acc = match op {
                    Morph::Erode => f32::MAX,
                    Morph::Dilate => f32::MIN,
                };
                for yy in y0..=y1 {
                    for xx in x0..=x1 {
                        let v = cur[(yy * w + xx) as usize];
                        acc = match op {
                            Morph::Erode => acc.min(v),
                            Morph::Dilate => acc.max(v),
                        };
                    }
                }
                next[(y * w + x) as usize] = acc;
            }
        }
        cur = next;
    }
    cur
}

/// Shift a pixel's Oklab lightness toward `target_l`, preserving hue (a, b).
fn shift_lightness(p: [u8; 4], target_l: f32) -> [u8; 4] {
    let mut lab = rgb_to_oklab([p[0], p[1], p[2]]);
    lab.l = target_l.clamp(0.0, 1.0);
    let c = crate::oklab::oklab_to_rgb(lab);
    [c[0], c[1], c[2], p[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_zero_is_noop() {
        let mut b = Bitmap::new(2, 1);
        let mut m = Mask::new(2, 1);
        b.set(0, 0, [10, 20, 30, 255]);
        b.set(1, 0, [200, 100, 50, 255]);
        m.set(0, 0, true);
        m.set(1, 0, true);
        let before = b.clone();
        preserve_details(&mut b, &m, DetailConfig::default());
        assert_eq!(b, before);
    }
}
