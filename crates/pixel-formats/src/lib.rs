//! `pixel-formats`: stable contracts for Pixel Pipeline.
//!
//! Profiles (PRD §7.2), reports (§12.4), batch manifests (§12.5), the
//! `pass/review/fail` status model (§7.11) and reason codes (§12.6).
//!
//! These types are the machine-facing surface used by agents, CI, MCP and
//! the CLI. They are versioned via `*_SCHEMA_VERSION` constants.

pub mod color;
pub mod error;
pub mod manifest;
pub mod profile;
pub mod report;
pub mod status;

pub use error::FormatError;
pub use manifest::{parse_size, BatchTask};
pub use profile::{
    AlphaConfig, Anchor, BackgroundMode, CleanupConfig, ColorSpace, CornerRule, Dithering, Fit,
    OutlineConfig, PaletteConfig, Profile, Target, PROFILE_SCHEMA_VERSION,
};
pub use report::{Artifacts, Canvas, QaMetrics, Report, REPORT_SCHEMA_VERSION};
pub use status::{Reason, ReasonCode, Status};

/// The tool version, sourced from the crate version at build time.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
