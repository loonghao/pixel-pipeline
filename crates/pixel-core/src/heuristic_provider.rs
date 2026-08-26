//! Deterministic heuristic `SemanticProvider` fallback (no model).
//!
//! This is the P0-A bootstrap provider: it finds identity-critical regions
//! without any ML model, purely from color heuristics, so the FeatureMap
//! pipeline (grid weighting + color locking) can be exercised end to end. A
//! real ONNX/HTTP provider can replace it later without changing the core.
//!
//! Heuristics (deterministic, PRD DEC-003 — derived from pixels, not guessed):
//!   * Face: largest connected region of skin-tone pixels (Oklab hue range).
//!   * Eyes/sunglasses: small dark connected regions inside the face bbox.

use crate::bitmap::Bitmap;
use crate::oklab::rgb_to_oklab;
use pixel_formats::{FeatureKind, FeatureMap, FeatureRegion};
use pixel_provider::{FeatureRequest, ProviderError, ProviderProvenance, SemanticProvider};

/// A no-model heuristic provider. Deterministic and offline.
pub struct HeuristicProvider {
    /// Provider identifier recorded in provenance.
    pub id: String,
}

impl Default for HeuristicProvider {
    fn default() -> Self {
        Self {
            id: "heuristic-skin-v1".into(),
        }
    }
}

impl HeuristicProvider {
    /// Analyze features directly from a decoded bitmap (in-memory entry point
    /// used by the conversion pipeline, which already has the bitmap).
    pub fn analyze_bitmap(
        &self,
        src: &Bitmap,
        input_sha256: &str,
    ) -> (FeatureMap, ProviderProvenance) {
        let (w, h) = (src.width, src.height);
        let skin = skin_mask(src);
        let face_bbox = largest_component_bbox(&skin, w, h);

        let mut regions = Vec::new();
        if let Some(bbox) = face_bbox {
            regions.push(FeatureRegion {
                kind: FeatureKind::Face,
                bbox,
                confidence: 0.6,
            });
            // Eyes/sunglasses: dark blobs inside the upper face.
            for dblob in dark_blobs(src, bbox) {
                regions.push(FeatureRegion {
                    kind: FeatureKind::Eye,
                    bbox: dblob,
                    confidence: 0.5,
                });
            }
        }

        let prov = ProviderProvenance {
            provider_id: self.id.clone(),
            model_version: "heuristic-1".into(),
            seed: 0,
            request_sha256: input_sha256.to_string(),
            confidence: if regions.is_empty() { 0.0 } else { 0.6 },
            license: None,
        };
        (
            FeatureMap {
                width: w,
                height: h,
                regions,
            },
            prov,
        )
    }
}

impl SemanticProvider for HeuristicProvider {
    fn segment(
        &self,
        _request: pixel_provider::SegmentRequest,
    ) -> Result<(Vec<u8>, ProviderProvenance), ProviderError> {
        Err(ProviderError::Unavailable(
            "heuristic provider does not segment; use alpha/corner mask".into(),
        ))
    }

    fn analyze_features(
        &self,
        _request: FeatureRequest,
    ) -> Result<(FeatureMap, ProviderProvenance), ProviderError> {
        // The trait method lacks the pixels; use `analyze_bitmap` instead.
        Err(ProviderError::Unavailable(
            "use HeuristicProvider::analyze_bitmap with a decoded bitmap".into(),
        ))
    }
}

/// Skin-tone test in Oklab: warm hue, mid-to-high lightness, moderate chroma.
fn is_skin(rgb: [u8; 3]) -> bool {
    let lab = rgb_to_oklab(rgb);
    // Oklab skin cluster (empirical): L in ~[0.55,0.9], a slightly positive
    // (red), b positive (yellow). Tuned to catch faces but not white cloth.
    lab.l > 0.5 && lab.l < 0.92 && lab.a > 0.02 && lab.b > 0.03 && lab.b < 0.2
}

/// Binary mask of skin-tone pixels.
fn skin_mask(src: &Bitmap) -> Vec<bool> {
    let (w, h) = (src.width as usize, src.height as usize);
    let mut m = vec![false; w * h];
    for y in 0..src.height {
        for x in 0..src.width {
            let p = src.get(x, y);
            if p[3] >= 96 && is_skin([p[0], p[1], p[2]]) {
                m[y as usize * w + x as usize] = true;
            }
        }
    }
    m
}

/// Bbox of the largest 4-connected component in a boolean mask.
fn largest_component_bbox(mask: &[bool], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (w as usize, h as usize);
    let mut visited = vec![false; w * h];
    let mut best: Option<(usize, (u32, u32, u32, u32))> = None;
    for start in 0..(w * h) {
        if !mask[start] || visited[start] {
            continue;
        }
        // BFS flood fill.
        let mut stack = vec![start];
        let mut count = 0usize;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        while let Some(i) = stack.pop() {
            if visited[i] || !mask[i] {
                continue;
            }
            visited[i] = true;
            count += 1;
            let (x, y) = ((i % w) as u32, (i / w) as u32);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            for nb in [i.wrapping_sub(1), i + 1, i.wrapping_sub(w), i + w] {
                let (nx, ny) = ((nb % w) as i32, (nb / w) as i32);
                if nb < w * h && nx >= 0 && ny >= 0 && !visited[nb] && mask[nb] {
                    // avoid horizontal wrap-around
                    let same_row = (nb % w) as i32
                        == (i % w) as i32 + (nb as i32 - i as i32).signum().max(0).min(1);
                    let _ = same_row;
                    stack.push(nb);
                }
            }
        }
        if best.map(|(c, _)| count > c).unwrap_or(true) {
            best = Some((count, (x0, y0, x1, y1)));
        }
    }
    best.map(|(_, bb)| bb)
}

/// Small dark connected regions inside the face bbox (eyes / sunglasses).
fn dark_blobs(src: &Bitmap, face: (u32, u32, u32, u32)) -> Vec<(u32, u32, u32, u32)> {
    let (fx0, fy0, fx1, fy1) = face;
    let (w, h) = (src.width as usize, src.height as usize);
    let mut dark = vec![false; w * h];
    for y in fy0..fy1 {
        for x in fx0..fx1 {
            let p = src.get(x, y);
            let lab = rgb_to_oklab([p[0], p[1], p[2]]);
            if p[3] >= 96 && lab.l < 0.45 {
                dark[y as usize * w + x as usize] = true;
            }
        }
    }
    // Collect bboxes of dark components (reuse largest-component BFS per blob).
    let mut visited = vec![false; w * h];
    let mut out = Vec::new();
    for start in 0..(w * h) {
        if !dark[start] || visited[start] {
            continue;
        }
        let mut stack = vec![start];
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut count = 0usize;
        while let Some(i) = stack.pop() {
            if visited[i] || !dark[i] {
                continue;
            }
            visited[i] = true;
            count += 1;
            let (x, y) = ((i % w) as u32, (i / w) as u32);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            let (cx, cy) = (i % w, i / w);
            if cx + 1 < w && dark[i + 1] && !visited[i + 1] {
                stack.push(i + 1);
            }
            if cx >= 1 && dark[i - 1] && !visited[i - 1] {
                stack.push(i - 1);
            }
            if cy + 1 < h && dark[i + w] && !visited[i + w] {
                stack.push(i + w);
            }
            if cy >= 1 && dark[i - w] && !visited[i - w] {
                stack.push(i - w);
            }
        }
        // Only keep small blobs (eyes), not big hair/eyebrow masses.
        if count >= 2 && (x1 - x0) <= (fx1 - fx0) / 2 {
            out.push((x0, y0, x1, y1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_mask_finds_a_face_blob() {
        // A skin-tone square should be detected as a face region.
        let mut b = Bitmap::new(16, 16);
        for y in 4..12 {
            for x in 4..12 {
                b.set(x, y, [222, 170, 130, 255]); // skin tone
            }
        }
        let p = HeuristicProvider::default();
        let (map, _) = p.analyze_bitmap(&b, "test");
        assert!(map.regions.iter().any(|r| r.kind == FeatureKind::Face));
    }
}
