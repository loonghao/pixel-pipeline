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
//!      w_h1 favours keeping bright details on dark backgrounds,
//!      w_h2 favours whichever extreme (bright/dark) is most distinctive;
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

#[derive(Clone, Copy, PartialEq)]
enum Morph {
    Erode,
    Dilate,
}

/// Per-pixel weight in [0,1]: how much to favour expanding bright detail.
///
/// Uses O(n) separable sliding-window min/max and an O(n·(r+256)) Huang
/// sliding-histogram median instead of the naive O(n·r²·log r²) window sort.
fn weight_map(gray: &[f32], w: u32, h: u32, radius: u32) -> Vec<f32> {
    let mn = window_extremum(gray, w, h, radius, Morph::Erode);
    let mx = window_extremum(gray, w, h, radius, Morph::Dilate);
    let med = window_median(gray, w, h, radius);
    let mut out = vec![0.5f32; gray.len()];
    for i in 0..gray.len() {
        let bright_dist = (mx[i] - med[i]).max(0.0);
        let dark_dist = (med[i] - mn[i]).max(0.0);
        // w_h1: darker surroundings -> keep bright details.
        let w_h1 = 1.0 - med[i];
        // w_h2: whichever extreme is most distinctive gets kept.
        let w_h2 = bright_dist - dark_dist;
        out[i] = 1.0 / (1.0 + (-(w_h1 + w_h2) * 4.0).exp());
    }
    out
}

/// Sliding-window min/max along one line (stride-addressable), clamped window
/// `[i-r, i+r]`, via a monotonic index deque. O(n) per line.
fn slide_line(src: &[f32], dst: &mut [f32], n: usize, stride: usize, r: usize, op: Morph) {
    let better = |a: f32, b: f32| match op {
        Morph::Erode => a <= b,
        Morph::Dilate => a >= b,
    };
    let mut deque: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let mut added = 0usize; // next candidate index to add
    for i in 0..n {
        let hi = (i + r).min(n - 1);
        while added <= hi {
            let v = src[added * stride];
            while let Some(&back) = deque.back() {
                if better(v, src[back * stride]) {
                    deque.pop_back();
                } else {
                    break;
                }
            }
            deque.push_back(added);
            added += 1;
        }
        let lo = i.saturating_sub(r);
        while let Some(&front) = deque.front() {
            if front < lo {
                deque.pop_front();
            } else {
                break;
            }
        }
        dst[i * stride] = src[deque.front().copied().unwrap() * stride];
    }
}

/// Separable square-window min/max over the whole image. O(w·h).
fn window_extremum(gray: &[f32], w: u32, h: u32, r: u32, op: Morph) -> Vec<f32> {
    let (w, h, r) = (w as usize, h as usize, r as usize);
    let mut tmp = vec![0f32; gray.len()];
    let mut out = vec![0f32; gray.len()];
    for y in 0..h {
        slide_line(&gray[y * w..], &mut tmp[y * w..], w, 1, r, op);
    }
    for x in 0..w {
        slide_line(&tmp[x..], &mut out[x..], h, w, r, op);
    }
    out
}

/// Square-window median via Huang's sliding histogram over 256 lightness bins.
/// Matches the previous upper-median (`vals[len/2]`) at bin precision.
fn window_median(gray: &[f32], w: u32, h: u32, r: u32) -> Vec<f32> {
    let (w, h, r) = (w as usize, h as usize, r as usize);
    let bins: Vec<u8> = gray
        .iter()
        .map(|l| (l.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    let mut out = vec![0f32; gray.len()];
    let mut hist = [0u32; 256];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r).min(h - 1));
        hist.fill(0);
        let mut count = 0u32;
        // Seed the window for x = 0: columns [0, min(r, w-1)].
        for yy in y0..=y1 {
            for xx in 0..=r.min(w - 1) {
                hist[bins[yy * w + xx] as usize] += 1;
                count += 1;
            }
        }
        for x in 0..w {
            if x > 0 {
                // Add the entering column, drop the leaving column.
                if x + r < w {
                    for yy in y0..=y1 {
                        hist[bins[yy * w + x + r] as usize] += 1;
                        count += 1;
                    }
                }
                if x > r {
                    for yy in y0..=y1 {
                        hist[bins[yy * w + x - r - 1] as usize] -= 1;
                        count -= 1;
                    }
                }
            }
            // Upper median: rank len/2 (0-based) => cumulative > len/2.
            let rank = count / 2;
            let mut cum = 0u32;
            let mut med_bin = 0usize;
            for (b, &c) in hist.iter().enumerate() {
                cum += c;
                if cum > rank {
                    med_bin = b;
                    break;
                }
            }
            out[y * w + x] = med_bin as f32 / 255.0;
        }
    }
    out
}

/// Iterated grayscale erosion/dilation with a square structuring element,
/// using the separable O(n) sliding-window pass per iteration.
fn morph(gray: &[f32], w: u32, h: u32, r: u32, iters: u32, op: Morph) -> Vec<f32> {
    let mut cur = gray.to_vec();
    for _ in 0..iters.max(1) {
        cur = window_extremum(&cur, w, h, r, op);
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

    /// Deterministic pseudo-random grayscale test image.
    fn test_gray(w: u32, h: u32) -> Vec<f32> {
        let mut v = Vec::with_capacity((w * h) as usize);
        let mut s = 0x9e3779b9u32;
        for _ in 0..w * h {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            v.push(((s >> 8) & 0xff) as f32 / 255.0);
        }
        v
    }

    fn naive_extremum(g: &[f32], w: u32, h: u32, r: u32, op: Morph) -> Vec<f32> {
        let mut out = vec![0f32; g.len()];
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
                        let v = g[(yy * w + xx) as usize];
                        acc = match op {
                            Morph::Erode => acc.min(v),
                            Morph::Dilate => acc.max(v),
                        };
                    }
                }
                out[(y * w + x) as usize] = acc;
            }
        }
        out
    }

    #[test]
    fn separable_extremum_matches_naive() {
        let (w, h) = (13, 9);
        let g = test_gray(w, h);
        for r in [1, 2, 4] {
            for op in [Morph::Erode, Morph::Dilate] {
                let fast = window_extremum(&g, w, h, r, op);
                let naive = naive_extremum(&g, w, h, r, op);
                assert_eq!(fast, naive, "r={r}");
            }
        }
    }

    #[test]
    fn histogram_median_matches_naive_at_bin_precision() {
        let (w, h) = (11, 7);
        // Quantize the input to bin precision so both paths agree exactly.
        let g: Vec<f32> = test_gray(w, h)
            .iter()
            .map(|l| (l * 255.0).round() / 255.0)
            .collect();
        for r in [1, 3] {
            let fast = window_median(&g, w, h, r);
            for y in 0..h {
                for x in 0..w {
                    let (x0, y0) = (x.saturating_sub(r), y.saturating_sub(r));
                    let (x1, y1) = ((x + r).min(w - 1), (y + r).min(h - 1));
                    let mut vals = Vec::new();
                    for yy in y0..=y1 {
                        for xx in x0..=x1 {
                            vals.push(g[(yy * w + xx) as usize]);
                        }
                    }
                    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let expect = vals[vals.len() / 2];
                    let got = fast[(y * w + x) as usize];
                    assert!(
                        (got - expect).abs() < 1.0 / 255.0 / 2.0 + 1e-6,
                        "r={r} ({x},{y}): {got} vs {expect}"
                    );
                }
            }
        }
    }

    #[test]
    fn preserve_details_is_deterministic() {
        let (w, h) = (16u32, 16u32);
        let mut a = Bitmap::new(w, h);
        let mut m = Mask::new(w, h);
        let g = test_gray(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = (g[(y * w + x) as usize] * 255.0) as u8;
                a.set(x, y, [v, v / 2, 255 - v, 255]);
                m.set(x, y, true);
            }
        }
        let mut b = a.clone();
        let cfg = DetailConfig {
            radius: 3,
            iterations: 1,
        };
        preserve_details(&mut a, &m, cfg);
        preserve_details(&mut b, &m, cfg);
        assert_eq!(a, b);
    }
}
