//! Compose body + outline into final and mask artifacts (PRD §7.10).

use crate::bitmap::{Bitmap, Mask};

/// Render a binary mask to a black/white RGBA bitmap (white = set).
pub fn mask_to_bitmap(mask: &Mask) -> Bitmap {
    let mut b = Bitmap::new(mask.width, mask.height);
    for y in 0..mask.height {
        for x in 0..mask.width {
            if mask.get(x, y) {
                b.set(x, y, [255, 255, 255, 255]);
            }
        }
    }
    b
}

/// Render the outline mask as a solid-color RGBA bitmap (transparent elsewhere).
pub fn outline_to_bitmap(outline: &Mask, color: [u8; 4]) -> Bitmap {
    let mut b = Bitmap::new(outline.width, outline.height);
    for y in 0..outline.height {
        for x in 0..outline.width {
            if outline.get(x, y) {
                b.set(x, y, color);
            }
        }
    }
    b
}

/// Compose body over a fresh canvas, then paint the outline color where the
/// outline mask is set (PRD §7.7 — outline is derived, not from source RGB).
pub fn compose_final(body: &Bitmap, outline: &Mask, outline_color: [u8; 4]) -> Bitmap {
    let mut out = body.clone();
    for y in 0..out.height {
        for x in 0..out.width {
            if outline.get(x, y) {
                out.set(x, y, outline_color);
            }
        }
    }
    out
}
