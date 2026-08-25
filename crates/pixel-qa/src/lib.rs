//! `pixel-qa`: deterministic static QA and the release gate (PRD §7.8, §14.7).
//!
//! QA turns raw metrics into a `pass/review/fail` status plus stable reason
//! codes. It never lets a `review` be silently upgraded to `pass` (PRD §7.11).

use pixel_core::bitmap::{Bitmap, Mask};
use pixel_core::mask::MaskSource;
use pixel_core::outline::compile_outline;
use pixel_formats::color::parse_hex_color;
use pixel_formats::{Profile, QaMetrics, Reason, ReasonCode, Status};

/// Inputs to static QA gathered from a conversion or an existing sprite.
pub struct QaInput<'a> {
    pub profile: &'a Profile,
    pub metrics: QaMetrics,
    pub mask_source: MaskSource,
}

/// Evaluate the release gate over precomputed metrics (PRD §14.7).
pub fn evaluate(input: &QaInput) -> (Status, Vec<Reason>) {
    let m = &input.metrics;
    let mut reasons = Vec::new();
    let mut status = Status::Pass;

    if !m.dimension_valid {
        reasons.push(Reason::new(ReasonCode::DimensionMismatch, Status::Fail));
        status = status.merge(Status::Fail);
    }
    if !m.alpha_binary {
        reasons.push(Reason::new(ReasonCode::AlphaNotBinary, Status::Fail));
        status = status.merge(Status::Fail);
    }
    if m.body_pixels == 0 {
        reasons.push(Reason::new(ReasonCode::BodyEmpty, Status::Fail));
        status = status.merge(Status::Fail);
    }
    if m.body_pixels_in_reserved_border > 0 {
        reasons.push(Reason::new(ReasonCode::BodyInReservedBorder, Status::Fail));
        status = status.merge(Status::Fail);
    }
    if m.palette_colors > m.palette_limit {
        reasons.push(Reason::new(ReasonCode::PaletteLimitExceeded, Status::Fail));
        status = status.merge(Status::Fail);
    }
    if m.outline_extra_pixels > 0 {
        reasons.push(Reason::new(ReasonCode::OutlineExtraPixels, Status::Fail));
        status = status.merge(Status::Fail);
    }
    if m.outline_missing_pixels > 0 {
        reasons.push(Reason::new(ReasonCode::OutlineMissingPixels, Status::Fail));
        status = status.merge(Status::Fail);
    }
    if m.outline_color_mismatch_pixels > 0 {
        reasons.push(Reason::new(ReasonCode::OutlineColorMismatch, Status::Fail));
        status = status.merge(Status::Fail);
    }

    // Corner-background inference can never auto-pass (PRD FR-MASK-002, §7.11).
    if matches!(input.mask_source, MaskSource::CornerBackground) && status == Status::Pass {
        reasons.push(Reason::with_detail(
            ReasonCode::SemanticConfidenceLow,
            Status::Review,
            "foreground inferred from flat background",
        ));
        status = status.merge(Status::Review);
    }

    (status, reasons)
}

/// Compare an existing final sprite against the outline expected from a body
/// mask (PRD §7.8). Returns (extra, missing, color_mismatch) pixel counts.
pub fn compare_outline(
    final_sprite: &Bitmap,
    body_mask: &Mask,
    profile: &Profile,
) -> (u32, u32, u32) {
    let expected = compile_outline(body_mask, profile);
    let outline_color = parse_hex_color(&profile.outline.color).unwrap_or([0, 0, 0, 255]);
    let mut extra = 0;
    let mut missing = 0;
    let mut mismatch = 0;
    for y in 0..final_sprite.height {
        for x in 0..final_sprite.width {
            let opaque = final_sprite.get(x, y)[3] == 255;
            let actual_outline = opaque && !body_mask.get(x, y);
            let exp = expected.get(x, y);
            if actual_outline && !exp {
                extra += 1;
            }
            if exp && !actual_outline {
                missing += 1;
            }
            if actual_outline && exp && final_sprite.get(x, y) != outline_color {
                mismatch += 1;
            }
        }
    }
    (extra, missing, mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_metrics() -> QaMetrics {
        QaMetrics {
            dimension_valid: true,
            alpha_binary: true,
            body_pixels: 100,
            outline_pixels: 40,
            body_components: 1,
            palette_colors: 8,
            palette_limit: 16,
            outline_extra_pixels: 0,
            outline_missing_pixels: 0,
            outline_color_mismatch_pixels: 0,
            body_pixels_in_reserved_border: 0,
        }
    }

    #[test]
    fn clean_metrics_pass() {
        let input = QaInput {
            profile: &crate::tests::dummy_profile(),
            metrics: base_metrics(),
            mask_source: MaskSource::Alpha,
        };
        let (status, reasons) = evaluate(&input);
        assert_eq!(status, Status::Pass);
        assert!(reasons.is_empty());
    }

    #[test]
    fn extra_outline_fails() {
        let mut m = base_metrics();
        m.outline_extra_pixels = 3;
        let input = QaInput {
            profile: &crate::tests::dummy_profile(),
            metrics: m,
            mask_source: MaskSource::Alpha,
        };
        assert_eq!(evaluate(&input).0, Status::Fail);
    }

    #[test]
    fn corner_bg_forces_review() {
        let input = QaInput {
            profile: &crate::tests::dummy_profile(),
            metrics: base_metrics(),
            mask_source: MaskSource::CornerBackground,
        };
        assert_eq!(evaluate(&input).0, Status::Review);
    }

    pub(crate) fn dummy_profile() -> Profile {
        Profile::from_toml(include_str!("../../../profiles/character-48.toml")).unwrap()
    }
}
