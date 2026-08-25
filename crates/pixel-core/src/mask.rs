//! Foreground/body mask derivation and cleanup (PRD §7.4, §14.2, §14.4).

use crate::bitmap::{Bitmap, Mask};

/// How the foreground mask was derived; recorded in the report (PRD §12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskSource {
    /// Derived from the source alpha channel (preferred, deterministic).
    Alpha,
    /// Estimated from flat-background corner sampling (forces `review`).
    CornerBackground,
}

impl MaskSource {
    pub fn as_str(self) -> &'static str {
        match self {
            MaskSource::Alpha => "alpha",
            MaskSource::CornerBackground => "corner-background",
        }
    }
}

/// Fraction of source pixels that have any partial transparency. Used to
/// decide whether the source has a usable alpha channel.
pub fn alpha_coverage(src: &Bitmap) -> f32 {
    let total = (src.width as usize) * (src.height as usize);
    if total == 0 {
        return 0.0;
    }
    let non_opaque = src.data.chunks_exact(4).filter(|p| p[3] < 255).count();
    non_opaque as f32 / total as f32
}

/// Build a high-resolution foreground mask from source alpha (PRD §14.2).
pub fn foreground_from_alpha(src: &Bitmap, threshold: u8) -> Mask {
    let mut m = Mask::new(src.width, src.height);
    for y in 0..src.height {
        for x in 0..src.width {
            m.set(x, y, src.get(x, y)[3] >= threshold);
        }
    }
    m
}

/// Flat-background fallback: sample four corners, take a median-ish background
/// color and mark pixels far from it as foreground (PRD §14.2). Callers must
/// treat the resulting asset as at least `review`.
pub fn foreground_from_corners(src: &Bitmap, tolerance: u8) -> Mask {
    let corners = [
        src.get(0, 0),
        src.get(src.width - 1, 0),
        src.get(0, src.height - 1),
        src.get(src.width - 1, src.height - 1),
    ];
    let bg = median_color(&corners);
    let tol = (tolerance as i32) * 3;
    let mut m = Mask::new(src.width, src.height);
    for y in 0..src.height {
        for x in 0..src.width {
            let p = src.get(x, y);
            let d = (p[0] as i32 - bg[0] as i32).abs()
                + (p[1] as i32 - bg[1] as i32).abs()
                + (p[2] as i32 - bg[2] as i32).abs();
            m.set(x, y, d > tol);
        }
    }
    m
}

fn median_color(colors: &[[u8; 4]]) -> [u8; 4] {
    let mut out = [0u8; 4];
    for ch in 0..4 {
        let mut vals: Vec<u8> = colors.iter().map(|c| c[ch]).collect();
        vals.sort_unstable();
        out[ch] = vals[vals.len() / 2];
    }
    out
}

/// Remove connected components smaller than `min_pixels` (PRD §14.4).
/// Uses deterministic scan order + 8-connectivity flood fill.
pub fn remove_small_components(mask: &mut Mask, min_pixels: u32) {
    if min_pixels <= 1 {
        return;
    }
    let w = mask.width;
    let h = mask.height;
    let mut visited = vec![false; (w as usize) * (h as usize)];
    for y in 0..h {
        for x in 0..w {
            let idx = (y as usize) * (w as usize) + (x as usize);
            if !mask.get(x, y) || visited[idx] {
                continue;
            }
            let mut stack = vec![(x, y)];
            let mut component = Vec::new();
            visited[idx] = true;
            while let Some((cx, cy)) = stack.pop() {
                component.push((cx, cy));
                for (nx, ny) in neighbors8(cx, cy, w, h) {
                    let nidx = (ny as usize) * (w as usize) + (nx as usize);
                    if mask.get(nx, ny) && !visited[nidx] {
                        visited[nidx] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            if (component.len() as u32) < min_pixels {
                for (px, py) in component {
                    mask.set(px, py, false);
                }
            }
        }
    }
}

/// Count connected components (8-connectivity) in a mask.
pub fn count_components(mask: &Mask) -> u32 {
    let w = mask.width;
    let h = mask.height;
    let mut visited = vec![false; (w as usize) * (h as usize)];
    let mut count = 0;
    for y in 0..h {
        for x in 0..w {
            let idx = (y as usize) * (w as usize) + (x as usize);
            if !mask.get(x, y) || visited[idx] {
                continue;
            }
            count += 1;
            let mut stack = vec![(x, y)];
            visited[idx] = true;
            while let Some((cx, cy)) = stack.pop() {
                for (nx, ny) in neighbors8(cx, cy, w, h) {
                    let nidx = (ny as usize) * (w as usize) + (nx as usize);
                    if mask.get(nx, ny) && !visited[nidx] {
                        visited[nidx] = true;
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }
    count
}

/// 8-connected in-bounds neighbors of a pixel.
pub fn neighbors8(x: u32, y: u32, w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(8);
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx >= 0 && ny >= 0 && (nx as u32) < w && (ny as u32) < h {
                out.push((nx as u32, ny as u32));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_from_alpha_respects_threshold() {
        let mut src = Bitmap::new(2, 1);
        src.set(0, 0, [10, 10, 10, 50]);
        src.set(1, 0, [10, 10, 10, 200]);
        let m = foreground_from_alpha(&src, 96);
        assert!(!m.get(0, 0));
        assert!(m.get(1, 0));
    }

    #[test]
    fn counts_disconnected_components() {
        let mut m = Mask::new(5, 1);
        m.set(0, 0, true);
        m.set(4, 0, true);
        assert_eq!(count_components(&m), 2);
    }

    #[test]
    fn removes_small_components() {
        let mut m = Mask::new(5, 1);
        m.set(0, 0, true); // lone pixel, dropped
        m.set(2, 0, true);
        m.set(3, 0, true); // 2-pixel component, kept
        remove_small_components(&mut m, 2);
        assert!(!m.get(0, 0));
        assert!(m.get(2, 0) && m.get(3, 0));
    }

    #[test]
    fn corner_fallback_marks_far_pixels() {
        let mut src = Bitmap::new(3, 3);
        for y in 0..3 {
            for x in 0..3 {
                src.set(x, y, [255, 255, 255, 255]);
            }
        }
        src.set(1, 1, [0, 0, 0, 255]);
        let m = foreground_from_corners(&src, 24);
        assert!(m.get(1, 1));
        assert!(!m.get(0, 0));
    }
}
