extends Node2D
## Side-by-side validation of the pipeline's true-pixel output at multiple
## resolutions. Reads manifest.json, builds one AnimatedSprite2D rig per size
## (32/48/64 px) and drives them from a single shared input so the same
## character walks/runs/idles in every column at once. Tab cycles between the
## compare row and a solo (zoomed) view of each size.

const MANIFEST_PATH := "res://manifest.json"
const BASELINE_Y := 452.0        # shared feet line for every column
const COMPARE_H := 192.0         # on-screen sprite height in the compare row
const SOLO_H := 384.0            # on-screen sprite height in solo view

@export var swap_seconds := 0.0  # 0 = manual; >0 auto-advances the demo state

var _facing := "down"
var _state := "idle"
var _view := -1                  # -1 = compare all; 0..n-1 = solo that size
var _rigs: Array = []            # [{ sprite, label, cell, x }]
var _anims: Array = []

@onready var _hud: CanvasLayer = $HUD
@onready var _info: Label = $HUD/Info

func _ready() -> void:
	var manifest := _load_manifest()
	if manifest.is_empty():
		push_error("manifest.json missing or invalid at %s" % MANIFEST_PATH)
		return
	_anims = manifest.get("animations", [])
	var sizes: Array = manifest.get("sizes", [])
	var flip_right := bool(manifest.get("right_is_flipped_left", true))
	var n := sizes.size()
	var span := 1152.0
	for i in n:
		var size_def: Dictionary = sizes[i]
		var cell := int(size_def.get("cell", 64))
		var dir := String(size_def.get("dir", "res://assets/"))
		var x := span * (float(i) + 0.5) / float(max(n, 1))
		var sprite := AnimatedSprite2D.new()
		sprite.sprite_frames = _build_frames(dir)
		sprite.centered = true
		sprite.position = Vector2(x, 0.0)
		add_child(sprite)
		var label := Label.new()
		label.text = "%s  (%s)" % [String(size_def.get("label", "%dpx" % cell)),
			String(size_def.get("profile", ""))]
		label.position = Vector2(x - 90.0, BASELINE_Y + 16.0)
		label.size = Vector2(180.0, 24.0)
		label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		_hud.add_child(label)
		_rigs.append({ "sprite": sprite, "label": label, "cell": cell, "x": x,
			"flip_right": flip_right })
	_relayout()
	_apply_animation()

func _load_manifest() -> Dictionary:
	if not FileAccess.file_exists(MANIFEST_PATH):
		return {}
	var text := FileAccess.get_file_as_string(MANIFEST_PATH)
	var data: Variant = JSON.parse_string(text)
	return data if typeof(data) == TYPE_DICTIONARY else {}

func _build_frames(dir: String) -> SpriteFrames:
	var frames := SpriteFrames.new()
	frames.remove_animation("default")
	for anim in _anims:
		var prefix := String(anim["prefix"])
		var fps := float(anim.get("fps", 8))
		var loop := bool(anim.get("loop", true))
		var count := int(anim.get("frames", 1))
		var directions: Dictionary = anim["directions"]
		for dir_name in directions.keys():
			var row := int(directions[dir_name])
			var anim_name := "%s_%s" % [anim["name"], dir_name]
			frames.add_animation(anim_name)
			frames.set_animation_loop(anim_name, loop)
			frames.set_animation_speed(anim_name, fps)
			for col in count:
				var path := "%s%s_r%dc%d.png" % [dir, prefix, row, col]
				var tex: Texture2D = load(path)
				if tex != null:
					frames.add_frame(anim_name, tex)
				else:
					push_warning("missing frame: %s" % path)
	return frames

func _relayout() -> void:
	for i in _rigs.size():
		var rig: Dictionary = _rigs[i]
		var sprite: AnimatedSprite2D = rig["sprite"]
		var cell: int = rig["cell"]
		var visible := _view == -1 or _view == i
		sprite.visible = visible
		rig["label"].visible = _view == -1
		var target_h := COMPARE_H if _view == -1 else SOLO_H
		var s := target_h / float(cell)
		sprite.scale = Vector2(s, s)
		var x: float = rig["x"] if _view == -1 else 576.0
		sprite.position = Vector2(x, BASELINE_Y - target_h * 0.5)

func _process(delta: float) -> void:
	var input := _read_input()
	var running := Input.is_key_pressed(KEY_SHIFT)
	if input.length() > 0.1:
		input = input.normalized()
		_facing = _facing_from_vector(input)
		_state = "run" if running else "walk"
	else:
		_state = "idle"
	_apply_animation()
	_update_info(running)

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_TAB:
			_view = -1 if _view == _rigs.size() - 1 else _view + 1
			_relayout()
		elif event.keycode >= KEY_1 and event.keycode <= KEY_3:
			_view = int(event.keycode) - int(KEY_1)
			_relayout()
		elif event.keycode == KEY_0:
			_view = -1
			_relayout()

func _read_input() -> Vector2:
	var v := Vector2(
		Input.get_action_strength("ui_right") - Input.get_action_strength("ui_left"),
		Input.get_action_strength("ui_down") - Input.get_action_strength("ui_up"))
	if Input.is_physical_key_pressed(KEY_D): v.x += 1.0
	if Input.is_physical_key_pressed(KEY_A): v.x -= 1.0
	if Input.is_physical_key_pressed(KEY_S): v.y += 1.0
	if Input.is_physical_key_pressed(KEY_W): v.y -= 1.0
	return v

func _facing_from_vector(v: Vector2) -> String:
	if absf(v.x) > absf(v.y):
		return "right" if v.x > 0.0 else "left"
	return "down" if v.y > 0.0 else "up"

func _apply_animation() -> void:
	var dir := "left" if _facing == "right" else _facing
	for rig in _rigs:
		var sprite: AnimatedSprite2D = rig["sprite"]
		sprite.flip_h = _facing == "right" and bool(rig["flip_right"])
		var anim_name := "%s_%s" % [_state, dir]
		if sprite.sprite_frames == null or not sprite.sprite_frames.has_animation(anim_name):
			continue
		if sprite.animation != anim_name or not sprite.is_playing():
			sprite.play(anim_name)

func _update_info(running: bool) -> void:
	if _info == null:
		return
	var mode := "compare all" if _view == -1 else "solo %s" % _rigs[_view]["label"].text
	_info.text = "Pixel Pipeline - Multi-size Asset Validation\n" \
		+ "Move: Arrow keys / WASD    Run: hold Shift    View: Tab / 1-3 / 0\n" \
		+ "State: %s    Facing: %s%s    View: %s" % [
			_state, _facing, ("  [SHIFT]" if running else ""), mode]
