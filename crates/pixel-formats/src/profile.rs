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
}

fn default_connectivity() -> u8 {
    8
}
fn default_corner_rule() -> CornerRule {
    CornerRule::PixelArt
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupConfig {
    #[serde(default = "default_min_component_pixels")]
    pub min_component_pixels: u32,
    #[serde(default)]
    pub fill_single_pixel_holes: bool,
}

fn default_min_component_pixels() -> u32 {
    1
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            min_component_pixels: 1,
            fill_single_pixel_holes: false,
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
