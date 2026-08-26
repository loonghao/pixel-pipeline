//! Three-state quality gate and stable reason codes (PRD §7.11, §12.6).

use serde::{Deserialize, Serialize};

/// Result state of a task. See PRD §7.11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// All hard rules pass and no ambiguous inference was used.
    Pass,
    /// Hard rules pass, but a segmentation/semantic/composition ambiguity exists.
    Review,
    /// One or more game-asset hard rules failed.
    Fail,
}

impl Status {
    /// CLI exit code associated with this status (PRD §7.11).
    pub fn exit_code(self) -> i32 {
        match self {
            Status::Pass => 0,
            Status::Review => 2,
            Status::Fail => 3,
        }
    }

    /// Combine two statuses, keeping the most severe (fail > review > pass).
    pub fn merge(self, other: Status) -> Status {
        use Status::*;
        match (self, other) {
            (Fail, _) | (_, Fail) => Fail,
            (Review, _) | (_, Review) => Review,
            _ => Pass,
        }
    }
}

/// Stable machine-routable reason codes (PRD §12.6).
///
/// Serialized as the exact SCREAMING_SNAKE_CASE strings from the PRD so agents
/// can route on them without natural-language parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReasonCode {
    SourceAlphaMissing,
    ForegroundTouchesBorder,
    BodyEmpty,
    BodyInReservedBorder,
    AlphaNotBinary,
    PaletteLimitExceeded,
    OutlineExtraPixels,
    OutlineMissingPixels,
    OutlineColorMismatch,
    BodyComponentsExceeded,
    SemanticConfidenceLow,
    TemporalTopologyDrift,
    PivotDrift,
    DimensionMismatch,
}

/// A single reason attached to a report, pairing a stable code with the
/// status level it contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    pub code: ReasonCode,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Reason {
    pub fn new(code: ReasonCode, status: Status) -> Self {
        Self {
            code,
            status,
            detail: None,
        }
    }

    pub fn with_detail(code: ReasonCode, status: Status, detail: impl Into<String>) -> Self {
        Self {
            code,
            status,
            detail: Some(detail.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_prd() {
        assert_eq!(Status::Pass.exit_code(), 0);
        assert_eq!(Status::Review.exit_code(), 2);
        assert_eq!(Status::Fail.exit_code(), 3);
    }

    #[test]
    fn merge_keeps_most_severe() {
        assert_eq!(Status::Pass.merge(Status::Review), Status::Review);
        assert_eq!(Status::Review.merge(Status::Fail), Status::Fail);
        assert_eq!(Status::Pass.merge(Status::Pass), Status::Pass);
    }

    #[test]
    fn reason_code_serializes_to_screaming_snake() {
        let json = serde_json::to_string(&ReasonCode::OutlineExtraPixels).unwrap();
        assert_eq!(json, "\"OUTLINE_EXTRA_PIXELS\"");
    }
}
