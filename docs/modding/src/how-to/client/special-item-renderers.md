# Drawing an item with a special model renderer

Use `register_special_model_renderer` (NeoForge's `RegisterSpecialModelRendererEvent`) when an
ordinary item model cannot describe the drawing. Register a factory for a `minecraft:special`
type. For each node of that type, the factory decodes its configuration and bakes a renderer
at client resource load. The renderer implements vanilla's `SpecialModelRenderer` split: extract
an argument from the item being drawn, then submit geometry. The last published registration
for a type wins. A [failed registration callback](../../concepts/load-phases.md#registration-and-setup)
changes neither the winning factory nor the replacement reports.

## Decode, bake, then draw

This native mod's skillet always draws the pan. It adds food one model pixel above the pan when
the stack contains more than one item. Keep your `mod_api` dependency and add serde with its
`derive` feature.

```rust,ignore
use mod_api::{
    ClientHost, Hooks, Identifier, RegisterSpecialModelRendererEvent, RegistrationContext,
    SpecialModelCollector, SpecialModelContext, SpecialModelFactory, SpecialModelGeometry,
    SpecialModelRenderer, SpecialModelStack, SpecialModelTints,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SkilletConfig {
    pan: String,
    food: String,
    food_height_pixels: i32,
}

struct Skillet {
    pan: SpecialModelGeometry,
    food: SpecialModelGeometry,
    food_height: f32,
}

impl SpecialModelRenderer for Skillet {
    type Argument = bool;

    fn extract_argument(&self, _host: &dyn ClientHost, stack: &SpecialModelStack<'_>) -> bool {
        stack.count() > 1
    }

    fn submit(
        &self,
        show_food: &bool,
        _context: &SpecialModelContext,
        collector: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String> {
        collector.submit_geometry(&self.pan, SpecialModelTints::Untinted)?;
        if *show_food {
            let pose = [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, self.food_height,
                0.0, 0.0, 1.0, 0.0,
            ];
            collector.with_pose(pose, |child| {
                child.submit_geometry(&self.food, SpecialModelTints::Untinted)
            })?;
        }
        Ok(())
    }
}

pub struct SkilletMod;

#[mod_api::sunrise_mod]
impl Hooks for SkilletMod {
    fn register_special_model_renderer(
        &self,
        _context: &RegistrationContext,
        event: &mut RegisterSpecialModelRendererEvent<'_>,
    ) {
        event.renderers.register(
            Identifier::parse("example:skillet").unwrap(),
            SpecialModelFactory::new(
                |fields| fields.decode::<SkilletConfig>(),
                |config, bake| {
                    Ok(Skillet {
                        pan: bake.bake_model(&config.pan)?,
                        food: bake.bake_model(&config.food)?,
                        food_height: config.food_height_pixels as f32 / 16.0,
                    })
                },
            ),
        );
    }
}
```

Put the definition in `assets/example/items/skillet.json` and the referenced models in
`assets/example/models/item/`. The decoder receives the fields of the inline `model` object
other than `type`. Here the three fields are required; the `base` is not part of the config.

```json
{
  "model": {
    "type": "minecraft:special",
    "base": "example:item/skillet",
    "model": {
      "type": "example:skillet",
      "pan": "example:item/skillet_pan",
      "food": "example:item/skillet_food",
      "food_height_pixels": 1
    }
  }
}
```

Use `bake_model` for geometry from the selected packs or `bake_geometry` for your own
`SpecialModelQuad`s referencing item or block atlas sprites. Matching vanilla's `FaceBakery` and
`MaterialBaker`, `bake_geometry` defaults degenerate quads to `Up`, accepts any finite UV, and
substitutes the missing texture sprite when a sprite is absent while preserving
`force_translucent`. Non-finite coordinates or UVs, invalid sprite identifiers, out-of-range
tint indices and light emissions over 15 still fail the bake. Keep the resulting geometry leases
on the renderer. Do not look up or rebake models during `submit`.

`extract_argument` runs for each draw, even for an unchanged stack. It gets a `ClientHost` and a
borrowed `SpecialModelStack`: `item()`, `count()` and `get(&descriptor)` read only what you need.
Component reads include item defaults unless the stack overrides or removes them. If a bundle
selects the item, this is the selected stack, not the bundle. `submit` receives the argument and
the actual `SpecialModelContext`, with the display context, optional level game time and partial
tick. The game does not cache the renderer's draw output.

## Submit geometry and nested stacks

- `submit_geometry(&lease, tints)` submits baked geometry under the current pose. Use
  `SpecialModelTints::Untinted`, `Uniform(argb)` for every quad, or `Indexed(palette)` with a
  shared `Arc<[u32]>` indexed by quad tint index. Colors are packed ARGB. With an indexed
  palette, an unindexed quad or a missing entry draws white before lighting.
- `with_pose(transform, |child| ...)` lends a collector at a child pose. The transform is a
  row-major 3×4 affine; the current pose multiplies it on the left, and the composed pose must
  be finite. Model space is in blocks from the model's corner (element coordinates divided by
  16). When the callback returns, subsequent submissions on the parent use the unchanged
  parent pose. Submit directly on the parent when no local transform is needed.
- `submit_stack(&stack, ItemDisplayContext::Fixed)` resolves an item stack with its own models,
  display transform, tints and foil in the specified context. The current pose places its
  centre. There is no nested stack count cap. A repeated `(stack, display)` pair or insufficient
  native stack space to resolve the next child refuses the enclosing layer. Delegating the
  same stack in a different `ItemDisplayContext` is not a cycle.

The special node's `base` supplies its display transforms, particle and GUI light; any node
`transformation` also applies. Geometry submitted directly uses the outer item's foil. The
engine carries packed light, overlay and outline through the ordinary item draw path, so the
mod does not pass those values to the collector.

## Reloads and failures

Registration runs once at client startup, not on reload. Pass a validated `Identifier` (bare
paths default to `minecraft:`, matching vanilla and NeoForge). The last registration of a type
wins, including replacements for vanilla types. Each resource load decodes the winning
definitions and bakes new renderer instances and leases for enabled factories. Submit only
leases from the current load. Item states already resolved before a reload retain their baked
geometry and can finish drawing it.

An unknown type, a disabled winning factory, or an error or panic in decode or bake refuses the
enclosing item definition. That item draws the missing model. Other definitions still load; the
game does not try the built-in decoder for an overridden type or a lower pack's definition.

The factory, argument extraction and submission run as the mod that registered the type.
A failed draw rolls back the entire special layer, including earlier and nested submissions.
The game logs each ordinary refusal without disabling the callback: a subsequent valid draw
can still succeed. Ignoring a collector error does not make the layer commit; propagate it with `?`.

If argument extraction or submission panics, the game logs an error and shows an error toast
naming the mod. It disables the draw callback for that registered type, including all of its baked
nodes and any rebuilt on reload, for the rest of the session. The factory and other registrations
are not disabled by that draw panic, even if the panic came from a nested submission. The outermost
refused layer draws the missing model; unrelated item layers still draw normally.
