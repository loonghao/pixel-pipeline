//! Real PNG/CLI regression: a pose occupying the remainder column must survive.
use image::{Rgba, RgbaImage};
use std::process::Command;

#[test]
fn remainder_pose_survives_conversion_and_invalid_grid_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("sheet.png");
    let output = dir.path().join("pose.png");
    let mut sheet = RgbaImage::new(9, 8);
    for y in 2..6 {
        sheet.put_pixel(8, y, Rgba([220, 50, 40, 255]));
    }
    sheet.save(&input).unwrap();
    let run = |grid: &str, target: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_pixelpipe"))
            .arg("convert")
            .arg(&input)
            .arg("-o")
            .arg(target)
            .args([
                "--grid",
                grid,
                "--profile",
                "character-32",
                "--emit-palette",
            ])
            .output()
            .unwrap()
    };
    let result = run("1x2", &output);
    assert!(
        matches!(result.status.code(), Some(0 | 2 | 3)),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(report["output"]
        .as_str()
        .unwrap()
        .ends_with("pose_r0c1.png"));
    let png = image::open(dir.path().join("pose_r0c1.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(png.dimensions(), (32, 32));
    assert!(png.pixels().any(|p| p[3] == 255));
    assert!(png.pixels().all(|p| p[3] == 0 || p[3] == 255));
    let bytes = std::fs::read(dir.path().join("pose_r0c1.png")).unwrap();
    assert_eq!(result.stdout, run("1x2", &output).stdout);
    assert_eq!(
        bytes,
        std::fs::read(dir.path().join("pose_r0c1.png")).unwrap()
    );

    let invalid_dir = dir.path().join("invalid");
    let invalid = run("1x4294967295", &invalid_dir.join("pose.png"));
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("cannot exceed sheet dimensions"));
    assert!(!invalid_dir.exists());
}
