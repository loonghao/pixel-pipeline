//! Identity-critical feature regions (PRD §7.5 FR-RECON-004, §14.3).
//!
//! A `FeatureMap` marks regions of the *source* image that a pixel artist
//! would protect by hand — face, eyes, sunglasses — so the deterministic
//! reconstructor can weight them higher and the palette stage can lock their
//! colors. These types live in the contract layer so both `pixel-core` and
//! `pixel-provider` use them without a circular dependency.

use serde::{Deserialize, Serialize};

/// A semantic feature category worth preserving at small sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureKind {
    Face,
    Eye,
    Sunglasses,
    Mouth,
    WeaponTip,
    /// Anything not covered above.
    Other,
}

/// One detected feature region, in *source* pixel coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureRegion {
    pub kind: FeatureKind,
    /// Inclusive top-left and exclusive bottom-right: (x0, y0, x1, y1).
    pub bbox: (u32, u32, u32, u32),
    /// Provider confidence in [0, 1].
    pub confidence: f32,
}

/// A map of identity-critical feature regions over the source image.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeatureMap {
    /// Source image dimensions these regions refer to.
    pub width: u32,
    pub height: u32,
    pub regions: Vec<FeatureRegion>,
}

impl FeatureMap {
    /// True if `(x, y)` falls inside any identity-critical region.
    pub fn is_critical(&self, x: u32, y: u32) -> bool {
        self.regions.iter().any(|r| {
            matches!(
                r.kind,
                FeatureKind::Face | FeatureKind::Eye | FeatureKind::Sunglasses
            ) && x >= r.bbox.0
                && y >= r.bbox.1
                && x < r.bbox.2
                && y < r.bbox.3
        })
    }

    /// Highest confidence among regions containing `(x, y)`, else 0.
    pub fn weight_at(&self, x: u32, y: u32) -> f32 {
        self.regions
            .iter()
            .filter(|r| x >= r.bbox.0 && y >= r.bbox.1 && x < r.bbox.2 && y < r.bbox.3)
            .map(|r| r.confidence)
            .fold(0f32, f32::max)
    }

    /// True if no regions were detected.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_covers_face_eye_sunglasses_only() {
        let mut m = FeatureMap {
            width: 10,
            height: 10,
            regions: vec![
                FeatureRegion {
                    kind: FeatureKind::Face,
                    bbox: (1, 1, 4, 4),
                    confidence: 0.9,
                },
                FeatureRegion {
                    kind: FeatureKind::WeaponTip,
                    bbox: (6, 6, 9, 9),
                    confidence: 0.8,
                },
            ],
        };
        assert!(m.is_critical(2, 2));
        assert!(!m.is_critical(7, 7)); // weapon tip is not face/eye/sunglasses
        assert!(!m.is_critical(0, 0));
        m.regions.clear();
        assert!(m.is_empty());
    }

    #[test]
    fn weight_at_returns_max_confidence() {
        let m = FeatureMap {
            width: 10,
            height: 10,
            regions: vec![
                FeatureRegion {
                    kind: FeatureKind::Face,
                    bbox: (0, 0, 5, 5),
                    confidence: 0.5,
                },
                FeatureRegion {
                    kind: FeatureKind::Eye,
                    bbox: (0, 0, 5, 5),
                    confidence: 0.9,
                },
            ],
        };
        assert!((m.weight_at(2, 2) - 0.9).abs() < 1e-6);
        assert_eq!(m.weight_at(9, 9), 0.0);
    }
}
