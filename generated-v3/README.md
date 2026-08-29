# Generated v3 food and identity corpus

This local experiment contains seven Codex-generated game sources: a food
monster, scarf-and-wrench character, snail, carnivorous plant, baozi, ramen
bowl, and pizza. Each source is compiled independently at 32×32, 48×48, and
64×64 with the named `character-*` or `item-64` profile. There are no chained
conversions.

The interactive lab is [`index.html`](index.html). It reads
[`resolution-data.js`](resolution-data.js) and displays the source beside each
resolution, the profile, edge recall, colour error, mask source, and status.
The machine-readable report is kept in `resolution-lab/reports.json` and the
source/variant contract in `manifest.json`.

`identity-annotations.jsonl` records conservative boxes and keypoints for
silhouettes, faces, eyes, mouths, weapons, scarf/ribbon tails, folds, noodles,
toppings, and other identity anchors. They are proposal labels generated from
the source images, not artist approval. All generated-v3 records are
`evaluation-only`; no model trained from them is production-ready.

## Reproduce locally

```powershell
python tools/build_generated_v3_annotations.py
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build_generated_v3_lab.ps1
python tools/build_generated_v3_supervised.py
python tools/pixel_advisor.py train `
  --candidates training/supervised-v2/candidates.jsonl `
  --pairs training/supervised-v2/pairs.jsonl `
  --allow-weak-labels `
  --output training/supervised-v2/model.warm-start.json
```

The advisor is a small explainable ranker for choosing among deterministic
compiler candidates. It cannot redraw sprites, replace semantic masks, or
override the static/artist/engine acceptance gates. GPU is not required for
this warm start; a future learned pixel renderer would need a separate
rights-cleared dataset, held-out semantic masks, and a GPU training job.
