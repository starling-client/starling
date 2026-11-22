# Drawing a layer over a living entity

A client mod can draw over a living entity without replacing its model: a mod's armour, a glow, a
backpack, something held or worn. This is the counterpart of NeoForge 26.3's
`EntityRenderersEvent.AddLayers` and `RenderLayer`: the mod adds a layer to a living entity type or to
a player model, and the game draws it over every entity of that target in view, after its model and
the layers before it.

## 1. Write the layer

A layer implements `EntityRenderLayer`. Its `submit` gets the entity's render state, the borrowed
final model if the body was posed, and a draw context. This is the context a mob or block entity
submit hook draws with (`quad`, `cube`, `block`, `item`, `model_part`, `push`, `pop`, `translate`,
`rotate_about`, `scale`):

```rust,ignore
use mod_api::*;
use std::sync::Arc;

struct Halo;
impl EntityRenderLayer for Halo {
    fn submit(
        &self,
        _entity: &EntityRenderState,
        model: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        // Follow the head's visibility when there is one. Still draw on a replacement
        // renderer or a body without a part named "head".
        if model.and_then(|body| body.part("head")).is_some_and(|head| !head.visible) {
            return Ok(());
        }
        // In the entity's frame: at its feet, turned with its body. Any atlas sprite will do:
        // a vanilla one, as here, or one of the mod's own assets.
        let face = Some(CubeFace::new("block/gold_block"));
        draw.cube([-4.0, 34.0, -4.0], [4.0, 35.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}
```

The pose stack starts in the entity's frame: at its feet, turned with its body, +z the way it faces,
+x its left, +y up, in blocks, as `render_living_pre` draws. `entity.yaw` is the body's turn and
`entity.pitch` the head's. If `model` is `Some(body)`, look up `body.part(name)` to
[read a part's final local pose](model-drawing.md#read-the-final-body-model). To attach geometry,
compose its parents' poses and use `visible` if the attachment should disappear with the body part.
Independent layers can draw without a body part. The model is borrowed only for `submit`. `draw`
also has the partial tick, game time and entity light. Every entity of a target shares the one
layer, so keep per-entity values out of it. A layer has no `Host`; one that needs the world reads
it in a hook and keeps what it read.

## 2. Add it

```rust,ignore
fn register_entity_layers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityLayersEvent<'_>,
) {
    let halo = Arc::new(Halo);
    for target in [
        LayerTarget::Entity("minecraft:zombie".into()),
        LayerTarget::Player(PlayerModel::Wide),
        LayerTarget::Player(PlayerModel::Slim),
    ] {
        event.layers.add(target, halo.clone()).unwrap();
    }
}
```

The hook is asked on every resource load, the first one and every reload (F3+T, a resource pack
change), after the entity renderers resolve: a reload builds the renderers again, and layers don't
carry over, so add them each time. The event also tells what renderers this load built:

| Method | Tells |
| --- | --- |
| `entity_types()` | every entity type with a renderer, vanilla's and the mods', sorted (not `minecraft:player`: a player's renderers are its models') |
| `is_living(type)` | whether the type's renderer is a living entity's, the only kind that takes layers |
| `player_models()` | `Wide` and `Slim` |
| `has_mod_renderer(type)` | whether a mod registered the type's renderer ([entity renderers](entity-renderers.md)) |

- A layer attaches only to a living renderer, a mod's own included. `add` refuses a target that is no
  entity type, a type whose renderer isn't a living one (a boat, a display, a projectile),
  `minecraft:player`, whose layers go to a player model, and a living type that draws through a route
  of its own which doesn't run layers yet (the armour stand, the ender dragon): the `Err` names the
  target, and the game logs it too.
- Several mods may add to one target: the layers draw in successful registration order, after the
  entity's model and its vanilla layers. A [failed registration callback](../../concepts/load-phases.md#registration-and-setup)
  adds no layers; other callbacks can still register theirs. On a type a mod registered a renderer for
  ([entity renderers](entity-renderers.md)), they draw after that renderer, which replaces the model
  and the vanilla layers. They receive `model: None` when the replacement renderer succeeds.
- An entity whose `render_living_pre` or `render_player_pre` a mod canceled draws none of its layers,
  as NeoForge's canceled `RenderLivingEvent.Pre` skips them.
- `submit` returns `Ok(())` to keep what it drew. A layer that returns an `Err` or panics is logged once and asked no more until the next resource load
  adds it again; what it drew that frame is dropped.

## Where layers draw

The game resolves each target's layers when they are added, so a frame finds an entity's layers with
no lookup by name and no scan over the mods; with no layer at all, it checks one flag per entity and
builds nothing. The draw retains its existing body poses only when the target has a layer to draw
or a Post listener can read them. A layer on one type does not request captures for other types.
The layers draw on the mobs (vanilla's and the content mods' living types) and on the players,
including this window's own in third person when it holds the camera. In the ordered body pass, layers
submit in level join order, interleaved with general entity renderer submissions. The armour stand
and the ender dragon have draws of their own that don't run layers, so `add` refuses them.

Use `render_living_post` and `render_player_post` to draw after the layers without registering for
one specific type (NeoForge's `RenderLivingEvent.Post` and `RenderPlayerEvent.Post`). They borrow the
same final model when the draw produced one, subject to
[Pre cancellation](model-drawing.md#before-and-after-a-living-entity-draws).
