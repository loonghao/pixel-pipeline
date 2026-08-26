//! Target-grid reconstruction (PRD §7.5, §14.3).
//!
//! Each target body cell maps to a rectangular region of the source. We
//! compute foreground coverage and alpha-weighted, linear-space average color.
//! This is the deterministic baseline — not nearest-neighbour shrink (FR-RECON-002).

use crate::bitmap::{Bitmap, Mask};
use crate::oklab::{linear_to_srgb, srgb_to_linear};
use pixel_formats::{Anchor, FeatureMap, Profile};

/// Result of reconstruction: a body bitmap and its binary mask, both sized to
/// the full target canvas (body content placed inside the reserved region).
pub struct Reconstructed {
    pub body: Bitmap,
    pub body_mask: Mask,
    /// Target-canvas mask of body pixels that map onto identity-critical
    /// feature regions (face/eyes/...). Empty when no FeatureMap was supplied.
    /// Used to lock their colors during palette quantization (FR-PALETTE-005).
    pub feature_mask: Mask,
}

/// Reconstruct the source (clipped to `fg` foreground) onto the target grid.
///
/// `features` (optional) marks identity-critical regions; their source pixels
/// get `profile.features.saliency_weight`× sampling weight so key features
/// survive downsampling (PRD §7.5 FR-RECON-004, §14.3).
pub fn reconstruct(
    src: &Bitmap,
    fg: &Mask,
    profile: &Profile,
    features: Option<&FeatureMap>,
) -> Reconstructed {
    let (bw, bh) = profile.body_region();
    let reserved = profile.outline.width + profile.transparent_margin;

    // Bounding box of foreground in source; fall back to full image if empty.
    let bbox = foreground_bbox(fg).unwrap_or((0, 0, src.width, src.height));
    let (sx0, sy0, sx1, sy1) = bbox;
    let src_w = (sx1 - sx0).max(1);
    let src_h = (sy1 - sy0).max(1);

    // Contain fit: keep aspect ratio inside the body region.
    let scale = f64::min(bw as f64 / src_w as f64, bh as f64 / src_h as f64);
    let fit_w = ((src_w as f64 * scale).round() as u32).clamp(1, bw);
    let fit_h = ((src_h as f64 * scale).round() as u32).clamp(1, bh);

    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false);
    let mut cell = Bitmap::new(fit_w, fit_h);
    let mut cell_mask = Mask::new(fit_w, fit_h);
    let mut cell_feature = Mask::new(fit_w, fit_h);
    for ty in 0..fit_h {
        for tx in 0..fit_w {
            let rx0 = sx0 + (tx as u64 * src_w as u64 / fit_w as u64) as u32;
            let rx1 = sx0 + ((tx + 1) as u64 * src_w as u64 / fit_w as u64) as u32;
            let ry0 = sy0 + (ty as u64 * src_h as u64 / fit_h as u64) as u32;
            let ry1 = sy0 + ((ty + 1) as u64 * src_h as u64 / fit_h as u64) as u32;
            let (rx1, ry1) = (rx1.max(rx0 + 1), ry1.max(ry0 + 1));
            let (px, cov) = sample_region(
                src,
                fg,
                rx0,
                rx1,
                ry0,
                ry1,
                features,
                profile.features.saliency_weight,
            );
            if cov >= profile.alpha.coverage_threshold {
                cell.set(tx, ty, px);
                cell_mask.set(tx, ty, true);
                // Mark the cell as a feature cell if any source pixel in its
                // region is identity-critical (face/eye/sunglasses).
                if use_features && region_has_critical(features.unwrap(), rx0, rx1, ry0, ry1) {
                    cell_feature.set(tx, ty, true);
                }
            }
        }
    }

    // Place the fitted cell inside the full canvas according to the anchor.
    let mut body = Bitmap::new(profile.target.width, profile.target.height);
    let mut body_mask = Mask::new(profile.target.width, profile.target.height);
    let mut feature_mask = Mask::new(profile.target.width, profile.target.height);
    let off_x = reserved + (bw - fit_w) / 2;
    let off_y = match profile.anchor {
        Anchor::Center => reserved + (bh - fit_h) / 2,
        Anchor::BottomCenter => reserved + (bh - fit_h),
    };
    for ty in 0..fit_h {
        for tx in 0..fit_w {
            if cell_mask.get(tx, ty) {
                body.set(off_x + tx, off_y + ty, cell.get(tx, ty));
                body_mask.set(off_x + tx, off_y + ty, true);
                if cell_feature.get(tx, ty) {
                    feature_mask.set(off_x + tx, off_y + ty, true);
                }
            }
        }
    }
    Reconstructed {
        body,
        body_mask,
        feature_mask,
    }
}

/// True if any pixel in the source region is identity-critical.
fn region_has_critical(f: &FeatureMap, x0: u32, x1: u32, y0: u32, y1: u32) -> bool {
    for y in y0..y1.min(f.height) {
        for x in x0..x1.min(f.width) {
            if f.is_critical(x, y) {
                return true;
            }
        }
    }
    false
}

/// Alpha-weighted linear-space average of a source region + foreground coverage.
///
/// When `features` is supplied and `saliency_weight > 1`, source pixels inside
/// an identity-critical feature region contribute `saliency_weight`× more to
/// the average, so small key features are not washed out by their surroundings.
fn sample_region(
    src: &Bitmap,
    fg: &Mask,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
) -> ([u8; 4], f32) {
    let mut lr = 0f64;
    let mut lg = 0f64;
    let mut lb = 0f64;
    let mut wsum = 0f64;
    let mut fg_count = 0u32;
    let mut total = 0u32;
    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false) && saliency_weight > 1.0;
    for y in y0..y1 {
        for x in x0..x1 {
            total += 1;
            if !fg.get(x, y) {
                continue;
            }
            fg_count += 1;
            let p = src.get(x, y);
            let a = p[3] as f64 / 255.0;
            // Saliency weight: identity-critical pixels count more.
            let saliency = if use_features {
                let w = features.unwrap().weight_at(x, y) as f64;
                1.0 + ((saliency_weight - 1.0) as f64) * w
            } else {
                1.0
            };
            let weight = a * saliency;
            lr += srgb_to_linear(p[0]) as f64 * weight;
            lg += srgb_to_linear(p[1]) as f64 * weight;
            lb += srgb_to_linear(p[2]) as f64 * weight;
            wsum += weight;
        }
    }
    let coverage = if total == 0 {
        0.0
    } else {
        fg_count as f32 / total as f32
    };
    if wsum <= 0.0 {
        return ([0, 0, 0, 0], coverage);
    }
    let rgb = [
        linear_to_srgb((lr / wsum) as f32),
        linear_to_srgb((lg / wsum) as f32),
        linear_to_srgb((lb / wsum) as f32),
    ];
    ([rgb[0], rgb[1], rgb[2], 255], coverage)
}

/// Tight foreground bounding box `(x0, y0, x1, y1)` (exclusive upper bound).
pub fn foreground_bbox(fg: &Mask) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for y in 0..fg.height {
        for x in 0..fg.width {
            if fg.get(x, y) {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if any {
        Some((x0, y0, x1, y1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::from_toml(include_str!("../../../profiles/character-48.toml")).unwrap()
    }

    #[test]
    fn reconstruct_targets_canvas_and_reserves_border() {
        let p = profile();
        let mut src = Bitmap::new(20, 20);
        let mut fg = Mask::new(20, 20);
        for y in 4..16 {
            for x in 4..16 {
                src.set(x, y, [180, 60, 60, 255]);
                fg.set(x, y, true);
            }
        }
        let recon = reconstruct(&src, &fg, &p, None);
        assert_eq!(recon.body.width, p.target.width);
        assert_eq!(recon.body.height, p.target.height);
        assert!(recon.body_mask.count() > 0);

        // No body pixel may land in the reserved outline+margin border.
        let reserved = p.outline.width + p.transparent_margin;
        let (w, h) = (p.target.width, p.target.height);
        for y in 0..h {
            for x in 0..w {
                let in_border =
                    x < reserved || y < reserved || x >= w - reserved || y >= h - reserved;
                if in_border {
                    assert!(!recon.body_mask.get(x, y));
                }
            }
        }
    }

    #[test]
    fn foreground_bbox_is_tight() {
        let mut fg = Mask::new(10, 10);
        fg.set(3, 4, true);
        fg.set(7, 8, true);
        assert_eq!(foreground_bbox(&fg), Some((3, 4, 8, 9)));
    }
}
