//! Stable JSON report contract (PRD §12.4).

use crate::status::{Reason, Status};
use serde::{Deserialize, Serialize};

/// Current report schema version.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

/// Static QA metrics block (PRD §12.4, §14.7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QaMetrics {
    pub dimension_valid: bool,
    pub alpha_binary: bool,
    pub body_pixels: u32,
    pub outline_pixels: u32,
    pub body_components: u32,
    pub palette_colors: u32,
    pub palette_limit: u32,
    pub outline_extra_pixels: u32,
    pub outline_missing_pixels: u32,
    pub outline_color_mismatch_pixels: u32,
    pub body_pixels_in_reserved_border: u32,
}

/// Output artifact paths (PRD §7.10).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_png: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_mask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_mask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
}

/// Full conversion / validation report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub tool_version: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub profile: String,
    pub profile_sha256: String,
    pub input_sha256: String,
    pub canvas: Canvas,
    pub mask_source: String,
    pub cached: bool,
    pub qa: QaMetrics,
    #[serde(default)]
    pub reasons: Vec<Reason>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub artifacts: Artifacts,
}

impl Report {
    /// Serialize to compact single-line JSON (for JSONL / batch stdout).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("report is always serializable")
    }

    /// Serialize to pretty JSON (for `--pretty` and sidecar report files).
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("report is always serializable")
    }
}
