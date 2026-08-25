//! Core in-memory image + mask types and RGBA image I/O.

use crate::error::CoreError;
use std::path::Path;

/// Default maximum input pixels (PRD §15.1: 64 MP).
pub const DEFAULT_MAX_PIXELS: u64 = 64 * 1024 * 1024;

pub type Rgba = [u8; 4];

/// A dense RGBA8 bitmap, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major RGBA.
    pub data: Vec<u8>,
}

impl Bitmap {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0u8; (width as usize) * (height as usize) * 4],
        }
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> Rgba {
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, px: Rgba) {
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        self.data[i..i + 4].copy_from_slice(&px);
    }

    /// Decode an image file into RGBA8, enforcing the pixel limit *before*
    /// full allocation where possible (PRD §7.1 FR-INSPECT-005, §15.5).
    pub fn load(path: &Path, max_pixels: u64) -> Result<Self, CoreError> {
        let reader = image::ImageReader::open(path)
            .map_err(|e| CoreError::Io(e.to_string()))?
            .with_guessed_format()
            .map_err(|e| CoreError::Decode(e.to_string()))?;
        if let Ok((w, h)) = reader.into_dimensions() {
            if (w as u64) * (h as u64) > max_pixels {
                return Err(CoreError::InputTooLarge(format!(
                    "{w}x{h} exceeds max {max_pixels} pixels"
                )));
            }
        }
        let img = image::ImageReader::open(path)
            .map_err(|e| CoreError::Io(e.to_string()))?
            .with_guessed_format()
            .map_err(|e| CoreError::Decode(e.to_string()))?
            .decode()
            .map_err(|e| CoreError::Decode(e.to_string()))?;
        let rgba = img.to_rgba8();
        Ok(Self {
            width: rgba.width(),
            height: rgba.height(),
            data: rgba.into_raw(),
        })
    }

    /// Encode this bitmap as PNG to `path`.
    pub fn save_png(&self, path: &Path) -> Result<(), CoreError> {
        let buf = image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
            .ok_or_else(|| CoreError::Encode("buffer size mismatch".into()))?;
        buf.save_with_format(path, image::ImageFormat::Png)
            .map_err(|e| CoreError::Encode(e.to_string()))
    }

    /// Encode this bitmap as PNG bytes (for atomic writes).
    pub fn to_png_bytes(&self) -> Result<Vec<u8>, CoreError> {
        let buf = image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
            .ok_or_else(|| CoreError::Encode("buffer size mismatch".into()))?;
        let mut out = std::io::Cursor::new(Vec::new());
        buf.write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| CoreError::Encode(e.to_string()))?;
        Ok(out.into_inner())
    }

    /// Nearest-neighbour integer upscale for previews (PRD §7.10).
    pub fn upscale_nearest(&self, factor: u32) -> Bitmap {
        let factor = factor.max(1);
        let mut out = Bitmap::new(self.width * factor, self.height * factor);
        for y in 0..self.height {
            for x in 0..self.width {
                let px = self.get(x, y);
                for dy in 0..factor {
                    for dx in 0..factor {
                        out.set(x * factor + dx, y * factor + dy, px);
                    }
                }
            }
        }
        out
    }
}

/// A binary mask, row-major, `true` = set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub bits: Vec<bool>,
}

impl Mask {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bits: vec![false; (width as usize) * (height as usize)],
        }
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> bool {
        self.bits[(y as usize) * (self.width as usize) + (x as usize)]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, v: bool) {
        self.bits[(y as usize) * (self.width as usize) + (x as usize)] = v;
    }

    pub fn count(&self) -> u32 {
        self.bits.iter().filter(|b| **b).count() as u32
    }
}
