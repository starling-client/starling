# Baked models and their keys

Every successful resource load bakes each block state's models and the standalone models once,
into one store for that load. That store belongs to the game and never changes while it is
published: the mesher, block entity drawing and a mod all read the same baked results. A mod
holds keys into it and borrows the geometry and metadata it needs, never the renderer's model
types. The [design](https://github.com/sunrise-org/sunrise/blob/main/docs/design/mod-api/baked-models.md)
covers it; block-entity model data and editing a baking result are separate changes.

[Register standalone files and bakers](model-drawing.md#standalone-model-bakers) once at
client startup. During a resource load a baker appends its declared model files through a
borrowed `StandaloneModelBakeContext`; it cannot edit a model another listener registered.
Only a completed assembly enters the store. Read its published quads by the result ID,
just as for a directly registered file.

## Keys

A `BakedModelKey` names one baked model in one published store: its resource generation and its
slot. Ask for them in `model_baking_completed`, when the new store is published, and keep them
until the next one:

```rust,ignore
fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
    let assets = host.render_assets();
    // Use the same BlockState you get from a level read or a placement event.
    let Some(state) = host.block_state_id("minecraft:stone") else { return };
    let Ok(keys) = assets.block_state_models(state) else { return };
    // A standalone model's handle names a baked model too.
    let part = Identifier::parse("mymod:block/part")
        .ok()
        .and_then(|id| assets.model(&id).ok())
        .map(BakedModelKey::from);
    // A state no blockstate file binds has no keys.
    let Some(first) = keys.iter().next().or(part) else { return };
    // Keep `first` if you need to look up this model again later in the same generation.
    let Ok(view) = assets.baked_model(first) else { return };
    for quad in view.quads() {
        // quad.positions, quad.uvs and quad.sprite borrow data from this generation.
        // quad.baked_argb is separate from quad.tint_index.
    }
}
```

| Call | Answers |
| --- | --- |
| `block_state_models(state: BlockState)` | A borrowed `BakedModelKeys<'_>` view of every model the state can select, in bake order without repeats. Read `len()` or `is_empty()` without scanning, or iterate over the view directly or with `iter()` to get keys by value without picking a position's seeded variant. Equality compares the generation and the ordered keys. Empty for a valid state with no baked binding. `UnknownModel` if the live state index lacks the state. |
| `block_state_model(state: BlockState)` | The state as one drawable `ModelRef` (see [Model drawing](model-drawing.md)). `UnknownModel` for air, a state no blockstate file binds, or a state the live state index lacks. |
| `baked_model(key)` | A `BakedModelView<'_>` borrowing the geometry and `particle` sprite name, and giving the model's `ambient_occlusion` flag. `StaleModel` once a reload superseded the key; `UnknownModel` for a slot the store lacks. |

The model borrows from `render_assets()`. `view.quads()` iterates in bake order without copying the
geometry or allocating a list. Use `view.quad(i)` for direct, zero-based lookup in the same order;
it returns `None` out of range. Keep the keys, not the views, between callbacks. `quad.baked_argb`
is the baked `0xAARRGGBB` colour if set; `quad.tint_index` names a block tint source, or is `-1`
for none. Neither is the final rendered colour. The view does not carry resolved lighting,
render passes, per-quad ambient occlusion overrides, or shading overrides. The quad iterator
reports its remaining length in constant time (`quads.len()` or `quads.count()`) and can walk
from either end (`quads.next_back()` or `view.quads().rev()`).

## Generations

A successful reload (F3+T, a pack turned on or off, a server's pack) bakes a new store from the
resources as they are now, never from the previous store, and publishes it before
`model_baking_completed`. Keys from the previous load then answer `StaleModel`; ask again. A
failed reload publishes nothing, and the keys of the last successful load stay valid. A dedicated
server bakes no models: its store is empty.

All three calls are available through `ClientHost::render_assets()` in Setup, Reloading and World.
A state the live index lacks gives `UnknownModel` on both.
