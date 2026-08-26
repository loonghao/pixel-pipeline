//! Oklab color space + sRGB<->linear helpers (PRD §14.1, §14.5, FR-PALETTE-002).
//!
//! The conversion matrices use the canonical Oklab constants (Ottosson); we
//! keep their full precision even though f32 rounds them.
#![allow(clippy::excessive_precision)]

/// Convert an 8-bit sRGB channel to linear [0,1].
#[inline]
pub fn srgb_to_linear(c: u8) -> f32 {
    let x = c as f32 / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert a linear [0,1] channel back to 8-bit sRGB.
#[inline]
pub fn linear_to_srgb(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let s = if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// An Oklab color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

/// Convert linear-sRGB (r,g,b in [0,1]) to Oklab.
pub fn linear_srgb_to_oklab(r: f32, g: f32, b: f32) -> Oklab {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    Oklab {
        l: 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        a: 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        b: 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    }
}

/// Convert an 8-bit sRGB pixel to Oklab.
pub fn rgb_to_oklab(rgb: [u8; 3]) -> Oklab {
    linear_srgb_to_oklab(
        srgb_to_linear(rgb[0]),
        srgb_to_linear(rgb[1]),
        srgb_to_linear(rgb[2]),
    )
}

/// Convert Oklab back to linear-sRGB (r,g,b), clamped to [0,1].
pub fn oklab_to_linear_srgb(c: Oklab) -> (f32, f32, f32) {
    let l_ = c.l + 0.3963377774 * c.a + 0.2158037573 * c.b;
    let m_ = c.l - 0.1055613458 * c.a - 0.0638541728 * c.b;
    let s_ = c.l - 0.0894841775 * c.a - 1.2914855480 * c.b;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
    (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
}

/// Convert an Oklab color back to an 8-bit sRGB pixel.
pub fn oklab_to_rgb(c: Oklab) -> [u8; 3] {
    let (r, g, b) = oklab_to_linear_srgb(c);
    [linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b)]
}

/// Squared Euclidean distance in Oklab space.
#[inline]
pub fn oklab_distance_sq(a: Oklab, b: Oklab) -> f32 {
    let dl = a.l - b.l;
    let da = a.a - b.a;
    let db = a.b - b.b;
    dl * dl + da * da + db * db
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_linear_roundtrip_endpoints() {
        assert_eq!(linear_to_srgb(srgb_to_linear(0)), 0);
        assert_eq!(linear_to_srgb(srgb_to_linear(255)), 255);
    }

    #[test]
    fn identical_colors_zero_distance() {
        let a = rgb_to_oklab([120, 30, 200]);
        assert!(oklab_distance_sq(a, a) < 1e-9);
    }

    #[test]
    fn black_and_white_are_far() {
        let black = rgb_to_oklab([0, 0, 0]);
        let white = rgb_to_oklab([255, 255, 255]);
        assert!(oklab_distance_sq(black, white) > 0.5);
    }

    #[test]
    fn oklab_rgb_roundtrip_is_near_identity() {
        for c in [[0, 0, 0], [255, 255, 255], [120, 30, 200], [17, 200, 90]] {
            let back = oklab_to_rgb(rgb_to_oklab(c));
            for ch in 0..3 {
                assert!(
                    (back[ch] as i32 - c[ch] as i32).abs() <= 1,
                    "channel {ch}: {} vs {}",
                    back[ch],
                    c[ch]
                );
            }
        }
    }
}
