//! Sprite-sheet slicing (PRD §0.2 pipeline entry).
//!
//! Real AI/game inputs are often *sheets* of many poses on a transparent
//! background, not a single subject. Feeding a whole sheet into the target-grid
//! reconstructor squashes every pose into one canvas. This module splits a
//! sheet into individual cells so each can be converted on its own. All
//! functions are deterministic (row-major, top-left first).

use crate::bitmap::Bitmap;

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

/// Slice a sheet into equal cells according to `spec`. Cells are returned in
/// row-major order. Empty (fully transparent) cells are skipped so an agent
/// only gets real sprites.
pub fn slice(sheet: &Bitmap, spec: SheetSpec) -> Vec<Cell> {
    let (rows, cols) = match spec {
        SheetSpec::Grid { rows, cols } => (rows.max(1), cols.max(1)),
        SheetSpec::Cell { w, h } => {
            let cols = if w == 0 { 1 } else { sheet.width.div_ceil(w) };
            let rows = if h == 0 { 1 } else { sheet.height.div_ceil(h) };
            (rows.max(1), cols.max(1))
        }
    };
    let mut cells = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let (x0, y0, cw, ch) = match spec {
                SheetSpec::Grid { .. } => {
                    let cw = sheet.width / cols;
                    let ch = sheet.height / rows;
                    (col * cw, row * ch, cw, ch)
                }
                SheetSpec::Cell { w, h } => (col * w, row * h, w, h),
            };
            if cw == 0 || ch == 0 {
                continue;
            }
            let bitmap = sheet.crop(x0, y0, cw, ch);
            if !is_cell_empty(&bitmap) {
                cells.push(Cell { row, col, bitmap });
            }
        }
    }
    cells
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
        let cells = slice(&sheet, SheetSpec::Grid { rows: 2, cols: 3 });
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
        let cells = slice(&sheet, SheetSpec::Cell { w: 19, h: 12 });
        assert!(!cells.is_empty());
    }
}
