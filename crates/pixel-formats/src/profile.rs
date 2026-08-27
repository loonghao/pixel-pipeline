//! Versioned conversion profiles (PRD §7.2, §12.3).

use crate::error::FormatError;
use serde::{Deserialize, Serialize};

/// Current profile schema version.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    #[default]
    Contain,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    Center,
    #[default]
    BottomCenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AlphaConfig {
    /// Alpha value at/above which a source pixel counts as foreground (0-255).
    pub threshold: u8,
    /// Foreground coverage fraction (0-1) for a target cell to be solid.
    pub coverage_threshold: f32,
    #[serde(default)]
    pub background: BackgroundMode,
    #[serde(default = "default_background_tolerance")]
    pub background_tolerance: u8,
}

fn default_background_tolerance() -> u8 {
    24
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundMode {
    #[default]
    Auto,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorSpace {
    Oklab,
    Srgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dithering {
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteConfig {
    pub max_colors: u32,
    #[serde(default = "default_color_space")]
    pub color_space: ColorSpace,
    #[serde(default = "default_dithering")]
    pub dithering: Dithering,
    /// Number of flat lightness bands to snap body colors to before quantizing
    /// (cel-shading / posterization). `0` disables it and preserves the smooth
    /// area-averaged reconstruction. This is a stylization step layered on top
    /// of the deterministic reconstruction (PRD §14.5 note on shading bands).
    #[serde(default)]
    pub posterize_levels: u32,
    /// Build the palette from *source-resolution* pixels and quantize the
    /// source before downsampling (quantize-then-snap order, PRD §14.5).
    /// Small identity-critical regions (eyes, sunglasses) still have thousands
    /// of pixels at source resolution, so they win palette slots that they
    /// would lose when quantizing the already-downsampled grid. Pair with
    /// `sampling.mode = "mode"` so each cell votes among exact palette colors.
    #[serde(default)]
    pub quantize_source: bool,
    /// In sprite-sheet mode, build one palette from *all* cells and share it
    /// across every frame (Aseprite's "New Palette from Sprite": all frames
    /// feed a single histogram). Eliminates frame-to-frame color flicker in
    /// animations. Ignored for single-sprite conversions.
    #[serde(default = "default_sheet_shared")]
    pub sheet_shared: bool,
}

fn default_sheet_shared() -> bool {
    true
}

/// How a target cell samples its source region (PRD §7.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SamplingMode {
    /// Alpha-weighted linear-space average (deterministic baseline).
    #[default]
    Area,
    /// Dominant-cluster centroid per cell (K-Centroid): a small k-means in
    /// Oklab keeps hard edges instead of averaging them away.
    KCentroid,
    /// Weighted majority vote of exact colors per cell. Intended for the
    /// quantize-then-snap order (`palette.quantize_source = true`), where the
    /// source is already palette-quantized: each cell then picks its dominant
    /// palette color, exactly like a pixel artist filling a grid cell.
    Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingConfig {
    #[serde(default)]
    pub mode: SamplingMode,
    /// Number of color clusters per cell for `k-centroid` (2 recommended;
    /// more clusters reintroduce noise).
    #[serde(default = "default_centroids")]
    pub centroids: u32,
}

fn default_centroids() -> u32 {
    2
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            mode: SamplingMode::Area,
            centroids: 2,
        }
    }
}

/// Post-quantization pixel-art convergence pass (Gerstner-style refinement).
/// All steps are deterministic, only reuse existing palette colors and never
/// touch identity-critical feature pixels. Default off (schema-compatible with
/// older profiles); the shipped character profiles enable it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizeConfigToml {
    /// Lloyd iterations alternating palette refinement and pixel re-assignment
    /// after median-cut. `0` disables.
    #[serde(default)]
    pub palette_iterations: u32,
    /// Absorb orphan pixels (no same-color neighbour) into the dominant
    /// neighbouring color.
    #[serde(default)]
    pub merge_orphans: bool,
    /// Remove single-pixel stair artifacts (jaggies) on color boundaries.
    #[serde(default)]
    pub jaggy_cleanup: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailConfigToml {
    /// Local window half-size for contrast-aware detail preservation
    /// (PRD §14.3 saliency/edge weight). `0` disables the pass entirely.
    #[serde(default)]
    pub radius: u32,
    /// Erode/dilate iterations for the expansion.
    #[serde(default = "default_detail_iterations")]
    pub iterations: u32,
}

fn default_detail_iterations() -> u32 {
    1
}

impl Default for DetailConfigToml {
    fn default() -> Self {
        Self {
            radius: 0,
            iterations: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FeatureConfigToml {
    /// Sampling-weight multiplier applied to identity-critical feature regions
    /// (face/eyes/...) during target-grid reconstruction (PRD §14.3 saliency /
    /// Feature Map weight). `1.0` = no extra weight; values > 1 make key
    /// features survive downsampling. Only used when a FeatureMap is supplied.
    #[serde(default = "default_saliency_weight")]
    pub saliency_weight: f32,
    /// Lock the colors of identity-critical feature pixels so palette
    /// quantization never merges them into a neighbour (FR-PALETTE-005).
    #[serde(default)]
    pub lock_feature_colors: bool,
}

fn default_saliency_weight() -> f32 {
    1.0
}

impl Default for FeatureConfigToml {
    fn default() -> Self {
        Self {
            saliency_weight: 1.0,
            lock_feature_colors: false,
        }
    }
}

fn default_color_space() -> ColorSpace {
    ColorSpace::Oklab
}
fn default_dithering() -> Dithering {
    Dithering::None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CornerRule {
    PixelArt,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineConfig {
    pub width: u32,
    pub color: String,
    #[serde(default = "default_connectivity")]
    pub connectivity: u8,
    #[serde(default = "default_corner_rule")]
    pub corner_rule: CornerRule,
    /// Draw deterministic internal outlines between adjacent body regions whose
    /// perceptual color differs by more than `internal_threshold` (PRD §14.6
    /// open question on internal contours). `false` keeps only the external
    /// one-pixel outline. Internal-outline pixels stay inside the body mask, so
    /// they never affect the external-outline QA gate (`actual == expected`).
    #[serde(default)]
    pub internal: bool,
    /// Oklab distance above which an internal boundary becomes an internal
    /// outline line. Only used when `internal = true`.
    #[serde(default = "default_internal_threshold")]
    pub internal_threshold: f32,
}

fn default_connectivity() -> u8 {
    8
}
fn default_corner_rule() -> CornerRule {
    CornerRule::PixelArt
}
fn default_internal_threshold() -> f32 {
    0.10
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupConfig {
    #[serde(default = "default_min_component_pixels")]
    pub min_component_pixels: u32,
    #[serde(default)]
    pub fill_single_pixel_holes: bool,
    /// Maximum allowed body connected components before a result is flagged
    /// (PRD §2.1, DEC-008). `0` disables the check. A single-subject sprite is
    /// usually one component plus a few legitimate detached parts; a value well
    /// above that catches whole sprite sheets squashed into one canvas.
    #[serde(default)]
    pub max_body_components: u32,
}

fn default_min_component_pixels() -> u32 {
    1
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            min_component_pixels: 1,
            fill_single_pixel_holes: false,
            max_body_components: 0,
        }
    }
}

/// A versioned, Git-friendly conversion profile (PRD §7.2, §12.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub fit: Fit,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub transparent_margin: u32,
    pub target: Target,
    pub alpha: AlphaConfig,
    pub palette: PaletteConfig,
    pub outline: OutlineConfig,
    #[serde(default)]
    pub cleanup: CleanupConfig,
    /// How target cells sample the source (PRD §7.5). Default: area average.
    #[serde(default)]
    pub sampling: SamplingConfig,
    /// Optional post-quantization convergence pass. Default off.
    #[serde(default)]
    pub optimize: OptimizeConfigToml,
    /// Optional contrast-aware detail preservation (PRD §14.3). Default off.
    #[serde(default)]
    pub detail: DetailConfigToml,
    /// Optional identity-critical feature weighting (PRD §7.5 FR-RECON-004).
    /// Only used when a Semantic Provider supplies a FeatureMap.
    #[serde(default)]
    pub features: FeatureConfigToml,
}

impl Profile {
    /// Parse a profile from TOML text and validate it.
    pub fn from_toml(text: &str) -> Result<Self, FormatError> {
        let profile: Profile =
            toml::from_str(text).map_err(|e| FormatError::Parse(e.to_string()))?;
        profile.validate()?;
        Ok(profile)
    }

    /// Serialize the profile back to canonical TOML.
    pub fn to_toml(&self) -> Result<String, FormatError> {
        toml::to_string_pretty(self).map_err(|e| FormatError::Serialize(e.to_string()))
    }

    /// Validate structural invariants (PRD FR-PROFILE-003).
    pub fn validate(&self) -> Result<(), FormatError> {
        if self.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(FormatError::Validation(format!(
                "unsupported profile schema_version {} (expected {})",
                self.schema_version, PROFILE_SCHEMA_VERSION
            )));
        }
        if self.target.width == 0 || self.target.height == 0 {
            return Err(FormatError::Validation(
                "target width/height must be > 0".into(),
            ));
        }
        if self.palette.max_colors == 0 {
            return Err(FormatError::Validation(
                "palette.max_colors must be > 0".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.alpha.coverage_threshold) {
            return Err(FormatError::Validation(
                "alpha.coverage_threshold must be within 0..=1".into(),
            ));
        }
        if self.outline.connectivity != 4 && self.outline.connectivity != 8 {
            return Err(FormatError::Validation(
                "outline.connectivity must be 4 or 8".into(),
            ));
        }
        if self.sampling.centroids == 0 {
            return Err(FormatError::Validation(
                "sampling.centroids must be > 0".into(),
            ));
        }
        let reserved = self.outline.width + self.transparent_margin;
        if self.target.width <= 2 * reserved || self.target.height <= 2 * reserved {
            return Err(FormatError::Validation(
                "target too small for outline + transparent margin".into(),
            ));
        }
        crate::color::parse_hex_color(&self.outline.color)
            .map_err(|e| FormatError::Validation(format!("outline.color: {e}")))?;
        Ok(())
    }

    /// Body region available after reserving outline + margin (PRD §7.3).
    pub fn body_region(&self) -> (u32, u32) {
        let reserved = 2 * (self.outline.width + self.transparent_margin);
        (
            self.target.width.saturating_sub(reserved),
            self.target.height.saturating_sub(reserved),
        )
    }
}
