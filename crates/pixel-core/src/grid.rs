//! Target-grid reconstruction (PRD §7.5, §14.3).
//!
//! Each target body cell maps to a rectangular region of the source. We
//! compute foreground coverage and alpha-weighted, linear-space average color.
//! This is the deterministic baseline — not nearest-neighbour shrink (FR-RECON-002).

use crate::bitmap::{Bitmap, Mask};
use crate::oklab::{linear_to_srgb, oklab_to_rgb, rgb_to_oklab, srgb_to_linear, Oklab};
use pixel_formats::{Anchor, FeatureMap, Profile, SamplingMode};

/// Result of reconstruction: a body bitmap and its binary mask, both sized to
/// the full target canvas (body content placed inside the reserved region).
pub struct Reconstructed {
    pub body: Bitmap,
    pub body_mask: Mask,
    /// Target-canvas mask of body pixels that map onto identity-critical
    /// feature regions (face/eyes/...). Empty when no FeatureMap was supplied.
    /// Used to lock their colors during palette quantization (FR-PALETTE-005).
    pub feature_mask: Mask,
}

/// Reconstruct the source (clipped to `fg` foreground) onto the target grid.
///
/// `features` (optional) marks identity-critical regions; their source pixels
/// get `profile.features.saliency_weight`× sampling weight so key features
/// survive downsampling (PRD §7.5 FR-RECON-004, §14.3).
pub fn reconstruct(
    src: &Bitmap,
    fg: &Mask,
    profile: &Profile,
    features: Option<&FeatureMap>,
) -> Reconstructed {
    let (bw, bh) = profile.body_region();
    let reserved = profile.outline.width + profile.transparent_margin;

    // Bounding box of foreground in source; fall back to full image if empty.
    let bbox = foreground_bbox(fg).unwrap_or((0, 0, src.width, src.height));
    let (sx0, sy0, sx1, sy1) = bbox;
    let src_w = (sx1 - sx0).max(1);
    let src_h = (sy1 - sy0).max(1);

    // Contain fit: keep aspect ratio inside the body region.
    let scale = f64::min(bw as f64 / src_w as f64, bh as f64 / src_h as f64);
    let fit_w = ((src_w as f64 * scale).round() as u32).clamp(1, bw);
    let fit_h = ((src_h as f64 * scale).round() as u32).clamp(1, bh);

    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false);
    // Edge-aware sampling needs a source-wide gradient field; compute it once.
    let edge_map = if matches!(profile.sampling.mode, SamplingMode::Edge) {
        Some(edge_magnitude_map(src))
    } else {
        None
    };
    let mut cell = Bitmap::new(fit_w, fit_h);
    let mut cell_mask = Mask::new(fit_w, fit_h);
    let mut cell_feature = Mask::new(fit_w, fit_h);
    for ty in 0..fit_h {
        for tx in 0..fit_w {
            let rx0 = sx0 + (tx as u64 * src_w as u64 / fit_w as u64) as u32;
            let rx1 = sx0 + ((tx + 1) as u64 * src_w as u64 / fit_w as u64) as u32;
            let ry0 = sy0 + (ty as u64 * src_h as u64 / fit_h as u64) as u32;
            let ry1 = sy0 + ((ty + 1) as u64 * src_h as u64 / fit_h as u64) as u32;
            let (rx1, ry1) = (rx1.max(rx0 + 1), ry1.max(ry0 + 1));
            let (px, cov) = match profile.sampling.mode {
                SamplingMode::Area => sample_region(
                    src,
                    fg,
                    rx0,
                    rx1,
                    ry0,
                    ry1,
                    features,
                    profile.features.saliency_weight,
                ),
                SamplingMode::KCentroid => sample_region_kcentroid(
                    src,
                    fg,
                    rx0,
                    rx1,
                    ry0,
                    ry1,
                    features,
                    profile.features.saliency_weight,
                    profile.sampling.centroids,
                ),
                SamplingMode::Mode => sample_region_mode(
                    src,
                    fg,
                    rx0,
                    rx1,
                    ry0,
                    ry1,
                    features,
                    profile.features.saliency_weight,
                ),
                SamplingMode::Edge => sample_region_edge(
                    src,
                    fg,
                    rx0,
                    rx1,
                    ry0,
                    ry1,
                    features,
                    profile.features.saliency_weight,
                    edge_map.as_deref().unwrap(),
                    profile.sampling.edge_sensitivity,
                ),
                SamplingMode::TwoStage => sample_region_two_stage(
                    src,
                    fg,
                    rx0,
                    rx1,
                    ry0,
                    ry1,
                    features,
                    profile.features.saliency_weight,
                    profile.sampling.centroids,
                ),
            };
            if cov >= profile.alpha.coverage_threshold {
                cell.set(tx, ty, px);
                cell_mask.set(tx, ty, true);
                // Mark the cell as a feature cell if any source pixel in its
                // region is identity-critical (face/eye/sunglasses).
                if use_features && region_has_critical(features.unwrap(), rx0, rx1, ry0, ry1) {
                    cell_feature.set(tx, ty, true);
                }
            }
        }
    }

    // Place the fitted cell inside the full canvas according to the anchor.
    let mut body = Bitmap::new(profile.target.width, profile.target.height);
    let mut body_mask = Mask::new(profile.target.width, profile.target.height);
    let mut feature_mask = Mask::new(profile.target.width, profile.target.height);
    let off_x = reserved + (bw - fit_w) / 2;
    let off_y = match profile.anchor {
        Anchor::Center => reserved + (bh - fit_h) / 2,
        Anchor::BottomCenter => reserved + (bh - fit_h),
    };
    for ty in 0..fit_h {
        for tx in 0..fit_w {
            if cell_mask.get(tx, ty) {
                body.set(off_x + tx, off_y + ty, cell.get(tx, ty));
                body_mask.set(off_x + tx, off_y + ty, true);
                if cell_feature.get(tx, ty) {
                    feature_mask.set(off_x + tx, off_y + ty, true);
                }
            }
        }
    }
    Reconstructed {
        body,
        body_mask,
        feature_mask,
    }
}

/// True if any pixel in the source region is identity-critical.
fn region_has_critical(f: &FeatureMap, x0: u32, x1: u32, y0: u32, y1: u32) -> bool {
    for y in y0..y1.min(f.height) {
        for x in x0..x1.min(f.width) {
            if f.is_critical(x, y) {
                return true;
            }
        }
    }
    false
}

/// Alpha-weighted linear-space average of a source region + foreground coverage.
///
/// When `features` is supplied and `saliency_weight > 1`, source pixels inside
/// an identity-critical feature region contribute `saliency_weight`× more to
/// the average, so small key features are not washed out by their surroundings.
#[allow(clippy::too_many_arguments)]
fn sample_region(
    src: &Bitmap,
    fg: &Mask,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
) -> ([u8; 4], f32) {
    let mut lr = 0f64;
    let mut lg = 0f64;
    let mut lb = 0f64;
    let mut wsum = 0f64;
    let mut fg_count = 0u32;
    let mut total = 0u32;
    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false) && saliency_weight > 1.0;
    for y in y0..y1 {
        for x in x0..x1 {
            total += 1;
            if !fg.get(x, y) {
                continue;
            }
            fg_count += 1;
            let p = src.get(x, y);
            let a = p[3] as f64 / 255.0;
            // Saliency weight: identity-critical pixels count more.
            let saliency = if use_features {
                let w = features.unwrap().weight_at(x, y) as f64;
                1.0 + ((saliency_weight - 1.0) as f64) * w
            } else {
                1.0
            };
            let weight = a * saliency;
            lr += srgb_to_linear(p[0]) as f64 * weight;
            lg += srgb_to_linear(p[1]) as f64 * weight;
            lb += srgb_to_linear(p[2]) as f64 * weight;
            wsum += weight;
        }
    }
    let coverage = if total == 0 {
        0.0
    } else {
        fg_count as f32 / total as f32
    };
    if wsum <= 0.0 {
        return ([0, 0, 0, 0], coverage);
    }
    let rgb = [
        linear_to_srgb((lr / wsum) as f32),
        linear_to_srgb((lg / wsum) as f32),
        linear_to_srgb((lb / wsum) as f32),
    ];
    ([rgb[0], rgb[1], rgb[2], 255], coverage)
}

/// K-Centroid sampling of a source region + foreground coverage (PRD §7.5).
///
/// Instead of averaging the whole region (which blends edge colors into mud),
/// run a tiny deterministic k-means in Oklab over the region's foreground
/// pixels and return the centroid of the *dominant* cluster. This keeps hard
/// edges and dominant local colors at low resolution (Astropulse K-Centroid).
///
/// Determinism: centroids initialize from lightness quantiles of a stably
/// sorted pixel list, iterations are fixed, and all ties break toward the
/// lower cluster index.
#[allow(clippy::too_many_arguments)]
fn sample_region_kcentroid(
    src: &Bitmap,
    fg: &Mask,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
    centroids: u32,
) -> ([u8; 4], f32) {
    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false) && saliency_weight > 1.0;
    let mut pixels: Vec<(Oklab, f32)> = Vec::new();
    let mut fg_count = 0u32;
    let mut total = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            total += 1;
            if !fg.get(x, y) {
                continue;
            }
            fg_count += 1;
            let p = src.get(x, y);
            let a = p[3] as f32 / 255.0;
            let saliency = if use_features {
                let w = features.unwrap().weight_at(x, y);
                1.0 + (saliency_weight - 1.0) * w
            } else {
                1.0
            };
            let weight = a * saliency;
            if weight > 0.0 {
                pixels.push((rgb_to_oklab([p[0], p[1], p[2]]), weight));
            }
        }
    }
    let coverage = if total == 0 {
        0.0
    } else {
        fg_count as f32 / total as f32
    };
    if pixels.is_empty() {
        return ([0, 0, 0, 0], coverage);
    }

    let k = (centroids.max(1) as usize).min(pixels.len());
    let (centers, assign) = kmeans_clusters(&pixels, k);

    // Dominant cluster by total weight; ties toward the lower index.
    let mut weights = vec![0f64; k];
    for (i, (_, w)) in pixels.iter().enumerate() {
        weights[assign[i]] += *w as f64;
    }
    let mut dominant = 0usize;
    for (c, w) in weights.iter().enumerate() {
        if *w > weights[dominant] {
            dominant = c;
        }
    }
    let rgb = oklab_to_rgb(centers[dominant]);
    ([rgb[0], rgb[1], rgb[2], 255], coverage)
}

/// Weighted majority vote of exact colors in a source region (PRD §14.5).
///
/// Intended for the quantize-then-snap order (`palette.quantize_source`):
/// the source is already palette-quantized, so each cell holds only a handful
/// of distinct colors and the vote picks the dominant one — like a pixel
/// artist filling a grid cell. Weights are alpha × saliency; ties break toward
/// the lowest RGB value (deterministic via BTreeMap iteration order).
#[allow(clippy::too_many_arguments)]
fn sample_region_mode(
    src: &Bitmap,
    fg: &Mask,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
) -> ([u8; 4], f32) {
    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false) && saliency_weight > 1.0;
    let mut counts: std::collections::BTreeMap<[u8; 3], f64> = std::collections::BTreeMap::new();
    let mut fg_count = 0u32;
    let mut total = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            total += 1;
            if !fg.get(x, y) {
                continue;
            }
            fg_count += 1;
            let p = src.get(x, y);
            let a = p[3] as f64 / 255.0;
            let saliency = if use_features {
                let w = features.unwrap().weight_at(x, y) as f64;
                1.0 + ((saliency_weight - 1.0) as f64) * w
            } else {
                1.0
            };
            let weight = a * saliency;
            if weight > 0.0 {
                *counts.entry([p[0], p[1], p[2]]).or_insert(0.0) += weight;
            }
        }
    }
    let coverage = if total == 0 {
        0.0
    } else {
        fg_count as f32 / total as f32
    };
    let mut best: Option<([u8; 3], f64)> = None;
    for (color, w) in &counts {
        // Strictly greater keeps the first (lowest-RGB) color on ties.
        if best.map(|(_, bw)| *w > bw).unwrap_or(true) {
            best = Some((*color, *w));
        }
    }
    match best {
        Some((c, _)) => ([c[0], c[1], c[2], 255], coverage),
        None => ([0, 0, 0, 0], coverage),
    }
}

/// Deterministic weighted k-means over Oklab colors (shared by `k-centroid`
/// and `edge` sampling). Centers initialize from lightness quantiles of a
/// stably sorted pixel list, iterations are fixed at 8, and assignment ties
/// break toward the lower cluster index. Returns the final centers and the
/// per-pixel cluster assignment (parallel to `pixels`).
fn kmeans_clusters(pixels: &[(Oklab, f32)], k: usize) -> (Vec<Oklab>, Vec<usize>) {
    // Stable order by (L, a, b) so quantile initialization is deterministic.
    let mut order: Vec<usize> = (0..pixels.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (pixels[i].0, pixels[j].0);
        (a.l, a.a, a.b)
            .partial_cmp(&(b.l, b.a, b.b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut centers: Vec<Oklab> = (0..k)
        .map(|j| {
            let idx = if k == 1 {
                order.len() / 2
            } else {
                j * (order.len() - 1) / (k - 1)
            };
            pixels[order[idx]].0
        })
        .collect();

    let mut assign = vec![0usize; pixels.len()];
    for _ in 0..8 {
        // Assignment: nearest center, ties toward the lower index.
        for (i, (lab, _)) in pixels.iter().enumerate() {
            let mut best = 0usize;
            let mut best_d = f32::MAX;
            for (c, center) in centers.iter().enumerate() {
                let d = crate::oklab::oklab_distance_sq(*lab, *center);
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            assign[i] = best;
        }
        // Update: weighted mean per cluster; empty clusters keep their center.
        let mut sums = vec![(0f64, 0f64, 0f64, 0f64); k];
        for (i, (lab, w)) in pixels.iter().enumerate() {
            let s = &mut sums[assign[i]];
            s.0 += lab.l as f64 * *w as f64;
            s.1 += lab.a as f64 * *w as f64;
            s.2 += lab.b as f64 * *w as f64;
            s.3 += *w as f64;
        }
        for (c, s) in sums.iter().enumerate() {
            if s.3 > 0.0 {
                centers[c] = Oklab {
                    l: (s.0 / s.3) as f32,
                    a: (s.1 / s.3) as f32,
                    b: (s.2 / s.3) as f32,
                };
            }
        }
    }
    (centers, assign)
}

/// Per-pixel edge magnitude of `src`, normalized to `[0, 1]` (row-major,
/// `width * height`). A 3×3 Sobel operator runs over Oklab lightness (the
/// perceptual luminance axis), with replicate padding at the borders. The map
/// is normalized by its maximum so `edge_sensitivity` is scale-independent.
fn edge_magnitude_map(src: &Bitmap) -> Vec<f32> {
    let (w, h) = (src.width, src.height);
    let n = (w * h) as usize;
    let mut lum = vec![0f32; n];
    for y in 0..h {
        for x in 0..w {
            let p = src.get(x, y);
            lum[(y * w + x) as usize] = rgb_to_oklab([p[0], p[1], p[2]]).l;
        }
    }
    let at = |x: i64, y: i64| -> f32 {
        let xc = x.clamp(0, w as i64 - 1) as u32;
        let yc = y.clamp(0, h as i64 - 1) as u32;
        lum[(yc * w + xc) as usize]
    };
    let mut mag = vec![0f32; n];
    let mut maxm = 0f32;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let gx = at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1)
                - at(x + 1, y - 1)
                - 2.0 * at(x + 1, y)
                - at(x + 1, y + 1);
            let gy = at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1)
                - at(x - 1, y + 1)
                - 2.0 * at(x, y + 1)
                - at(x + 1, y + 1);
            let m = (gx * gx + gy * gy).sqrt();
            mag[(y as u32 * w + x as u32) as usize] = m;
            if m > maxm {
                maxm = m;
            }
        }
    }
    if maxm > 0.0 {
        for m in mag.iter_mut() {
            *m /= maxm;
        }
    }
    mag
}

/// Edge-aware sampling of a source region + foreground coverage (PRD §7.5).
///
/// Splits the cell's foreground pixels into two Oklab clusters (deterministic
/// weighted k-means) like `k-centroid`, then chooses the representative cluster
/// based on the cell's local edge strength. In a flat cell (no edge) it returns
/// the *dominant* cluster, exactly like `k-centroid`. As the cell's edge
/// strength rises, selection is biased smoothly toward the *minority* cluster —
/// the thin, high-contrast detail (an outline, eye, or hair strand) that a flat
/// background would otherwise outvote. This keeps lines and small features from
/// dissolving under downsampling while leaving smooth regions untouched.
///
/// `edge_map` is the source's normalized Sobel magnitude; `edge_sensitivity`
/// controls how quickly a cell's edge flips selection toward the detail cluster
/// (`0` reproduces plain dominant-cluster sampling).
#[allow(clippy::too_many_arguments)]
fn sample_region_edge(
    src: &Bitmap,
    fg: &Mask,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
    edge_map: &[f32],
    edge_sensitivity: f32,
) -> ([u8; 4], f32) {
    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false) && saliency_weight > 1.0;
    let mut pixels: Vec<(Oklab, f32)> = Vec::new();
    let mut cell_edge = 0f32;
    let mut fg_count = 0u32;
    let mut total = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            total += 1;
            if !fg.get(x, y) {
                continue;
            }
            fg_count += 1;
            let p = src.get(x, y);
            let a = p[3] as f32 / 255.0;
            let saliency = if use_features {
                let w = features.unwrap().weight_at(x, y);
                1.0 + (saliency_weight - 1.0) * w
            } else {
                1.0
            };
            let weight = a * saliency;
            if weight > 0.0 {
                pixels.push((rgb_to_oklab([p[0], p[1], p[2]]), weight));
                cell_edge = cell_edge.max(edge_map[(y * src.width + x) as usize]);
            }
        }
    }
    let coverage = if total == 0 {
        0.0
    } else {
        fg_count as f32 / total as f32
    };
    if pixels.is_empty() {
        return ([0, 0, 0, 0], coverage);
    }

    let k = 2usize.min(pixels.len());
    let (centers, assign) = kmeans_clusters(&pixels, k);

    // Per-cluster weight and the cell total.
    let mut wsum = vec![0f64; k];
    let mut total_w = 0f64;
    for (i, (_, w)) in pixels.iter().enumerate() {
        wsum[assign[i]] += *w as f64;
        total_w += *w as f64;
    }

    // Edge influence in [0, 1]: 0 = flat (keep dominant), 1 = strong edge
    // (prefer the minority detail). The selection metric interpolates between
    // "largest cluster" and "smallest cluster" so the flip is smooth and
    // deterministic; ties break toward the lower cluster index.
    let influence = (cell_edge * edge_sensitivity).clamp(0.0, 1.0) as f64;
    let mut best = 0usize;
    let mut best_metric = f64::MIN;
    for (c, &w) in wsum.iter().enumerate() {
        let metric = w + influence * (total_w - 2.0 * w);
        if metric > best_metric {
            best_metric = metric;
            best = c;
        }
    }
    let rgb = oklab_to_rgb(centers[best]);
    ([rgb[0], rgb[1], rgb[2], 255], coverage)
}

/// Two-stage sampling: decouple structure from color (PRD §7.5).
///
/// Stage 1 (structure): cluster the cell's foreground pixels into up to
/// `centroids` Oklab clusters with the shared deterministic weighted k-means,
/// then pick the winning cluster by a *center-weighted* vote — pixels near the
/// cell center count more, so a cell centered on a thin outline keeps that
/// outline even when a flat background covers more area. Stage 2 (color): set
/// the cell color to the alpha-weighted linear-RGB mean of the *original*
/// source pixels in the winning cluster, so the color stays accurate and
/// denoised and rare accents survive instead of collapsing to a cluster
/// center. Fully deterministic; `centroids` reuses the `k-centroid` count.
#[allow(clippy::too_many_arguments)]
fn sample_region_two_stage(
    src: &Bitmap,
    fg: &Mask,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    features: Option<&FeatureMap>,
    saliency_weight: f32,
    centroids: u32,
) -> ([u8; 4], f32) {
    let use_features = features.map(|f| !f.is_empty()).unwrap_or(false) && saliency_weight > 1.0;
    let mut pixels: Vec<(Oklab, f32)> = Vec::new();
    // Parallel per-pixel data: center-vote weight and linear-RGB for coloring.
    let mut center_w: Vec<f64> = Vec::new();
    let mut lin: Vec<(f64, f64, f64)> = Vec::new();
    let mut fg_count = 0u32;
    let mut total = 0u32;

    // Cell center and half-extents for the separable tent (Bartlett) window.
    let cx = (x0 as f64 + (x1 - 1) as f64) * 0.5;
    let cy = (y0 as f64 + (y1 - 1) as f64) * 0.5;
    let hx = ((x1 - x0) as f64 * 0.5).max(1.0);
    let hy = ((y1 - y0) as f64 * 0.5).max(1.0);

    for y in y0..y1 {
        for x in x0..x1 {
            total += 1;
            if !fg.get(x, y) {
                continue;
            }
            fg_count += 1;
            let p = src.get(x, y);
            let a = p[3] as f32 / 255.0;
            let saliency = if use_features {
                let w = features.unwrap().weight_at(x, y);
                1.0 + (saliency_weight - 1.0) * w
            } else {
                1.0
            };
            let weight = a * saliency;
            if weight <= 0.0 {
                continue;
            }
            // Separable tent window in [0.15, 1]: center pixels dominate the
            // structure vote, but edge pixels keep a small say so a single-row
            // or single-column cell still produces a decision.
            let wx = 1.0 - (x as f64 - cx).abs() / hx;
            let wy = 1.0 - (y as f64 - cy).abs() / hy;
            let cw = wx.min(wy).clamp(0.15, 1.0);
            pixels.push((rgb_to_oklab([p[0], p[1], p[2]]), weight));
            center_w.push(cw);
            lin.push((
                srgb_to_linear(p[0]) as f64,
                srgb_to_linear(p[1]) as f64,
                srgb_to_linear(p[2]) as f64,
            ));
        }
    }
    let coverage = if total == 0 {
        0.0
    } else {
        fg_count as f32 / total as f32
    };
    if pixels.is_empty() {
        return ([0, 0, 0, 0], coverage);
    }

    let k = (centroids.max(1) as usize).min(pixels.len());
    let (_centers, assign) = kmeans_clusters(&pixels, k);

    // Stage 1: winning cluster by center-weighted vote (× sampling weight).
    // Ties break toward the lower cluster index.
    let mut vote = vec![0f64; k];
    for (i, (_, w)) in pixels.iter().enumerate() {
        vote[assign[i]] += *w as f64 * center_w[i];
    }
    let mut best = 0usize;
    for (c, &v) in vote.iter().enumerate() {
        if v > vote[best] {
            best = c;
        }
    }

    // Stage 2: color = alpha-weighted linear-RGB mean of the ORIGINAL pixels in
    // the winning cluster (accurate, denoised; rare accents survive).
    let (mut lr, mut lg, mut lb, mut wsum) = (0f64, 0f64, 0f64, 0f64);
    for (i, (_, w)) in pixels.iter().enumerate() {
        if assign[i] != best {
            continue;
        }
        let wt = *w as f64;
        lr += lin[i].0 * wt;
        lg += lin[i].1 * wt;
        lb += lin[i].2 * wt;
        wsum += wt;
    }
    if wsum <= 0.0 {
        return ([0, 0, 0, 0], coverage);
    }
    let rgb = [
        linear_to_srgb((lr / wsum) as f32),
        linear_to_srgb((lg / wsum) as f32),
        linear_to_srgb((lb / wsum) as f32),
    ];
    ([rgb[0], rgb[1], rgb[2], 255], coverage)
}

/// Tight foreground bounding box `(x0, y0, x1, y1)` (exclusive upper bound).
pub fn foreground_bbox(fg: &Mask) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for y in 0..fg.height {
        for x in 0..fg.width {
            if fg.get(x, y) {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if any {
        Some((x0, y0, x1, y1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::from_toml(include_str!("../../../profiles/character-48.toml")).unwrap()
    }

    #[test]
    fn reconstruct_targets_canvas_and_reserves_border() {
        let p = profile();
        let mut src = Bitmap::new(20, 20);
        let mut fg = Mask::new(20, 20);
        for y in 4..16 {
            for x in 4..16 {
                src.set(x, y, [180, 60, 60, 255]);
                fg.set(x, y, true);
            }
        }
        let recon = reconstruct(&src, &fg, &p, None);
        assert_eq!(recon.body.width, p.target.width);
        assert_eq!(recon.body.height, p.target.height);
        assert!(recon.body_mask.count() > 0);

        // No body pixel may land in the reserved outline+margin border.
        let reserved = p.outline.width + p.transparent_margin;
        let (w, h) = (p.target.width, p.target.height);
        for y in 0..h {
            for x in 0..w {
                let in_border =
                    x < reserved || y < reserved || x >= w - reserved || y >= h - reserved;
                if in_border {
                    assert!(!recon.body_mask.get(x, y));
                }
            }
        }
    }

    #[test]
    fn kcentroid_returns_dominant_color_not_average() {
        let mut src = Bitmap::new(10, 1);
        let mut fg = Mask::new(10, 1);
        for x in 0..10 {
            let c = if x < 6 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            src.set(x, 0, c);
            fg.set(x, 0, true);
        }
        let (px, cov) = sample_region_kcentroid(&src, &fg, 0, 10, 0, 1, None, 1.0, 2);
        assert_eq!(cov, 1.0);
        // The dominant cluster is red; area averaging would blend to purple.
        assert!(px[0] > 200 && px[2] < 60, "expected red-ish, got {px:?}");
    }

    #[test]
    fn kcentroid_is_deterministic() {
        let mut src = Bitmap::new(8, 8);
        let mut fg = Mask::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                src.set(x, y, [(x * 30) as u8, (y * 25) as u8, 128, 255]);
                fg.set(x, y, true);
            }
        }
        let a = sample_region_kcentroid(&src, &fg, 0, 8, 0, 8, None, 1.0, 2);
        let b = sample_region_kcentroid(&src, &fg, 0, 8, 0, 8, None, 1.0, 2);
        assert_eq!(a, b);
    }

    #[test]
    fn mode_sampling_picks_exact_majority_color() {
        let mut src = Bitmap::new(10, 1);
        let mut fg = Mask::new(10, 1);
        for x in 0..10 {
            let c = if x < 6 {
                [200, 40, 40, 255]
            } else {
                [40, 40, 200, 255]
            };
            src.set(x, 0, c);
            fg.set(x, 0, true);
        }
        let (px, cov) = sample_region_mode(&src, &fg, 0, 10, 0, 1, None, 1.0);
        assert_eq!(cov, 1.0);
        // Majority vote returns the exact dominant color, never a blend.
        assert_eq!(px, [200, 40, 40, 255]);
    }

    #[test]
    fn mode_sampling_ties_break_toward_lowest_rgb() {
        let mut src = Bitmap::new(2, 1);
        let mut fg = Mask::new(2, 1);
        src.set(0, 0, [10, 10, 10, 255]);
        src.set(1, 0, [200, 200, 200, 255]);
        fg.set(0, 0, true);
        fg.set(1, 0, true);
        let (px, _) = sample_region_mode(&src, &fg, 0, 2, 0, 1, None, 1.0);
        assert_eq!(px, [10, 10, 10, 255]);
    }

    #[test]
    fn edge_sampling_preserves_thin_high_contrast_feature() {
        // A light 4×4 cell crossed by a single dark column (4 of 16 pixels).
        // Area/dominant sampling returns light; edge sampling keeps the dark
        // detail because the cell has a strong edge and dark is the minority.
        let mut src = Bitmap::new(4, 4);
        let mut fg = Mask::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let c = if x == 1 {
                    [20, 20, 20, 255]
                } else {
                    [235, 235, 235, 255]
                };
                src.set(x, y, c);
                fg.set(x, y, true);
            }
        }
        let edges = edge_magnitude_map(&src);
        let (edge_px, _) = sample_region_edge(&src, &fg, 0, 4, 0, 4, None, 1.0, &edges, 3.0);
        let (dom_px, _) = sample_region_kcentroid(&src, &fg, 0, 4, 0, 4, None, 1.0, 2);
        assert!(
            edge_px[0] < 60,
            "edge keeps the dark detail, got {edge_px:?}"
        );
        assert!(
            dom_px[0] > 180,
            "dominant keeps the light majority, got {dom_px:?}"
        );
    }

    #[test]
    fn edge_sensitivity_zero_matches_dominant_cluster() {
        // With sensitivity 0 the edge influence vanishes, so the mode reduces
        // to plain dominant-cluster (k-centroid) selection: light wins.
        let mut src = Bitmap::new(4, 4);
        let mut fg = Mask::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let c = if x == 1 {
                    [20, 20, 20, 255]
                } else {
                    [235, 235, 235, 255]
                };
                src.set(x, y, c);
                fg.set(x, y, true);
            }
        }
        let edges = edge_magnitude_map(&src);
        let (px, _) = sample_region_edge(&src, &fg, 0, 4, 0, 4, None, 1.0, &edges, 0.0);
        assert!(px[0] > 180, "sensitivity 0 keeps the majority, got {px:?}");
    }

    #[test]
    fn edge_sampling_is_deterministic() {
        let mut src = Bitmap::new(8, 8);
        let mut fg = Mask::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                src.set(x, y, [(x * 30) as u8, (y * 25) as u8, 128, 255]);
                fg.set(x, y, true);
            }
        }
        let edges = edge_magnitude_map(&src);
        let a = sample_region_edge(&src, &fg, 0, 8, 0, 8, None, 1.0, &edges, 3.0);
        let b = sample_region_edge(&src, &fg, 0, 8, 0, 8, None, 1.0, &edges, 3.0);
        assert_eq!(a, b);
    }

    #[test]
    fn two_stage_is_crisp_and_accurate() {
        // 6 red / 4 blue in one row. Area averaging blends to purple; two-stage
        // votes the centered-majority red cluster and colors the cell from the
        // mean of the ORIGINAL red pixels, so it stays exactly red.
        let mut src = Bitmap::new(10, 1);
        let mut fg = Mask::new(10, 1);
        for x in 0..10 {
            let c = if x < 6 {
                [200, 40, 40, 255]
            } else {
                [40, 40, 200, 255]
            };
            src.set(x, 0, c);
            fg.set(x, 0, true);
        }
        let (two, cov) = sample_region_two_stage(&src, &fg, 0, 10, 0, 1, None, 1.0, 2);
        let (area, _) = sample_region(&src, &fg, 0, 10, 0, 1, None, 1.0);
        assert_eq!(cov, 1.0);
        // Crisp + accurate: the winning red cluster's mean is the exact red.
        assert!(
            (two[0] as i32 - 200).abs() <= 3 && two[2] <= 45,
            "expected crisp red ~[200,40,40], got {two:?}"
        );
        // Area sampling bleeds blue into the result; two-stage does not.
        assert!(
            area[2] > two[2] + 30,
            "area should blend blue in, area={area:?} two={two:?}"
        );
    }

    #[test]
    fn two_stage_center_weight_favors_centered_detail() {
        // A centered dark pair (2 px) framed by a lighter pair on each side
        // (4 px). Plain dominant-cluster (k-centroid) keeps the larger light
        // area; two-stage's center-weighted vote keeps the centered dark
        // detail — the mechanism that lands boundaries and outlines crisp.
        let mut src = Bitmap::new(6, 1);
        let mut fg = Mask::new(6, 1);
        for x in 0..6 {
            let c = if x == 2 || x == 3 {
                [20, 20, 20, 255]
            } else {
                [235, 235, 235, 255]
            };
            src.set(x, 0, c);
            fg.set(x, 0, true);
        }
        let (two, _) = sample_region_two_stage(&src, &fg, 0, 6, 0, 1, None, 1.0, 2);
        let (dom, _) = sample_region_kcentroid(&src, &fg, 0, 6, 0, 1, None, 1.0, 2);
        assert!(two[0] < 60, "center-weighted vote keeps dark, got {two:?}");
        assert!(
            dom[0] > 180,
            "dominant keeps the light majority, got {dom:?}"
        );
    }

    #[test]
    fn two_stage_is_deterministic() {
        let mut src = Bitmap::new(8, 8);
        let mut fg = Mask::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                src.set(x, y, [(x * 30) as u8, (y * 25) as u8, 128, 255]);
                fg.set(x, y, true);
            }
        }
        let a = sample_region_two_stage(&src, &fg, 0, 8, 0, 8, None, 1.0, 2);
        let b = sample_region_two_stage(&src, &fg, 0, 8, 0, 8, None, 1.0, 2);
        assert_eq!(a, b);
    }

    #[test]
    fn foreground_bbox_is_tight() {
        let mut fg = Mask::new(10, 10);
        fg.set(3, 4, true);
        fg.set(7, 8, true);
        assert_eq!(foreground_bbox(&fg), Some((3, 4, 8, 9)));
    }
}
