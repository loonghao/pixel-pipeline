-- PixelPipe × Aseprite bridge.
--
-- Adds "File > PixelPipe: Convert to True-Pixel..." which:
--   1. lays every frame of the active sprite onto one sheet (row-major),
--   2. runs `pixelpipe convert --grid RxC --emit-palette` — one shared
--      palette for the whole animation, QA-gated by the CLI,
--   3. reimports the converted cells as a new animation (frame durations
--      preserved) and applies the `.gpl` palette the CLI emitted.
--
-- Install: zip this folder as `pixelpipe.aseprite-extension` and add it via
-- Edit > Preferences > Extensions (or copy the folder into the Aseprite
-- extensions directory). The script shells out to pixelpipe, so grant it
-- full access when Aseprite asks.

local IS_WINDOWS = app.fs.pathSeparator == "\\"
local PROFILES = { "character-32", "character-48", "character-64", "custom" }

local function quote(s)
  return '"' .. s .. '"'
end

local function shell(cmd)
  if IS_WINDOWS then
    -- cmd.exe strips one outer quote pair; keep per-argument quoting intact.
    cmd = '"' .. cmd .. '"'
  end
  return os.execute(cmd)
end

-- Row-major grid layout for n frames, at most 8 columns.
local function grid_for(n)
  local cols = math.min(n, 8)
  return math.ceil(n / cols), cols
end

-- pixelpipe sheet-mode cell output path (`stem_rRcC.png`).
local function cell_path(dir, stem, row, col)
  return app.fs.joinPath(dir, string.format("%s_r%dc%d.png", stem, row, col))
end

-- Reimport converted cells as one animation; returns the new sprite or nil.
-- Cells pixelpipe skipped (fully transparent frames) stay as empty frames so
-- frame indices and durations keep matching the source animation.
local function import_result(src, dir, stem, outPath, n, cols)
  local paths = {}
  for i = 0, n - 1 do
    paths[i + 1] = (n == 1) and outPath
      or cell_path(dir, stem, math.floor(i / cols), i % cols)
  end
  local first = nil
  for _, p in ipairs(paths) do
    if app.fs.isFile(p) then
      first = p
      break
    end
  end
  if not first then
    return nil
  end

  local img0 = Image{ fromFile = first }
  local spr = Sprite(img0.width, img0.height, ColorMode.RGB)
  spr.filename = app.fs.joinPath(dir, stem .. ".aseprite")
  for i = 1, n do
    local frame = (i == 1) and spr.frames[1] or spr:newEmptyFrame()
    frame.duration = src.frames[i].duration
    if app.fs.isFile(paths[i]) then
      spr:newCel(spr.layers[1], frame, Image{ fromFile = paths[i] }, Point(0, 0))
    end
  end

  local gpl = app.fs.joinPath(dir, stem .. ".gpl")
  if app.fs.isFile(gpl) then
    spr:setPalette(Palette{ fromFile = gpl })
  end
  return spr
end


local function convert(prefs)
  local src = app.activeSprite
  if not src then
    return app.alert("PixelPipe: open a sprite first")
  end

  local defaultDir = src.filename ~= "" and app.fs.filePath(src.filename) or ""
  local dlg = Dialog("PixelPipe Convert")
  dlg:file{ id = "bin", label = "pixelpipe binary", filename = prefs.bin or "pixelpipe", open = true }
  dlg:combobox{ id = "profile", label = "profile", option = prefs.profile or "character-48", options = PROFILES }
  dlg:entry{ id = "customProfile", label = "custom .toml", text = prefs.customProfile or "" }
  dlg:check{ id = "detect", label = "features", text = "detect face/eyes", selected = prefs.detect ~= false }
  dlg:entry{ id = "outDir", label = "output dir", text = prefs.outDir or defaultDir }
  dlg:check{ id = "reimport", label = "result", text = "reimport as new sprite", selected = true }
  dlg:button{ id = "ok", text = "Convert", focus = true }
  dlg:button{ id = "cancel", text = "Cancel" }
  dlg:show()
  local d = dlg.data
  if not d.ok then
    return
  end

  prefs.bin, prefs.profile, prefs.customProfile = d.bin, d.profile, d.customProfile
  prefs.detect, prefs.outDir = d.detect, d.outDir

  local profileArg = (d.profile == "custom") and d.customProfile or d.profile
  if profileArg == "" then
    return app.alert("PixelPipe: pick a profile (or fill in the custom .toml path)")
  end
  local dir = d.outDir
  if dir == "" then
    dir = app.fs.tempPath or "."
  end
  app.fs.makeAllDirectories(dir)

  local title = src.filename ~= "" and app.fs.fileTitle(src.filename) or "sprite"
  local stem = title .. "-pixel"
  local sheet = app.fs.joinPath(dir, stem .. "-src.png")
  local outPath = app.fs.joinPath(dir, stem .. ".png")
  local logPath = app.fs.joinPath(dir, stem .. ".log")

  -- One sheet, row-major, no trim: uniform cells for `--grid`, and the CLI
  -- builds a single shared palette across the whole animation.
  local n = #src.frames
  local rows, cols = grid_for(n)
  app.command.ExportSpriteSheet{
    ui = false,
    askOverwrite = false,
    type = SpriteSheetType.ROWS,
    columns = cols,
    textureFilename = sheet,
    trim = false,
  }

  local cmd = quote(d.bin) .. " convert " .. quote(sheet)
    .. " -o " .. quote(outPath)
    .. " --profile " .. quote(profileArg)
    .. " --no-sidecars --emit-palette"
  if n > 1 then
    cmd = cmd .. " --grid " .. rows .. "x" .. cols
  end
  if d.detect then
    cmd = cmd .. " --detect-features"
  end
  cmd = cmd .. " > " .. quote(logPath) .. " 2>&1"

  local ok = shell(cmd)
  os.remove(sheet)

  local anyOut = false
  for i = 0, n - 1 do
    local p = (n == 1) and outPath or cell_path(dir, stem, math.floor(i / cols), i % cols)
    if app.fs.isFile(p) then
      anyOut = true
      break
    end
  end
  if not anyOut then
    return app.alert{
      title = "PixelPipe failed",
      text = { "Conversion produced no output.", "See log: " .. logPath },
    }
  end
  if not ok then
    -- Non-zero exit = QA review/fail; artifacts still exist, so keep going.
    app.alert{
      title = "PixelPipe QA",
      text = { "QA flagged review/fail; importing anyway.", "See log: " .. logPath },
    }
  end

  if d.reimport then
    local spr = import_result(src, dir, stem, outPath, n, cols)
    if spr then
      app.activeSprite = spr
    end
  end
end

function init(plugin)
  plugin:newCommand{
    id = "PixelPipeConvert",
    title = "PixelPipe: Convert to True-Pixel...",
    group = "file_export",
    onclick = function()
      convert(plugin.preferences)
    end,
  }
end

function exit(plugin) end
