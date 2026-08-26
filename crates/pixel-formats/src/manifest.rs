//! Batch JSONL manifest task contract (PRD §7.9, §12.5).

use crate::error::FormatError;
use serde::{Deserialize, Serialize};

/// A single batch task line. Either `profile` (path) or inline overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchTask {
    pub id: String,
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Inline size override like "48x48".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_colors: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl BatchTask {
    /// Parse one JSONL line into a task.
    pub fn from_line(line: &str) -> Result<Self, FormatError> {
        serde_json::from_str(line).map_err(|e| FormatError::Parse(e.to_string()))
    }
}

/// Parse a `WIDTHxHEIGHT` size string.
pub fn parse_size(s: &str) -> Result<(u32, u32), FormatError> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| FormatError::Parse(format!("invalid size '{s}', expected WxH")))?;
    let w: u32 = w
        .trim()
        .parse()
        .map_err(|_| FormatError::Parse(format!("invalid width in '{s}'")))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| FormatError::Parse(format!("invalid height in '{s}'")))?;
    if w == 0 || h == 0 {
        return Err(FormatError::Parse("size must be > 0".into()));
    }
    Ok((w, h))
}

/// Parse a `ROWSxCOLS` grid string for sprite-sheet slicing.
pub fn parse_grid(s: &str) -> Result<(u32, u32), FormatError> {
    let (r, c) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| FormatError::Parse(format!("invalid grid '{s}', expected ROWSxCOLS")))?;
    let rows: u32 = r
        .trim()
        .parse()
        .map_err(|_| FormatError::Parse(format!("invalid rows in '{s}'")))?;
    let cols: u32 = c
        .trim()
        .parse()
        .map_err(|_| FormatError::Parse(format!("invalid cols in '{s}'")))?;
    if rows == 0 || cols == 0 {
        return Err(FormatError::Parse("grid rows/cols must be > 0".into()));
    }
    Ok((rows, cols))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grid() {
        assert_eq!(parse_grid("2x3").unwrap(), (2, 3));
        assert!(parse_grid("2").is_err());
        assert!(parse_grid("0x3").is_err());
    }

    #[test]
    fn parses_size() {
        assert_eq!(parse_size("48x48").unwrap(), (48, 48));
        assert_eq!(parse_size("32X64").unwrap(), (32, 64));
        assert!(parse_size("48").is_err());
        assert!(parse_size("0x10").is_err());
    }

    #[test]
    fn parses_task_line() {
        let t =
            BatchTask::from_line(r#"{"id":"m1","input":"a.png","profile":"p/character-48.toml"}"#)
                .unwrap();
        assert_eq!(t.id, "m1");
        assert_eq!(t.profile.as_deref(), Some("p/character-48.toml"));
    }
}
