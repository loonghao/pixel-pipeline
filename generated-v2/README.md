# Generated v2 identity-stress corpus

This local corpus contains five Codex `imagegen` sources designed to exercise
the identity failures reported in the project: faces, held weapons, ribbons,
thin chains, small charms, and high-chroma accents. Each source was processed
with the same deterministic Pixel Pipeline compiler used by the 40-asset
regression corpus.

Open the local [comparison gallery](index.html) to inspect source and final
outputs side by side. `manifest.json` records the source hash, prompt focus,
profile, target size, compile contract hash, mask source, and static status.
Every compiler sidecar and report remains next to its output under `outputs/`.

The sources are retained as `evaluation-only` until the image-generation
provider terms and project ownership are explicitly confirmed. The white/flat
background variants are intentional stress cases: the compiler correctly marks
them `review` when foreground inference is uncertain instead of silently
accepting them as game-ready sprites.

## Re-run locally

```powershell
& .\target\release\pixelpipe.exe convert `
  training/generated-v2/raw/character/character-desert-ranger-v2.png `
  -o training/generated-v2/outputs/character/character-desert-ranger-v2.png `
  --profile character-48 --detect-features --emit-palette --pretty

& .\target\release\pixelpipe.exe convert `
  training/generated-v2/raw/prop/prop-ribbon-incense-burner-v1.png `
  -o training/generated-v2/outputs/prop/prop-ribbon-incense-burner-v1.png `
  --profile item-64 --emit-palette --pretty
```

Do not chain a converted output into another conversion. Add a new source ID
for a new Codex generation or an artist-redrawn revision, then record its own
hash and rights status.
