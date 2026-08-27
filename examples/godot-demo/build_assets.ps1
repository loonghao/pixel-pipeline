#requires -Version 5.1
# Regenerate the demo's true-pixel frames from the source sheets.
# Run from anywhere: paths resolve relative to this script.
$ErrorActionPreference = "Stop"

$demo = $PSScriptRoot
$root = Resolve-Path (Join-Path $demo "..\..")
$bin = Join-Path $root "target\release\pixelpipe.exe"
$src = Join-Path $root "examples\assets"
$out = Join-Path $demo "assets"

if (-not (Test-Path $bin)) {
    Write-Host "Building pixelpipe..."
    Push-Location $root
    cargo build --release --bin pixelpipe
    Pop-Location
}

$jobs = @(
    @{ name = "idle"; file = "ChatGPT Image Aug 25, 2026, 01_42_12 AM (2).png"; grid = "4x4" },
    @{ name = "walk"; file = "ChatGPT Image Aug 25, 2026, 01_42_12 AM (1).png"; grid = "4x4" },
    @{ name = "run";  file = "ChatGPT Image Aug 25, 2026, 01_42_12 AM (3).png"; grid = "4x6" }
)

# One profile per on-screen comparison column. The Godot demo renders each of
# these at an integer scale chosen so all columns share the same height.
$sizes = @(
    @{ dir = "32"; profile = "character-32" },
    @{ dir = "48"; profile = "character-48" },
    @{ dir = "64"; profile = "character-64" }
)

foreach ($s in $sizes) {
    $sizeOut = Join-Path $out $s.dir
    New-Item -ItemType Directory -Force -Path $sizeOut | Out-Null
    foreach ($j in $jobs) {
        $in = Join-Path $src $j.file
        $dst = Join-Path $sizeOut ($j.name + ".png")
        & $bin convert $in -o $dst --profile $s.profile --grid $j.grid --detect-features --no-sidecars 1>$null
        Write-Host ("{0}/{1}: grid {2} -> exit {3}" -f $s.dir, $j.name, $j.grid, $LASTEXITCODE)
    }
}

Write-Host "Done. Open examples/godot-demo in Godot 4 and press F5."
