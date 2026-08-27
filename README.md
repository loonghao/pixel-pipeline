# Pixel Pipeline

[![CI](https://github.com/loonghao/pixel-pipeline/actions/workflows/ci.yml/badge.svg)](https://github.com/loonghao/pixel-pipeline/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Agent-first, deterministic true-pixel asset compiler for game production.**

Pixel Pipeline turns illustrations and flat art into genuine pixel-art sprites:
target-grid reconstruction, Oklab palette quantization, a deterministic
one-pixel outline, and a machine-routable `pass` / `review` / `fail` quality
gate. Every run is reproducible and emits a structured JSON report designed to
be consumed by agents, CI, and tooling — not just humans.

## Features

- **Deterministic pipeline** — same input + profile ⇒ byte-identical output.
- **True pixels** — area-coverage grid reconstruction, not naive downscaling.
- **Compiled outlines** — outer outlines are *derived* from the body mask with
  pixel-art corner rules; optional internal outlines come from Oklab colour
  boundaries, never traced from source RGB.
- **Oklab palettes** — median-cut quantization in Oklab with a hard color
  budget and optional lightness posterization into flat cel-shading bands.
- **Pixel-art convergence** — an optional post-quantization pass (Oklab Lloyd
  palette refinement, orphan-pixel absorption, single-pixel jaggy cleanup).
  Deterministic, and the palette can only shrink — never grow.
- **Detail preservation** — a contrast-aware O(n) sliding-window pass expands
  small high-contrast features (eyes, hair strands) before downsampling so they
  survive at low resolution.
- **Shared sheet palettes & export** — sprite-sheet mode can build one palette
  across every frame (so animations never flicker); `--emit-palette` writes a
  GIMP/Aseprite `.gpl` sidecar.
- **A real quality gate** — hard rules produce stable reason codes; ambiguous
  segmentation can never silently `pass` (it is forced to `review`).
- **Agent-friendly CLI** — JSON on stdout, logs on stderr, status exit codes,
  atomic writes, and JSONL batch with `--resume` / `--jobs`.

## Install

```bash
# From source (Rust stable toolchain)
cargo build --release
# Binary at target/release/pixelpipe
```

Prebuilt binaries for Linux, macOS, and Windows are attached to each
[release](https://github.com/loonghao/pixel-pipeline/releases).

## Usage

```bash
# 1. Inspect an input and get a suggested mode (JSON on stdout)
pixelpipe inspect hero.png --pretty

# 2. Convert to a 48x48 true-pixel sprite with the built-in profile
pixelpipe convert hero.png -o out/hero.png --profile character-48 --pretty

# 2b. Slice a sprite sheet, then convert each cell
pixelpipe convert sheet.png -o out/idle.png --profile character-48 --grid 4x4

# 2c. Preserve identity-critical features (face/eyes) during reconstruction
pixelpipe convert hero.png -o out/hero.png --profile character-48 --detect-features

# 2d. Also emit the final palette as a GIMP/Aseprite .gpl sidecar
pixelpipe convert hero.png -o out/hero.png --profile character-48 --emit-palette

# 3. Validate an existing sprite against a profile's hard rules
pixelpipe validate out/hero.png --profile character-48 \
  --body-mask out/hero.body-mask.png

# 4. Batch-convert a JSONL manifest in parallel, resumable
pixelpipe batch tasks.jsonl --out-dir out --jobs 8 --resume
```

`convert` writes the final PNG plus sidecar artifacts (`*.body.png`,
`*.body-mask.png`, `*.outline-mask.png`, `*.preview.png`) and a
`*.report.json`.

### Batch manifest (JSONL)

One task per line:

```json
{"id":"hero-48","input":"hero.png","profile":"character-48"}
{"id":"slime","input":"slime.png","size":"32x32","max_colors":8}
```

## Profiles

Profiles are versioned TOML files describing the target grid, alpha handling,
palette budget, outline, and cleanup rules. Built-in profiles ship in the
binary; point `--profile` at a file path to use your own.

- `character-32` — 32×32, 10 colors
- `character-48` — 48×48, 12 colors
- `character-64` — 64×64, 16 colors

The shipped character profiles use the *quantize-then-snap* order: the palette
is built at source resolution (where small identity regions still win slots),
each target cell picks its dominant palette color, and the convergence and
detail passes run. Posterize and internal outlines are off for this order
(exact palette boundaries are already crisp); the external outline is
unaffected. See [`profiles/`](profiles/) for the full schema.

## Integrations

- **Aseprite** — [`integrations/aseprite/pixelpipe`](integrations/aseprite/pixelpipe)
  adds a *File → PixelPipe: Convert to True-Pixel…* command that lays every
  frame onto one sheet, shells out to `pixelpipe convert --emit-palette`, and
  reimports the result as a new animation with the emitted palette applied. Zip
  the folder as `pixelpipe.aseprite-extension` and install it via
  *Edit → Preferences → Extensions*.

## Status model & exit codes

| Status   | Exit | Meaning                                                  |
| -------- | ---- | -------------------------------------------------------- |
| `pass`   | `0`  | All hard rules pass; no ambiguous inference was used.    |
| `review` | `2`  | Hard rules pass, but a segmentation/composition ambiguity exists. |
| `fail`   | `3`  | One or more game-asset hard rules failed.                |

Reports include stable `reasons[]` with SCREAMING_SNAKE_CASE codes (e.g.
`OUTLINE_EXTRA_PIXELS`, `PALETTE_LIMIT_EXCEEDED`) so agents can route on
outcomes without parsing prose.

## Workspace layout

| Crate                    | Responsibility                                       |
| ------------------------ | ---------------------------------------------------- |
| `pixel-formats`          | Stable contracts: profiles, reports, status codes.   |
| `pixel-core`             | Deterministic conversion pipeline.                   |
| `pixel-qa`               | Static QA and the release gate.                      |
| `pixel-cache`            | Content-addressed cache key helpers.                 |
| `pixel-provider`         | Semantic feature analysis (heuristic fallback + ONNX stub). |
| `pixelpipe` (`apps/`)    | The CLI.                                             |

## Development

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
