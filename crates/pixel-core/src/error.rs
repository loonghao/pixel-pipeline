//! Error type for the deterministic core.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("encode error: {0}")]
    Encode(String),
    #[error("input too large: {0}")]
    InputTooLarge(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error(transparent)]
    Format(#[from] pixel_formats::FormatError),
}
