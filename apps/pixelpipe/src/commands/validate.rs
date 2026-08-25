//! `pixelpipe validate` (PRD §7.8, §11.2).
//!
//! Statically checks an existing final sprite (plus an optional body mask)
//! against a profile's hard rules and outline expectations. Writes no
//! artifacts; emits a report and exits with the status code.

use crate::util::{build_report, resolve_profile};
use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use pixel_core::bitmap::{Bitmap, Mask, DEFAULT_MAX_PIXELS};
use pixel_core::palette::distinct_colors;
use pixel_formats::color::parse_hex_color;
use pixel_formats::{Canvas, QaMetrics};
use std::path::PathBuf;

#[derive(ClapArgs)]
pub struct Args {
    /// Final sprite PNG to validate.
    pub input: PathBuf,
    /// Profile file path or built-in name.
    #[arg(short, long, default_value = "character-48")]
    pub profile: String,
    /// Optional body-mask PNG (white = body); else derived from the sprite.
    #[arg(long)]
    pub body_mask: Option<PathBuf>,
    /// Maximum decoded input pixels (safety limit).
    #[arg(long, default_value_t = DEFAULT_MAX_PIXELS)]
    pub max_pixels: u64,
    /// Pretty-print the report on stdout.
    #[arg(long)]
    pub pretty: bool,
}

/// Build a body mask from a loaded mask bitmap (any opaque, non-black pixel).
fn mask_from_bitmap(bmp: &Bitmap) -> Mask {
    let mut m = Mask::new(bmp.width, bmp.height);
    for y in 0..bmp.height {
        for x in 0..bmp.width {
            let p = bmp.get(x, y);
            m.set(x, y, p[3] == 255 && (p[0] > 0 || p[1] > 0 || p[2] > 0));
        }
    }
    m
}

/// Derive a body mask from a final sprite: opaque pixels not the outline color.
fn body_from_sprite(sprite: &Bitmap, outline_color: [u8; 4]) -> Mask {
    let mut m = Mask::new(sprite.width, sprite.height);
    for y in 0..sprite.height {
        for x in 0..sprite.width {
            let p = sprite.get(x, y);
            m.set(x, y, p[3] == 255 && p != outline_color);
        }
    }
    m
}

/// CLI entry point for `validate`.
pub fn run(args: Args) -> Result<i32> {
    let (profile, _) = resolve_profile(&args.profile)?;
    let profile_sha256 = crate::util::sha256_str(&profile.to_toml()?);

    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let input_sha256 = pixel_cache::sha256_hex(&bytes);
    let sprite = Bitmap::load(&args.input, args.max_pixels)?;
    let outline_color = parse_hex_color(&profile.outline.color).map_err(anyhow::Error::msg)?;

    let body_mask = match &args.body_mask {
        Some(path) => {
            let mb = Bitmap::load(path, args.max_pixels)?;
            mask_from_bitmap(&mb)
        }
        None => body_from_sprite(&sprite, outline_color),
    };

    let (extra, missing, mismatch) = pixel_qa::compare_outline(&sprite, &body_mask, &profile);

    let dimension_valid =
        sprite.width == profile.target.width && sprite.height == profile.target.height;
    let alpha_binary = sprite
        .data
        .chunks_exact(4)
        .all(|p| p[3] == 0 || p[3] == 255);
    let palette_colors = distinct_colors(&sprite, &body_mask);
    let body_components = pixel_core::mask::count_components(&body_mask);

    let metrics = QaMetrics {
        dimension_valid,
        alpha_binary,
        body_pixels: body_mask.count(),
        outline_pixels: 0,
        body_components,
        palette_colors,
        palette_limit: profile.palette.max_colors,
        outline_extra_pixels: extra,
        outline_missing_pixels: missing,
        outline_color_mismatch_pixels: mismatch,
        body_pixels_in_reserved_border: 0,
    };

    let qa_input = pixel_qa::QaInput {
        profile: &profile,
        metrics: metrics.clone(),
        mask_source: pixel_core::mask::MaskSource::Alpha,
    };
    let (status, reasons) = pixel_qa::evaluate(&qa_input);

    let report = build_report(
        None,
        &args.input,
        None,
        &profile.name,
        profile_sha256,
        input_sha256,
        Canvas {
            width: sprite.width,
            height: sprite.height,
        },
        "alpha",
        false,
        status,
        metrics,
        reasons,
        Vec::new(),
        Default::default(),
    );

    let json = if args.pretty {
        report.to_json_pretty()
    } else {
        report.to_json()
    };
    println!("{json}");
    Ok(report.status.exit_code())
}
