//! Deterministic internal-outline compiler (PRD §14.6 open question).
//!
//! The external outline (`outline.rs`) rings the silhouette. Hand-drawn pixel
//! art also uses dark 1px lines *inside* the silhouette to separate features
//! (hair strands, arm vs. torso, facial features). We derive those lines
//! deterministically from perceptual color boundaries between adjacent body
//! pixels — never from the source's baked-in dark RGB (PRD DEC-003/004).
//!
//! Because every internal-outline pixel stays inside the body mask, it never
//! becomes an `actual_outline` pixel (`opaque && !body_mask`) and therefore
//! cannot break the external-outline QA gate.

use crate::bitmap::{Bitmap, Mask};
use crate::oklab::{oklab_distance_sq, rgb_to_oklab, Oklab};

/// Compile the internal-outline mask: pixels that sit on the darker side of a
/// perceptual color boundary between two adjacent body regions.
///
/// We scan only the right and down neighbours so each boundary is considered
/// once, and mark the darker (lower Oklab lightness) of the two pixels. This
/// yields a single, deterministic 1px line per region boundary rather than a
/// double line on both sides.
pub fn compile_internal_outline(body: &Bitmap, body_mask: &Mask, threshold: f32) -> Mask {
    let mut edges = Mask::new(body_mask.width, body_mask.height);
    if threshold <= 0.0 {
        return edges;
    }
    let thr_sq = threshold * threshold;
    let lab_at = |x: u32, y: u32| -> Oklab {
        let p = body.get(x, y);
        rgb_to_oklab([p[0], p[1], p[2]])
    };
    for y in 0..body_mask.height {
        for x in 0..body_mask.width {
            if !body_mask.get(x, y) {
                continue;
            }
            let here = lab_at(x, y);
            for (nx, ny) in neighbors_right_down(x, y, body_mask.width, body_mask.height) {
                if !body_mask.get(nx, ny) {
                    continue;
                }
                let there = lab_at(nx, ny);
                if oklab_distance_sq(here, there) > thr_sq {
                    // Mark the darker side (ties go to the current pixel for
                    // deterministic, scan-order-independent results).
                    if here.l <= there.l {
                        edges.set(x, y, true);
                    } else {
                        edges.set(nx, ny, true);
                    }
                }
            }
        }
    }
    edges
}

/// Right and down in-bounds neighbours (each undirected edge visited once).
fn neighbors_right_down(x: u32, y: u32, w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(2);
    if x + 1 < w {
        out.push((x + 1, y));
    }
    if y + 1 < h {
        out.push((x, y + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_edges_when_region_is_uniform() {
        let mut body = Bitmap::new(4, 4);
        let mut mask = Mask::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                body.set(x, y, [120, 90, 60, 255]);
                mask.set(x, y, true);
            }
        }
        assert_eq!(compile_internal_outline(&body, &mask, 0.1).count(), 0);
    }

    #[test]
    fn marks_darker_side_of_a_boundary() {
        // Left half light, right half dark; boundary between x=1 and x=2.
        let mut body = Bitmap::new(4, 1);
        let mut mask = Mask::new(4, 1);
        for x in 0..4u32 {
            let c = if x < 2 { [230, 230, 230] } else { [20, 20, 20] };
            body.set(x, 0, [c[0], c[1], c[2], 255]);
            mask.set(x, 0, true);
        }
        let edges = compile_internal_outline(&body, &mask, 0.1);
        // The dark side (x=2) is marked, not the light side (x=1).
        assert!(edges.get(2, 0));
        assert!(!edges.get(1, 0));
        assert_eq!(edges.count(), 1);
    }

    #[test]
    fn edges_stay_inside_body_mask() {
        let mut body = Bitmap::new(3, 1);
        let mut mask = Mask::new(3, 1);
        // Only x=0 and x=2 are body; x=1 is a gap.
        body.set(0, 0, [230, 230, 230, 255]);
        body.set(2, 0, [20, 20, 20, 255]);
        mask.set(0, 0, true);
        mask.set(2, 0, true);
        let edges = compile_internal_outline(&body, &mask, 0.1);
        // No body-to-body adjacency across the gap => no internal edges.
        assert_eq!(edges.count(), 0);
    }
}
