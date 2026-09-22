//! Sprite-sheet slicing (PRD §0.2 pipeline entry).
//!
//! Real AI/game inputs are often *sheets* of many poses on a transparent
//! background, not a single subject. Feeding a whole sheet into the target-grid
//! reconstructor squashes every pose into one canvas. This module splits a
//! sheet into individual cells so each can be converted on its own. All
//! functions are deterministic (row-major, top-left first).

use crate::bitmap::Bitmap;
use crate::error::CoreError;

/// How to divide a sheet into cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetSpec {
    /// Explicit grid: `rows` × `cols` equal cells.
    Grid { rows: u32, cols: u32 },
    /// Explicit cell size in pixels; the sheet is tiled by `w`×`h`.
    Cell { w: u32, h: u32 },
}

/// One sliced cell and its row/column position in the sheet.
#[derive(Debug, Clone)]
pub struct Cell {
    pub row: u32,
    pub col: u32,
    pub bitmap: Bitmap,
}

/// A pixel is considered background when fully transparent.
#[inline]
fn is_transparent(px: [u8; 4]) -> bool {
    px[3] == 0
}

/// Slice a sheet in row-major order, skipping fully transparent cells.
/// Grid boundaries partition the entire image, including remainder pixels;
/// fixed-size cells at the right/bottom edge are clipped to the image bounds.
/// Zero dimensions and grids with subpixel cells are rejected before iteration.
pub fn slice(sheet: &Bitmap, spec: SheetSpec) -> Result<Vec<Cell>, CoreError> {
    if sheet.width == 0 || sheet.height == 0 {
        return Err(CoreError::Invalid(
            "sheet dimensions must be positive".into(),
        ));
    }
    let (rows, cols) = match spec {
        SheetSpec::Grid { rows, cols } => {
            if rows == 0 || cols == 0 || rows > sheet.height || cols > sheet.width {
                return Err(CoreError::Invalid(
                    "grid dimensions must be positive and cannot exceed sheet dimensions".into(),
                ));
            }
            (rows, cols)
        }
        SheetSpec::Cell { w, h } => {
            if w == 0 || h == 0 {
                return Err(CoreError::Invalid(
                    "cell dimensions must be positive".into(),
                ));
            }
            (sheet.height.div_ceil(h), sheet.width.div_ceil(w))
        }
    };
    let mut cells = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let (x0, y0, cw, ch) = match spec {
                SheetSpec::Grid { .. } => {
                    // Widen before multiplication: CLI dimensions are u32.
                    let boundary = |index: u32, length: u32, count: u32| {
                        (u64::from(index) * u64::from(length) / u64::from(count)) as u32
                    };
                    let x0 = boundary(col, sheet.width, cols);
                    let y0 = boundary(row, sheet.height, rows);
                    let x1 = boundary(col + 1, sheet.width, cols);
                    let y1 = boundary(row + 1, sheet.height, rows);
                    (x0, y0, x1 - x0, y1 - y0)
                }
                SheetSpec::Cell { w, h } => {
                    let x0 = col * w;
                    let y0 = row * h;
                    (x0, y0, w.min(sheet.width - x0), h.min(sheet.height - y0))
                }
            };
            let bitmap = sheet.crop(x0, y0, cw, ch);
            if !is_cell_empty(&bitmap) {
                cells.push(Cell { row, col, bitmap });
            }
        }
    }
    Ok(cells)
}

fn is_cell_empty(bmp: &Bitmap) -> bool {
    bmp.data.chunks_exact(4).all(|p| p[3] == 0)
}

/// Auto-detect a grid from transparent gutter rows/columns.
///
/// The heuristic finds runs of occupied columns separated by *substantial*
/// transparent gaps (the between-sprite gutters) and likewise for rows, then
/// returns the resulting `(rows, cols)`. To avoid splitting on the small
/// transparent gaps inside an organic silhouette (between an arm and the body,
/// strands of hair, etc.), a gap only separates bands when it is at least
/// `MIN_GUTTER_FRAC` of the axis length.
///
/// Returns `None` when no clear grid is found (a single subject, or a sheet
/// with irregular spacing), in which case the caller should treat the input as
/// a single sprite or pass an explicit `--grid`.
pub fn detect_grid(sheet: &Bitmap) -> Option<(u32, u32)> {
    let cols = count_bands(sheet, Axis::Vertical);
    let rows = count_bands(sheet, Axis::Horizontal);
    match (rows, cols) {
        (Some(r), Some(c)) if r * c >= 2 && r <= MAX_BANDS && c <= MAX_BANDS => Some((r, c)),
        _ => None,
    }
}

/// A gutter must span at least this fraction of the axis to separate sprites.
const MIN_GUTTER_FRAC: f32 = 0.01;
/// Reject implausible detections (organic art with many internal gaps).
const MAX_BANDS: u32 = 16;

#[derive(Clone, Copy)]
enum Axis {
    /// Scan columns (produces the column count).
    Vertical,
    /// Scan rows (produces the row count).
    Horizontal,
}

/// Count contiguous opaque bands along one axis, separated by transparent
/// gutters. A "line" (column or row) is occupied if it has any opaque pixel.
fn count_bands(sheet: &Bitmap, axis: Axis) -> Option<u32> {
    let (n_lines, line_len) = match axis {
        Axis::Vertical => (sheet.width, sheet.height),
        Axis::Horizontal => (sheet.height, sheet.width),
    };
    if n_lines == 0 || line_len == 0 {
        return None;
    }
    let occupied: Vec<bool> = (0..n_lines)
        .map(|l| line_occupied(sheet, axis, l))
        .collect();
    let min_gutter = ((n_lines as f32 * MIN_GUTTER_FRAC).ceil() as u32).max(1);

    // Count occupied bands, treating a transparent run as a separator only when
    // it is at least `min_gutter` long. Short internal gaps stay inside a band.
    let mut bands = 0u32;
    let mut in_band = false;
    let mut gap_run = 0u32;
    for &o in &occupied {
        if o {
            if !in_band && (bands == 0 || gap_run >= min_gutter) {
                bands += 1;
            }
            in_band = true;
            gap_run = 0;
        } else {
            gap_run += 1;
            if gap_run >= min_gutter {
                in_band = false;
            }
        }
    }
    if bands == 0 {
        None
    } else {
        Some(bands)
    }
}

fn line_occupied(sheet: &Bitmap, axis: Axis, line: u32) -> bool {
    match axis {
        Axis::Vertical => (0..sheet.height).any(|y| !is_transparent(sheet.get(line, y))),
        Axis::Horizontal => (0..sheet.width).any(|x| !is_transparent(sheet.get(x, line))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uneven_grid_preserves_every_source_pixel_in_order() {
        let mut sheet = Bitmap::new(7, 5);
        for y in 0..5 {
            for x in 0..7 {
                sheet.set(x, y, [x as u8, y as u8, 20, 255]);
            }
        }
        let cells = slice(&sheet, SheetSpec::Grid { rows: 2, cols: 3 }).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for (index, cell) in cells.iter().enumerate() {
            assert_eq!((cell.row, cell.col), (index as u32 / 3, index as u32 % 3));
            for pixel in cell.bitmap.data.chunks_exact(4) {
                assert!(seen.insert((pixel[0], pixel[1])), "overlapping cells");
            }
        }
        assert_eq!(seen.len(), 35);
        assert!(seen.contains(&(6, 4)));
    }

    #[test]
    fn invalid_grids_are_rejected_and_oversized_cells_are_clipped() {
        let mut sheet = Bitmap::new(7, 5);
        sheet.set(6, 4, [20, 40, 60, 255]);
        for spec in [
            SheetSpec::Grid { rows: 0, cols: 1 },
            SheetSpec::Grid {
                rows: 1,
                cols: u32::MAX,
            },
            SheetSpec::Cell { w: 0, h: 1 },
        ] {
            assert!(slice(&sheet, spec).is_err());
        }
        let cells = slice(
            &sheet,
            SheetSpec::Cell {
                w: u32::MAX,
                h: u32::MAX,
            },
        )
        .unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].bitmap.data, sheet.data);
        let cells = slice(&sheet, SheetSpec::Cell { w: 4, h: 3 }).unwrap();
        assert_eq!((cells[0].row, cells[0].col), (1, 1));
        assert_eq!((cells[0].bitmap.width, cells[0].bitmap.height), (3, 2));
        assert_eq!(cells[0].bitmap.get(2, 1), [20, 40, 60, 255]);
    }

    /// Build a sheet with 2 rows × 3 cols of solid squares separated by
    /// transparent gutters.
    fn grid_sheet() -> Bitmap {
        let mut b = Bitmap::new(38, 24); // 3 cols of 10 + gutters, 2 rows of 10
        let paint = |b: &mut Bitmap, cx: u32, cy: u32| {
            for y in cy..cy + 10 {
                for x in cx..cx + 10 {
                    b.set(x, y, [200, 100, 50, 255]);
                }
            }
        };
        for (r, cy) in [2u32, 14].into_iter().enumerate() {
            for (c, cx) in [2u32, 14, 26].into_iter().enumerate() {
                let _ = (r, c);
                paint(&mut b, cx, cy);
            }
        }
        b
    }

    #[test]
    fn detects_2x3_grid() {
        let sheet = grid_sheet();
        assert_eq!(detect_grid(&sheet), Some((2, 3)));
    }

    #[test]
    fn slice_grid_skips_empty_cells() {
        let sheet = grid_sheet();
        let cells = slice(&sheet, SheetSpec::Grid { rows: 2, cols: 3 }).unwrap();
        assert_eq!(cells.len(), 6);
        assert!(cells.iter().all(|c| c.bitmap.width > 0));
    }

    #[test]
    fn single_subject_has_no_grid() {
        let mut b = Bitmap::new(20, 20);
        for y in 4..16 {
            for x in 4..16 {
                b.set(x, y, [10, 20, 30, 255]);
            }
        }
        assert_eq!(detect_grid(&b), None);
    }

    #[test]
    fn slice_by_cell_size() {
        let sheet = grid_sheet();
        let cells = slice(&sheet, SheetSpec::Cell { w: 19, h: 12 }).unwrap();
        assert!(!cells.is_empty());
    }
}
