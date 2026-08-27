//! Pixel-art convergence pass (post-quantization refinement).
//!
//! Hand-drawn pixel art differs from a "filtered" downscale in that every
//! pixel is a deliberate decision. This module closes part of that gap with
//! three deterministic steps (Gerstner-style alternating refinement, plus
//! cluster-cleanliness rules):
//!
//!   1. `refine_palette`: Lloyd iterations alternating palette refinement and
//!      pixel re-assignment in Oklab, seeded by the median-cut palette and
//!      driven by the *pre-quantization* colors;
//!   2. orphan absorption: a pixel with no same-color neighbour is merged into
//!      its dominant neighbouring color;
//!   3. jaggy cleanup: single-pixel stair artifacts on color boundaries are
//!      absorbed into the surrounding region.
//!
//! All steps only reuse existing palette colors (the palette can shrink but
//! never grow) and never touch identity-critical feature pixels (`skip`).

use crate::bitmap::{Bitmap, Mask, Rgba};
use crate::oklab::{oklab_distance_sq, oklab_to_rgb, rgb_to_oklab, Oklab};

/// Refine `palette` with `iterations` Lloyd steps and remap `body` to it.
///
/// `reference` holds the pre-quantization body colors (post-posterize); using
/// them instead of the already-remapped `body` is what makes the iteration
/// meaningful. Pixels in `lock` (and palette entries exactly matching their
/// colors) are frozen so identity-critical colors never drift (FR-PALETTE-005).
pub fn refine_palette(
    body: &mut Bitmap,
    reference: &Bitmap,
    mask: &Mask,
    lock: Option<&Mask>,
    palette: &mut Vec<[u8; 3]>,
    iterations: u32,
) {
    if palette.is_empty() || iterations == 0 {
        return;
    }
    // Freeze palette entries that serve locked pixels (their bitmap colors
    // must remain exact palette members or the color count could grow).
    let mut frozen = vec![false; palette.len()];
    if let Some(lm) = lock {
        for y in 0..body.height {
            for x in 0..body.width {
                if mask.get(x, y) && lm.get(x, y) {
                    let p = body.get(x, y);
                    for (i, c) in palette.iter().enumerate() {
                        if *c == [p[0], p[1], p[2]] {
                            frozen[i] = true;
                        }
                    }
                }
            }
        }
    }

    let mut pal_lab: Vec<Oklab> = palette.iter().map(|c| rgb_to_oklab(*c)).collect();
    for _ in 0..iterations {
        let mut sums = vec![(0f64, 0f64, 0f64, 0u64); pal_lab.len()];
        for y in 0..body.height {
            for x in 0..body.width {
                if !mask.get(x, y) || lock.map(|lm| lm.get(x, y)).unwrap_or(false) {
                    continue;
                }
                let p = reference.get(x, y);
                let lab = rgb_to_oklab([p[0], p[1], p[2]]);
                let i = nearest(&pal_lab, lab);
                let s = &mut sums[i];
                s.0 += lab.l as f64;
                s.1 += lab.a as f64;
                s.2 += lab.b as f64;
                s.3 += 1;
            }
        }
        for (i, s) in sums.iter().enumerate() {
            if !frozen[i] && s.3 > 0 {
                pal_lab[i] = Oklab {
                    l: (s.0 / s.3 as f64) as f32,
                    a: (s.1 / s.3 as f64) as f32,
                    b: (s.2 / s.3 as f64) as f32,
                };
            }
        }
    }

    let mut refined: Vec<[u8; 3]> = pal_lab.iter().map(|l| oklab_to_rgb(*l)).collect();
    refined.sort_unstable();
    refined.dedup();
    let refined_lab: Vec<Oklab> = refined.iter().map(|c| rgb_to_oklab(*c)).collect();
    for y in 0..body.height {
        for x in 0..body.width {
            if !mask.get(x, y) || lock.map(|lm| lm.get(x, y)).unwrap_or(false) {
                continue;
            }
            let p = reference.get(x, y);
            let i = nearest(&refined_lab, rgb_to_oklab([p[0], p[1], p[2]]));
            let c = refined[i];
            body.set(x, y, [c[0], c[1], c[2], 255]);
        }
    }
    *palette = refined;
}

/// Nearest palette index in Oklab; ties break toward the lower index.
fn nearest(pal: &[Oklab], lab: Oklab) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::MAX;
    for (i, p) in pal.iter().enumerate() {
        let d = oklab_distance_sq(lab, *p);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// Counters reported by the convergence pass (for the conversion report).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OptimizeStats {
    pub orphans_merged: u32,
    pub jaggies_fixed: u32,
}

/// Run the full convergence pass per the profile `[optimize]` section.
///
/// `reference` holds the pre-quantization body colors (see `refine_palette`).
/// `skip` marks identity-critical pixels that must never be rewritten.
/// Cleanup rules repeat until stable (max 4 passes) so cascading orphans
/// settle; every pass is deterministic (reads a snapshot, writes the live
/// bitmap in row-major order).
pub fn run_optimize(
    body: &mut Bitmap,
    reference: &Bitmap,
    mask: &Mask,
    skip: Option<&Mask>,
    palette: &mut Vec<[u8; 3]>,
    cfg: &pixel_formats::OptimizeConfigToml,
) -> OptimizeStats {
    let mut stats = OptimizeStats::default();
    if cfg.palette_iterations > 0 {
        refine_palette(body, reference, mask, skip, palette, cfg.palette_iterations);
    }
    for _ in 0..4 {
        let mut changed = 0u32;
        if cfg.merge_orphans {
            let n = merge_orphans(body, mask, skip);
            stats.orphans_merged += n;
            changed += n;
        }
        if cfg.jaggy_cleanup {
            let n = jaggy_cleanup(body, mask, skip);
            stats.jaggies_fixed += n;
            changed += n;
        }
        if changed == 0 {
            break;
        }
    }
    stats
}

/// True when `(x, y)` may be rewritten by a cleanup rule.
fn writable(mask: &Mask, skip: Option<&Mask>, x: u32, y: u32) -> bool {
    mask.get(x, y) && !skip.map(|s| s.get(x, y)).unwrap_or(false)
}

/// In-mask 8-neighbour colors of `(x, y)` read from `snap`.
fn neighbor_colors(snap: &Bitmap, mask: &Mask, x: u32, y: u32, diag: bool) -> Vec<Rgba> {
    let mut out = Vec::with_capacity(8);
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if (dx == 0 && dy == 0) || (!diag && dx != 0 && dy != 0) {
                continue;
            }
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            if nx < 0 || ny < 0 || nx >= snap.width as i64 || ny >= snap.height as i64 {
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);
            if mask.get(nx, ny) {
                out.push(snap.get(nx, ny));
            }
        }
    }
    out
}

/// Most frequent color in `colors`; ties break toward the smallest RGBA tuple.
fn dominant_color(colors: &[Rgba]) -> Option<(Rgba, usize)> {
    let mut uniq: Vec<Rgba> = colors.to_vec();
    uniq.sort_unstable();
    uniq.dedup();
    let mut best: Option<(Rgba, usize)> = None;
    for c in uniq {
        let n = colors.iter().filter(|&&x| x == c).count();
        if best.map(|(_, bn)| n > bn).unwrap_or(true) {
            best = Some((c, n));
        }
    }
    best
}

/// Absorb pixels with no same-color 8-neighbour into the dominant
/// neighbouring color (cluster cleanliness: no lone noise pixels).
fn merge_orphans(body: &mut Bitmap, mask: &Mask, skip: Option<&Mask>) -> u32 {
    let snap = body.clone();
    let mut changed = 0u32;
    for y in 0..body.height {
        for x in 0..body.width {
            if !writable(mask, skip, x, y) {
                continue;
            }
            let me = snap.get(x, y);
            let neighbors = neighbor_colors(&snap, mask, x, y, true);
            if neighbors.is_empty() || neighbors.contains(&me) {
                continue;
            }
            if let Some((c, _)) = dominant_color(&neighbors) {
                body.set(x, y, c);
                changed += 1;
            }
        }
    }
    changed
}

/// Absorb single-pixel stair artifacts (jaggies): a pixel protruding from a
/// flat color edge joins the dominant neighbouring region.
///
/// A protrusion has exactly one same-color 4-neighbour whose two adjacent
/// diagonals are also same-color (a flat edge behind it), while >= 3 of its
/// 4-neighbours share one other color. Line ends and diagonal-line stairs do
/// not satisfy the flat-edge condition, so deliberate 1px details survive.
fn jaggy_cleanup(body: &mut Bitmap, mask: &Mask, skip: Option<&Mask>) -> u32 {
    let snap = body.clone();
    let mut changed = 0u32;
    let same_at = |snap: &Bitmap, me: Rgba, x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= snap.width as i64 || y >= snap.height as i64 {
            return false;
        }
        let (x, y) = (x as u32, y as u32);
        mask.get(x, y) && snap.get(x, y) == me
    };
    for y in 0..body.height {
        for x in 0..body.width {
            if !writable(mask, skip, x, y) {
                continue;
            }
            let me = snap.get(x, y);
            // Exactly one same-color 4-neighbour, pointing "back" to the edge.
            let dirs = [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)];
            let mut back: Option<(i64, i64)> = None;
            let mut same4 = 0;
            for (dx, dy) in dirs {
                if same_at(&snap, me, x as i64 + dx, y as i64 + dy) {
                    same4 += 1;
                    back = Some((dx, dy));
                }
            }
            if same4 != 1 {
                continue;
            }
            let (bx, by) = back.unwrap();
            // Flat edge: the two diagonals flanking the back neighbour must
            // also be same-color.
            let (px, py) = (by.abs(), bx.abs()); // perpendicular axis
            if !same_at(&snap, me, x as i64 + bx + px, y as i64 + by + py)
                || !same_at(&snap, me, x as i64 + bx - px, y as i64 + by - py)
            {
                continue;
            }
            let n4 = neighbor_colors(&snap, mask, x, y, false);
            let alt: Vec<Rgba> = n4.iter().copied().filter(|&c| c != me).collect();
            if let Some((c, n)) = dominant_color(&alt) {
                if n >= 3 {
                    body.set(x, y, c);
                    changed += 1;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_mask(w: u32, h: u32) -> Mask {
        let mut m = Mask::new(w, h);
        for y in 0..h {
            for x in 0..w {
                m.set(x, y, true);
            }
        }
        m
    }

    fn fill(b: &mut Bitmap, c: Rgba) {
        for y in 0..b.height {
            for x in 0..b.width {
                b.set(x, y, c);
            }
        }
    }

    const RED: Rgba = [200, 40, 40, 255];
    const BLUE: Rgba = [40, 40, 200, 255];

    #[test]
    fn orphan_pixel_is_absorbed() {
        let mut b = Bitmap::new(5, 5);
        fill(&mut b, RED);
        b.set(2, 2, BLUE);
        let m = full_mask(5, 5);
        let n = merge_orphans(&mut b, &m, None);
        assert_eq!(n, 1);
        assert_eq!(b.get(2, 2), RED);
    }

    #[test]
    fn two_pixel_cluster_survives_orphan_merge() {
        let mut b = Bitmap::new(5, 5);
        fill(&mut b, RED);
        b.set(2, 2, BLUE);
        b.set(3, 2, BLUE);
        let m = full_mask(5, 5);
        let n = merge_orphans(&mut b, &m, None);
        assert_eq!(n, 0);
        assert_eq!(b.get(2, 2), BLUE);
    }

    #[test]
    fn skip_mask_protects_pixels() {
        let mut b = Bitmap::new(5, 5);
        fill(&mut b, RED);
        b.set(2, 2, BLUE);
        let m = full_mask(5, 5);
        let mut skip = Mask::new(5, 5);
        skip.set(2, 2, true);
        let n = merge_orphans(&mut b, &m, Some(&skip));
        assert_eq!(n, 0);
        assert_eq!(b.get(2, 2), BLUE);
    }

    #[test]
    fn jaggy_protrusion_is_smoothed() {
        // Vertical boundary at x=2 with one blue pixel protruding into red.
        // Give the protrusion a diagonal same-color contact (jaggy, not
        // orphan) so only the jaggy rule can fix it.
        let mut b = Bitmap::new(6, 5);
        fill(&mut b, RED);
        for y in 0..5 {
            for x in 0..2 {
                b.set(x, y, BLUE);
            }
        }
        b.set(2, 2, BLUE); // protrusion: 4-neighbours = blue(1) red(3)
        let m = full_mask(6, 5);
        let orphans = merge_orphans(&mut b, &m, None);
        assert_eq!(orphans, 0, "protrusion touches its region, not an orphan");
        let n = jaggy_cleanup(&mut b, &m, None);
        assert_eq!(n, 1);
        assert_eq!(b.get(2, 2), RED);
    }

    #[test]
    fn line_end_survives_jaggy_cleanup() {
        // A 1px vertical antenna sticking out of a block: deliberate detail,
        // must not be eaten (no flat edge behind the tip).
        let mut b = Bitmap::new(5, 6);
        fill(&mut b, RED);
        for y in 3..6 {
            for x in 0..5 {
                b.set(x, y, BLUE);
            }
        }
        b.set(2, 1, BLUE);
        b.set(2, 2, BLUE); // antenna up from the blue block
        let m = full_mask(5, 6);
        let n = jaggy_cleanup(&mut b, &m, None);
        assert_eq!(n, 0);
        assert_eq!(b.get(2, 1), BLUE);
        assert_eq!(b.get(2, 2), BLUE);
    }

    #[test]
    fn refine_palette_improves_bad_seed() {
        // Reference: 60% bright red, 40% bright blue. Seed palette is far off.
        let mut reference = Bitmap::new(10, 1);
        let mut body = Bitmap::new(10, 1);
        let m = full_mask(10, 1);
        for x in 0..10 {
            let c = if x < 6 {
                [250, 30, 30, 255]
            } else {
                [30, 30, 250, 255]
            };
            reference.set(x, 0, c);
            body.set(x, 0, c);
        }
        let mut palette = vec![[120, 0, 0], [0, 0, 120]];
        refine_palette(&mut body, &reference, &m, None, &mut palette, 4);
        assert_eq!(palette.len(), 2);
        // Refined entries should move toward the bright cluster means.
        assert!(palette.iter().any(|c| c[0] > 200), "{palette:?}");
        assert!(palette.iter().any(|c| c[2] > 200), "{palette:?}");
        // Body must only contain palette colors.
        for x in 0..10 {
            let p = body.get(x, 0);
            assert!(palette.contains(&[p[0], p[1], p[2]]));
        }
    }

    #[test]
    fn run_optimize_is_deterministic() {
        let mut a = Bitmap::new(8, 8);
        let m = full_mask(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                let c = if (x + y) % 3 == 0 { RED } else { BLUE };
                a.set(x, y, c);
            }
        }
        let reference = a.clone();
        let mut b = a.clone();
        let cfg = pixel_formats::OptimizeConfigToml {
            palette_iterations: 2,
            merge_orphans: true,
            jaggy_cleanup: true,
        };
        let mut pal_a = vec![[200, 40, 40], [40, 40, 200]];
        let mut pal_b = pal_a.clone();
        let sa = run_optimize(&mut a, &reference, &m, None, &mut pal_a, &cfg);
        let sb = run_optimize(&mut b, &reference, &m, None, &mut pal_b, &cfg);
        assert_eq!(a, b);
        assert_eq!(sa, sb);
        assert_eq!(pal_a, pal_b);
    }
}
