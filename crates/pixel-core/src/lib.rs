//! `pixel-core`: the deterministic true-pixel compiler (PRD §7, §14).
//!
//! Pipeline: load → foreground/body mask → target-grid reconstruction →
//! palette quantization → one-pixel outline compilation → composition →
//! artifacts. All steps are deterministic and reproducible (PRD §15.2).

pub mod bitmap;
pub mod compose;
pub mod convert;
pub mod detail;
pub mod error;
pub mod grid;
pub mod heuristic_provider;
pub mod inspect;
pub mod internal_outline;
pub mod mask;
pub mod oklab;
pub mod outline;
pub mod palette;
pub mod sheet;

pub use bitmap::{Bitmap, Mask, Rgba};
pub use convert::{convert, convert_bitmap, ConvertOptions, ConvertOutput};
pub use error::CoreError;
pub use inspect::{inspect, InspectResult};
pub use sheet::{detect_grid, slice, Cell, SheetSpec};

/// Re-export the tool version from formats for a single source of truth.
pub use pixel_formats::TOOL_VERSION;
