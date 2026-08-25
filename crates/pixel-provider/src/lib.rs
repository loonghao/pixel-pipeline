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

/// Semantic provider that assists reconstruction of complex inputs (M3).
pub trait SemanticProvider {
    /// Return a foreground mask (row-major, 1 = foreground) with provenance.
    fn segment(
        &self,
        request: SegmentRequest,
    ) -> Result<(Vec<u8>, ProviderProvenance), ProviderError>;
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
