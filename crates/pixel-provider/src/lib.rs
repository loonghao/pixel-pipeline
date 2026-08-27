//! `pixel-provider`: pluggable provider traits (PRD §13.3).
//!
//! Providers supply *candidates* only; they can never bypass deterministic QA
//! (DEC-006). These traits define the stable interface for the future
//! semantic-reconstruction (M3) and animation (M4) milestones. P0 ships the
//! interface and metadata contract without a bundled model.

use serde::{Deserialize, Serialize};

/// Metadata every provider result must carry for provenance (PRD §13.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderProvenance {
    pub provider_id: String,
    pub model_version: String,
    pub seed: u64,
    pub request_sha256: String,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("provider failed: {0}")]
    Failed(String),
}

/// Request to segment a source image into a foreground mask.
#[derive(Debug, Clone)]
pub struct SegmentRequest {
    pub input_sha256: String,
    pub width: u32,
    pub height: u32,
}

// Feature types live in pixel-formats (the shared contract layer) so both the
// deterministic core (grid/palette) and providers can use them without a
// circular dependency. Re-export for convenience.
pub use pixel_formats::{FeatureKind, FeatureMap, FeatureRegion};

/// Request to analyze identity-critical features of a source image.
#[derive(Debug, Clone)]
pub struct FeatureRequest {
    pub input_sha256: String,
    pub width: u32,
    pub height: u32,
}

/// Semantic provider that assists reconstruction of complex inputs (M3).
pub trait SemanticProvider {
    /// Return a foreground mask (row-major, 1 = foreground) with provenance.
    fn segment(
        &self,
        request: SegmentRequest,
    ) -> Result<(Vec<u8>, ProviderProvenance), ProviderError>;

    /// Detect identity-critical feature regions (face/eyes/sunglasses/...).
    /// Default: no features (providers that only segment can skip this).
    fn analyze_features(
        &self,
        _request: FeatureRequest,
    ) -> Result<(FeatureMap, ProviderProvenance), ProviderError> {
        Err(ProviderError::Unavailable(
            "feature analysis not supported by this provider".into(),
        ))
    }
}

/// Request to generate animation candidate frames (M4).
#[derive(Debug, Clone)]
pub struct AnimationRequest {
    pub base_sprite_sha256: String,
    pub action: String,
    pub frames: u32,
}

/// Animation provider that generates candidate frames (M4).
pub trait AnimationProvider {
    fn generate(
        &self,
        request: AnimationRequest,
    ) -> Result<Vec<(Vec<u8>, ProviderProvenance)>, ProviderError>;
}

/// Request to generate pixel-art sprite candidates from a prompt and/or a
/// reference image (e.g. Retro Diffusion-class native low-res models).
#[derive(Debug, Clone)]
pub struct GenerationRequest {
    /// Text prompt describing the desired sprite (may be empty when a
    /// reference image drives the generation).
    pub prompt: String,
    /// SHA-256 of an optional reference image already resolved by the caller.
    pub reference_sha256: Option<String>,
    /// Target sprite dimensions; providers must generate at (or natively for)
    /// this resolution, not upscale-then-filter.
    pub width: u32,
    pub height: u32,
    /// Deterministic seed forwarded to the model.
    pub seed: u64,
    /// Number of candidates to return (the compiler + QA gates pick winners).
    pub candidates: u32,
}

/// A single generated candidate: RGBA8 pixels (row-major, `width * height * 4`
/// bytes at the requested dimensions) plus provenance.
#[derive(Debug, Clone)]
pub struct GenerationCandidate {
    pub rgba: Vec<u8>,
    pub provenance: ProviderProvenance,
}

/// Generative provider that produces sprite candidates (PRD §13.3, DEC-006).
///
/// Candidates are *inputs* to the deterministic pipeline: every candidate is
/// still compiled (grid/palette/outline) and must pass the QA gates before it
/// can ship. Providers never bypass QA.
pub trait GenerationProvider {
    fn generate(
        &self,
        request: GenerationRequest,
    ) -> Result<Vec<GenerationCandidate>, ProviderError>;
}
