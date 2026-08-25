//! Error types for the formats/contract layer.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("serialize error: {0}")]
    Serialize(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("io error: {0}")]
    Io(String),
}
