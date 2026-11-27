# Shader packs

Sunrise runs shader packs in the Iris/OptiFine `shaders/` format: a folder or a `.zip` in `shaderpacks/` with a
`shaders/` folder inside, written in GLSL. The format Sunrise implements is defined in
[cleanroom/shader-packs-spec.md](cleanroom/shader-packs-spec.md); `crates/sunrise-shaders` reads a pack and translates
its GLSL, and the renderer draws it.

Sunrise's own WGSL pack format (`sunrise-shaders.toml` and the `.sshader` package, API 0.1 and 0.2) is removed. A
`.sshader` file or such a folder in `shaderpacks/` is left out of the list with a note that its format is no longer
supported, and a saved choice of one falls back to no pack with the same note. The registry lists no such pack.

## Trying a pack in the game

Put the pack folder (or its `.zip`) in `shaderpacks/` beside `options.txt` (Open Pack Folder opens it) and pick
it on Options > Sunrise Settings > Shaders; one runs at a time, and clicking it again turns it off. Its
Shader Options... row opens the picked pack's own settings.

The Shaders page of Sunrise Settings has a Shader pack key for the same pick: press it and the picked pack turns off,
press it again and the same pack runs again. It is bound there (click the row, then press a key; Escape unbinds it),
`K` out of the box, and it is not a vanilla key binding, so rebinding it never turns into another mapping's key
(Replay Studio keeps physical `K` for its camera keyframe). Turning off remembers the pick in the Extras settings,
so the pack stays off across restarts until the key is pressed again; the menu's second click on the picked pack
does the same.

A picked pack's options open from Shaders' **Shader Options...** row, on a screen of their own, under the pack's name
in its page list: its screens, with the names the pack gives them, its profiles, and Reset. The row is greyed, saying
why, while no pack is picked or the pick's options couldn't be read. The main screen and those it links to are in the list (when
there are more than it has room for, they collapse into one `‹ Name ›` row that the arrows and a controller's
bumpers step through); deeper screens open from their links, and Escape goes back. A click, Enter or Right picks an
option's next value, a Shift click or Left the one before, and the options the pack lists in
`sliders` are sliders. Pointing at one shows what the pack says of it (`option.NAME.comment`); the search finds an
option on any screen. Shader Options' Undo puts back the pack's options as the screen opened with them (and the
remembered pick, which the key can change meanwhile); the pick and the Shadow Cache are Sunrise Settings', whose Undo
puts them back. Picking away from a pack saves its changed options, as Apply does. A pack's
options are saved to `shaderpacks/<pack>.txt` (`NAME=value` lines), and the pack loads again with them, with Apply
(shown while they differ from the saved ones) or on leaving the screen.

The **Shadow Cache** (off by default; `shadowCache` in `sunrise-graphics.txt`) skips the shadow pass and keeps the
shadow map and shadowcolor buffers of the frame before while nothing that draws them has changed (the world, the
time, the snapped shadow camera, the shadow settings, and the uniforms the shadow programs and their compute stages
read). It keeps them only while the pass would draw the same again: the shadow pass still runs every frame while an
entity casts a shadow, while the pack has `image.*` images, while a shadow program draws into, flips or stores into a
shadowcolor buffer that isn't cleared every frame (`shadowcolorNClear=false`: the program builds on the frame
before), while a shadow program or a compute stage attached to one binds the pack's `bufferObject.*` storage buffers
(they keep their contents between frames, so it may read or write something new each frame), and while a program
outside the shadow pass binds `shadowcolorimg0` or `shadowcolorimg1`, since its stores would otherwise pile up in the
kept buffers.

## Testing a pack frame by frame

A pack that keeps something from frame to frame (buffers, images, smoothed values) draws a little differently in every run,
as frames come when they come. With `SUNRISE_STILL=1` and the world frozen the scene holds still
([agent-control.md](agent-control.md#repeatable-screenshots)), and

```sh
sunrise-ctl screenshot-sequence 64 50 frame64.png
```

starts the pack over as if just loaded and draws 64 frames 50 ms apart on its clock: frame n is at n frames and
(n - 1) × 0.05 seconds. The
64th is written to `frame64.png` (and `frame64-hdr.png` with HDR output on) before the reply. STEP_MS may be a
decimal: `16.666667` steps a 60 fps clock. The same build, GPU
and driver write the same bytes every run, so two versions of a pack compare by a pixel diff. `screenshot-sequence
cancel` stops one.
