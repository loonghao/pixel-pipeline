//! Deterministic one-pixel outline compiler (PRD §7.7, §14.6).
//!
//! `expected_outline = dilate(body_mask, width, connectivity) - body_mask`,
//! with an optional pixel-art corner rule to drop pure-diagonal nubs.

use crate::bitmap::Mask;
use pixel_formats::{CornerRule, Profile};

/// Compile the expected outline mask from a body mask (PRD §7.7).
pub fn compile_outline(body: &Mask, profile: &Profile) -> Mask {
    let width = profile.outline.width.max(1);
    let conn = profile.outline.connectivity;
    let dilated = dilate(body, width, conn);
    let mut outline = Mask::new(body.width, body.height);
    for y in 0..body.height {
        for x in 0..body.width {
            outline.set(x, y, dilated.get(x, y) && !body.get(x, y));
        }
    }
    if matches!(profile.outline.corner_rule, CornerRule::PixelArt) && conn == 8 {
        apply_pixel_art_corner_rule(&mut outline, body);
    }
    outline
}

/// Dilate a mask by `width` steps using 4- or 8-connectivity.
pub fn dilate(mask: &Mask, width: u32, connectivity: u8) -> Mask {
    let mut cur = mask.clone();
    for _ in 0..width {
        let mut next = cur.clone();
        for y in 0..cur.height {
            for x in 0..cur.width {
                if cur.get(x, y) {
                    continue;
                }
                if has_set_neighbor(&cur, x, y, connectivity) {
                    next.set(x, y, true);
                }
            }
        }
        cur = next;
    }
    cur
}

fn has_set_neighbor(mask: &Mask, x: u32, y: u32, connectivity: u8) -> bool {
    let offsets: &[(i32, i32)] = if connectivity == 4 {
        &[(0, -1), (0, 1), (-1, 0), (1, 0)]
    } else {
        &[
            (0, -1),
            (0, 1),
            (-1, 0),
            (1, 0),
            (-1, -1),
            (1, -1),
            (-1, 1),
            (1, 1),
        ]
    };
    for (dx, dy) in offsets {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx >= 0
            && ny >= 0
            && (nx as u32) < mask.width
            && (ny as u32) < mask.height
            && mask.get(nx as u32, ny as u32)
        {
            return true;
        }
    }
    false
}

/// Drop outline pixels that touch the body only via a single diagonal and have
/// no orthogonal body neighbor (the "square-corner nub" of 8-dilation, §14.6).
fn apply_pixel_art_corner_rule(outline: &mut Mask, body: &Mask) {
    let w = outline.width;
    let h = outline.height;
    let to_clear: Vec<(u32, u32)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            outline.get(x, y)
                && !orthogonal_body(body, x, y)
                && diagonal_body_count(body, x, y) == 1
        })
        .collect();
    for (x, y) in to_clear {
        outline.set(x, y, false);
    }
}

fn orthogonal_body(body: &Mask, x: u32, y: u32) -> bool {
    for (dx, dy) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx >= 0
            && ny >= 0
            && (nx as u32) < body.width
            && (ny as u32) < body.height
            && body.get(nx as u32, ny as u32)
        {
            return true;
        }
    }
    false
}

fn diagonal_body_count(body: &Mask, x: u32, y: u32) -> u32 {
    let mut n = 0;
    for (dx, dy) in [(-1i32, -1i32), (1, -1), (-1, 1), (1, 1)] {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx >= 0
            && ny >= 0
            && (nx as u32) < body.width
            && (ny as u32) < body.height
            && body.get(nx as u32, ny as u32)
        {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixel_formats::{
        AlphaConfig, BackgroundMode, CleanupConfig, ColorSpace, DetailConfigToml, Dithering,
        FeatureConfigToml, Fit, OptimizeConfigToml, OutlineConfig, PaletteConfig, SamplingConfig,
        Target, PROFILE_SCHEMA_VERSION,
    };

    fn profile(corner_rule: CornerRule, connectivity: u8) -> Profile {
        Profile {
            schema_version: PROFILE_SCHEMA_VERSION,
            name: "t".into(),
            fit: Fit::Contain,
            anchor: pixel_formats::Anchor::BottomCenter,
            transparent_margin: 0,
            target: Target {
                width: 8,
                height: 8,
            },
            alpha: AlphaConfig {
                threshold: 96,
                coverage_threshold: 0.5,
                background: BackgroundMode::Auto,
                background_tolerance: 24,
            },
            palette: PaletteConfig {
                max_colors: 8,
                color_space: ColorSpace::Oklab,
                dithering: Dithering::None,
                dither_strength: 0.06,
                posterize_levels: 0,
                quantize_source: false,
                sheet_shared: true,
            },
            outline: OutlineConfig {
                width: 1,
                color: "#000000".into(),
                connectivity,
                corner_rule,
                internal: false,
                internal_threshold: 0.10,
            },
            cleanup: CleanupConfig::default(),
            sampling: SamplingConfig::default(),
            optimize: OptimizeConfigToml::default(),
            detail: DetailConfigToml::default(),
            features: FeatureConfigToml::default(),
        }
    }

    #[test]
    fn single_pixel_pixel_art_rule_drops_diagonal_nubs() {
        let mut body = Mask::new(5, 5);
        body.set(2, 2, true);
        let outline = compile_outline(&body, &profile(CornerRule::PixelArt, 8));
        // Only the 4 orthogonal neighbours survive the corner rule.
        assert_eq!(outline.count(), 4);
        assert!(outline.get(2, 1) && outline.get(1, 2) && outline.get(3, 2) && outline.get(2, 3));
        assert!(!outline.get(1, 1) && !outline.get(3, 3));
    }

    #[test]
    fn single_pixel_no_corner_rule_keeps_full_ring() {
        let mut body = Mask::new(5, 5);
        body.set(2, 2, true);
        let outline = compile_outline(&body, &profile(CornerRule::None, 8));
        assert_eq!(outline.count(), 8);
    }

    #[test]
    fn outline_never_overlaps_body() {
        let mut body = Mask::new(6, 6);
        for y in 2..4 {
            for x in 2..4 {
                body.set(x, y, true);
            }
        }
        let outline = compile_outline(&body, &profile(CornerRule::PixelArt, 4));
        for y in 0..6 {
            for x in 0..6 {
                assert!(!(outline.get(x, y) && body.get(x, y)));
            }
        }
    }
}
