# Playing entity keyframe animations

A mod or resource pack can ship a NeoForge-format entity animation. Resolve it through the
client's `RenderAssets::animation`, bake its bone names against your `ModelPart` tree once,
and keep the result for playback. Vanilla mobs use that sampler too.

## Write the resource

Put this file at `assets/demo/neoforge/animations/entity/arm/wave.json`. Its animation ID is
`demo:arm/wave`:

```json
{
  "length": 1.0,
  "loop": true,
  "animations": [
    {
      "bone": "arm",
      "target": "minecraft:rotation",
      "keyframes": [
        {"timestamp": 0.0, "target": [0, 0, 0], "interpolation": "minecraft:linear"},
        {"timestamp": 0.5, "target": [90, 0, 0], "interpolation": "minecraft:linear"},
        {"timestamp": 1.0, "target": [0, 0, 0], "interpolation": "minecraft:linear"}
      ]
    }
  ]
}
```

`length` and `timestamp` are seconds. Keep keyframes in time order. A number that isn't
finite (as an f32), or a `length` of 0 or less, rejects the file with a named diagnostic.
`loop` defaults to false. The built-in targets use vanilla's builder conventions:

- `minecraft:position`: texels, with the JSON y value negated when loaded.
- `minecraft:rotation`: degrees in the file, converted to radians when loaded.
- `minecraft:scale`: scale factors in the file; `[1, 1, 1]` adds no scale offset.

The built-in interpolations are `minecraft:linear` and `minecraft:catmullrom`. An identifier
without a namespace uses `minecraft:`. A target or interpolation that is neither built in nor
registered by a mod ([below](#registered-targets-and-interpolations)) rejects that file with a
named diagnostic; other animation files load independently. Lookup returns `None` when the ID
is absent or its file failed to load.

## Resolve and bake after resources are published

Resolve during `client_setup`, and resolve again during `client_resources_reloaded`. These
are published-resource reads, not per-frame work. For example, renderer-owned state can keep
its model and baked animation together:

```rust,ignore
use mod_api::{AnimationState, Identifier, KeyframeAnimation, ModelPart, RenderAssets};

struct AnimatedModel {
    root: ModelPart,
    animation: Option<KeyframeAnimation>,
    state: AnimationState,
}

impl AnimatedModel {
    fn new(root: ModelPart) -> Self {
        Self { root, animation: None, state: AnimationState::new() }
    }

    fn reload(&mut self, assets: &dyn RenderAssets, id: &Identifier) {
        self.animation = assets.animation(id).map(|holder| holder.bake(&self.root));
    }

    fn start(&mut self, tick_count: i32) {
        self.state.start(tick_count);
    }

    fn pose(&mut self, age_in_ticks: f32) {
        self.root.reset_all_poses();
        if let Some(animation) = &self.animation {
            animation.apply(&mut self.root, &self.state, age_in_ticks);
        }
    }
}
```

Call `reload` from those lifecycle hooks with the published assets and ID, and `start` when
that entity begins the action. Keep a separate animation state for each animated entity. A new
state is stopped; `start_if_stopped` does not restart an already playing animation,
`animate_when` starts or stops with a condition, and `stop` ends playback.

Baking follows vanilla 26.3's `ModelPart.createPartLookup`: the name `root` always means the
actual root, even if a child is also named `root`. Other names select the first match,
visiting each part's immediate children before their descendants. A missing bone is logged
and dropped at bake time; the other channels play. Re-bake if the model hierarchy
changes, not just when animation resources change.

## Apply a pose each frame

The model's pose is mutable, and animation offsets add to it. Reset to the initial pose
before applying that frame's animations, as in `pose` above; otherwise offsets accumulate.
`apply` measures elapsed time from the state's start tick at 50 ms per tick.
`apply_scaled` changes playback speed. For a walk cycle use
`apply_walk(root, walk_pos, walk_speed, speed_factor, scale_factor)`: its time is
`walk_pos * 50 * speed_factor` milliseconds, and its strength is
`min(walk_speed * scale_factor, 1)`. `apply_static` applies the first frame without a state.
Looping animations wrap elapsed time by their length.

Do not look up, clone or bake the animation every frame: those steps copy or allocate data.
Playback uses the retained child-index paths and channels without allocating or searching
bone names. Pose application alone does not draw anything; submit your model through its
renderer, alongside the [resolved-model drawing](../client/model-drawing.md) facilities.

## Registered targets and interpolations

A mod registers custom targets and interpolations once at client startup in
`Hooks::register_json_animation_types` (NeoForge's `RegisterJsonAnimationTypesEvent`), binding
each identifier directly to its callbacks via `event.register_target(id, apply, convert_target)`
and `event.register_interpolation(id, interpolate)`.

When animation files are parsed during resource load and reload, each custom target's
`convert_target` transforms its keyframe `[x, y, z]` vectors into internal units (matching
NeoForge's `AnimationParser`, just as the built-in targets apply `posVec`, `degreeVec` and
`scaleVec`), and the bound target and interpolation callbacks are attached directly to the
parsed `AnimationHolder`. Baking with `holder.bake(&root)` produces a `KeyframeAnimation` that
invokes those bound callbacks directly during `apply`, `apply_scaled`, `apply_walk` and
`apply_static`, with no lookup per sample. Any mod's animation file can use a custom target or
interpolation registered by another mod without needing a separate implementation at bake time.

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; duplicates and
collisions with the five built-ins (`minecraft:position`, `minecraft:rotation`,
`minecraft:scale`, `minecraft:linear`, `minecraft:catmullrom`) are rejected as registration
errors. See the [`register_json_animation_types` documentation](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/register_json_animation_types.rs)
for the registration contract.
