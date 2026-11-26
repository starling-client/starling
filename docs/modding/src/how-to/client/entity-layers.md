# Drawing a layer over a living entity

A client mod can inspect a living entity renderer's stock layers, hide stock layers permanently for
a resource load, and add layers of its own without replacing the entity's model: a mod's armour, a
glow, a backpack, something held or worn, or a replacement for a snow golem's pumpkin. This is the
counterpart of NeoForge 26.3's `EntityRenderersEvent.AddLayers` and `RenderLayer`: the mod edits a
living entity type's or player model's layers on each resource load, and the game draws each target's
added layers over every entity of that target in view, after its model, its unhidden stock layers
and the mod layers before it.

## 1. Write the layer

A layer implements `EntityRenderLayer`. Its `submit` gets the entity's render state, the borrowed
final model if the body was posed, a draw context (the one a mob or block entity submit hook draws
with: `quad`, `cube`, `block`, `item`, `model_part`, `push`, `pop`, `translate`, `rotate_about`,
`scale`), and a `StockLayerDraw` handle for conditionally redrawing hidden stock layers:

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
        _stock: &mut dyn StockLayerDraw,
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

## 2. Add or hide layers

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
change), after the entity renderers resolve: a reload builds the renderers again, and neither added
layers nor hidden stock layers carry over, so register them each time. The event also tells what
renderers this load built:

| Method | Tells |
| --- | --- |
| `entity_types()` | borrows the sorted slice of every entity type with a renderer, vanilla's and the mods' (not `minecraft:player`: a player's renderers are its models') |
| `is_living(type)` | whether the type's renderer is a living entity's, the only kind that takes layers |
| `player_models()` | borrows `[Wide, Slim]` |
| `has_mod_renderer(type)` | whether a mod registered the type's renderer ([entity renderers](entity-renderers.md)) |
| `stock_layers(&target)` | borrows the slice of `StockLayer` variants drawn by `target`'s renderer (empty for a mod renderer or a target that takes no layers) |

- A layer attaches only to a living renderer, a mod's own included. `add` refuses a target that is no
  entity type, a type whose renderer isn't a living one (a boat, a display, a projectile),
  `minecraft:player`, whose layers go to a player model, and a living type that draws through a route
  of its own which doesn't run layers yet (the armour stand, the ender dragon): the `Err` names the
  target, and the game logs it too.
- Several mods may add to one target: the layers draw in successful registration order, after the
  entity's model and its unhidden vanilla layers. A [failed registration callback](../../concepts/load-phases.md#registration-and-setup)
  adds no layers and hides no stock layers; other callbacks can still register theirs. On a type a
  mod registered a renderer for ([entity renderers](entity-renderers.md)), they draw after that
  renderer, which replaces the model and the vanilla layers. They receive `model: None` when the
  replacement renderer succeeds.
- An entity whose `render_living_pre` or `render_player_pre` a mod canceled draws none of its layers,
  as NeoForge's canceled `RenderLivingEvent.Pre` skips them.
- `submit` returns `Ok(())` to keep what it drew. A layer that returns an `Err` rolls back everything
  it drew that frame (including any `draw_stock_layer` geometry), logs a warning, and stays active
  for later draws while the other layers still run. A layer that panics rolls back what it drew that
  frame and disables the mod's layer instance for the rest of the session, with an error log and an
  in-game error toast naming the mod. Re-adding the same `Arc` on reload leaves it disabled, even on
  another target or at another position. A fresh instance does not inherit the old layer's failure.

## 3. Hiding and conditionally redrawing stock layers

To remove a vanilla stock layer from a renderer for the current resource load, call
`event.layers.hide(target, layer)` in `register_entity_layers`. For example, hiding
`StockLayer::SnowGolemHead` on `minecraft:snow_golem` stops vanilla's carved pumpkin from drawing
underneath a custom head layer:

```rust,ignore
fn register_entity_layers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityLayersEvent<'_>,
) {
    let target = LayerTarget::Entity("minecraft:snow_golem".into());
    if event.layers.stock_layers(&target).contains(&StockLayer::SnowGolemHead) {
        event.layers.hide(target, StockLayer::SnowGolemHead).unwrap();
    }
}
```

`hide` refuses, by name, any target that `add` refuses and any `StockLayer` not in
`stock_layers(&target)` (including any stock layer on a mod-replaced renderer). Hiding the same
layer twice is harmless and returns `Ok(())`. Because the renderer is shared across draws, a hidden
stock layer is omitted wherever that renderer draws it (the world and screen previews).

For per-entity visibility (the Rust form of subclassing a vanilla `RenderLayer` and calling
`super.render` conditionally), hide the stock layer on the target during `register_entity_layers`,
add a mod layer to that target, and call `stock.draw_stock_layer(layer)` inside `submit` for the
entities that should keep showing it:

```rust,ignore
struct ConditionalChestplate {
    keep_entity_id: u32,
}
impl EntityRenderLayer for ConditionalChestplate {
    fn submit(
        &self,
        entity: &EntityRenderState,
        _model: Option<&dyn FrameModel>,
        _draw: &mut dyn BlockEntityDrawContext,
        stock: &mut dyn StockLayerDraw,
    ) -> Result<(), String> {
        if entity.id == self.keep_entity_id {
            for pass in [ArmorLayerPass::Base, ArmorLayerPass::Glint, ArmorLayerPass::Trim] {
                stock.draw_stock_layer(StockLayer::Armor(ArmorLayerSlot::Chest, pass))?;
            }
        }
        Ok(())
    }
}
```

`draw_stock_layer` submits the stock layer with the entity's exact pose, lighting and draw
conditions for this frame; an empty slot draws nothing and returns `Ok(())`. It returns `Err` if
the target's renderer lacks `layer` or if `layer` was not hidden on this target (an unhidden layer
has already drawn). Within one `submit` call, the stock layer's geometry is submitted after
everything the mod layer emitted through `draw`, regardless of the call order.

## Where layers draw

The game resolves each target's layers and hidden stock layers when they are registered, so a frame
finds an entity's layers with no lookup by name and no scan over the mods; with no layer added and
no stock layer hidden, it checks one flag per entity and builds nothing. The draw retains its
existing body poses only when the target has a layer to draw or a Post listener can read them. A
layer on one type does not request captures for other types. The layers draw on the mobs (vanilla's
and the content mods' living types) and on the players, including this window's own in third person
when it holds the camera and in the inventory player preview. In the ordered body pass, layers
submit in level join order, interleaved with general entity renderer submissions. The armour stand
and the ender dragon have draws of their own that don't run layers, so `add` and `hide` refuse
them.

Use `render_living_post` and `render_player_post` to draw after the layers without registering for
one specific type (NeoForge's `RenderLivingEvent.Post` and `RenderPlayerEvent.Post`). They borrow the
same final model when the draw produced one, subject to
[Pre cancellation](model-drawing.md#before-and-after-a-living-entity-draws).
