# Asset automation audit (2026-09-07)

Scope: remote `main` at `a0b2505`, plus this change. The separate local feature
branches were not merged or modified. GitHub had no issues and one open PR
(Renovate configuration) at audit time; absence of issues is not evidence that
the production workflow is complete.

## Implemented correction

The old grid slicer used `width / columns` and `height / rows` for every cell.
For a 9-pixel-wide sheet split into two columns, column 8 disappeared. A pose
located there produced no output. Grid boundaries now partition the entire
image using widened integer arithmetic. Fixed-size cells retain clipped edge
regions, including when a requested cell is larger than the source. Invalid
dimensions fail before iteration and artifact writes.

The Rust `sheet::slice` API now returns `Result<Vec<Cell>, CoreError>`; Rust
callers must handle invalid specifications. CLI flags, valid divisible-grid
behavior, empty-cell skipping, output naming and JSONL ordering are unchanged.

Validation covers source-pixel coverage without overlap, extreme dimensions,
and an actual generated PNG passed through the CLI. The latter decodes the
output, checks binary alpha and target dimensions, repeats the conversion for
byte stability, and verifies invalid input creates no output directory. This
is a procedural regression fixture, not an artist-authored asset or editor
acceptance test.

## Architecture and remaining gaps

The crate split is useful: formats owns contracts, core owns deterministic
conversion, QA evaluates metrics, and the CLI orchestrates artifacts. Recent
main changes added semantic heuristics and quantize-before-snap convergence.
The ONNX provider remains a stub; existing heuristic output is not learned
semantic reconstruction.

| Area | Observed gap | Required follow-up |
| --- | --- | --- |
| Animation | Empty cells are skipped; reports contain no duration, tags, pivots or source rectangles. | Define a versioned animation manifest before accepting packed/trimmed editor atlases. Keep explicit empty frames and transform metadata with geometry. |
| Aseprite extension | Exports a flattened sheet and restores durations; does not restore source layers, tags or slices. Reuses filenames and tests file existence after execution, so stale output can be imported after failure. | Isolate each run and read current-run reports before importing; add real editor regression coverage. |
| Batch resume | Any parseable old report is reused without checking input/profile hashes or artifact existence. | Validate the effective conversion identity and expected artifacts before reuse. |
| Automatic grid | Counts occupied bands and assumes a regular grid; irregular spacing can still split a pose. | Keep explicit geometry authoritative; require confidence/geometry checks before inferring a production animation. |
| Palette/alpha | Shared sheet palette and binary-alpha QA exist. | Retain palette-union checks across frames and test target-engine filtering/import settings; static QA alone does not establish engine acceptance. |
| Outputs | Atomic writes apply to individual files, not the complete set. | Use a run manifest or completion marker if downstream consumers watch output directories. |

## Editor interfaces and ownership boundary

[Aseprite CLI](https://www.aseprite.org/docs/cli/) supports batch mode, Lua
scripts and PNG sheet plus JSON data export. Its
[ExportSpriteSheet API](https://www.aseprite.org/api/command/ExportSpriteSheet)
provides structured export parameters. The existing extension is Aseprite-only.

[LibreSprite's option definitions](https://github.com/LibreSprite/LibreSprite/blob/master/src/app/app_options.cpp)
include batch, script, sheet, JSON array/hash, layer and tag options. Its
[own scripting API](https://github.com/LibreSprite/LibreSprite/blob/master/SCRIPTING.md)
exposes image pixel writes and different sprite/layer methods. Shared option
names do not prove script compatibility or equivalent behavior in installed
releases. Probe the actual binary version/help and run separate creation/export
fixtures for each editor before advertising support.

Pixel Pipeline should consume validated raster candidates and explicit asset
metadata; editor discovery, scripting and host lifecycle belong to the ecology
adapter work. No duplicate adapter or paid installation is introduced here.
No Aseprite/LibreSprite runtime or game-engine import was exercised in this
audit. Capability references are source/documentation evidence only.
