//! The `register_entity_layers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The layers a mod inspects, hides or adds on living entity renderers (NeoForge's
    /// `EntityRenderersEvent.AddLayers`): `add(target, layer)`, `stock_layers(&target)` and
    /// `hide(target, stock_layer)`. Every living entity of the type, or every player of the player
    /// model, in view draws `EntityRenderLayer::submit` after its model, its unhidden stock layers
    /// and the mod layers added before, with `StockLayerDraw` to redraw a hidden stock layer per
    /// entity. The final body model is borrowed for `submit`; it is `None` when no body was posed,
    /// as after a successful replacement renderer. `entity_types`, `is_living`, `player_models`,
    /// `has_mod_renderer` and `stock_layers` tell what renderers this load built. A target that is
    /// no living entity type, or a stock layer a renderer lacks, is refused by name. Asked on the
    /// client on every resource load, the first one and each reload, after the renderers resolve
    /// (`register_entity_renderers`): added layers and hidden stock layers don't carry over from
    /// one load to the next, so register them again each time. A mod's layer that panics in
    /// `submit` stays disabled when re-added with the same `Arc`. In the ordered body pass,
    /// layers submit in level join order, interleaved with general entity renderer submissions.
    /// If this registration listener panics or is phase-refused, none of its added layers or
    /// stock-layer hides take effect; later listeners still register in the same order.
    #[notes = r#"## Entity layers

An entity layer (NeoForge's `EntityRenderersEvent.AddLayers` and `RenderLayer`) draws over a living entity
renderer or replaces one of its stock layers: armour of a mod's own, a glow, something held or worn, or a
custom head block in place of a snow golem's pumpkin. `Hooks::register_entity_layers` is asked on every
resource load, after the entity renderers resolve, since a reload builds the renderers again and neither
added layers nor hidden stock layers carry over; the game resolves each target's layers and hides to the
slot its draw reads, so a frame hands an entity to its target's layers and hides only. In the ordered body
pass the layers submit in level join order, including the local player at its own slot when its body is
submitted, interleaved with submissions from general entity renderers.

```rust,ignore
struct Halo;
impl EntityRenderLayer for Halo {
    fn submit(
        &self,
        _entity: &EntityRenderState,
        model: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
        _stock: &mut dyn StockLayerDraw,
    ) -> Result<(), String> {
        // Skip the halo when the body has a head and that part or an ancestor was hidden.
        // A replacement renderer or a body without this part still gets a halo.
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

The event tells what renderers this load built: `entity_types()` borrows the sorted slice of every entity
type with a renderer, `is_living(type)` says whether it is a living entity's, `player_models()` borrows
the `[Wide, Slim]` slice, `has_mod_renderer(type)` says whether a mod registered the type's renderer, and
`stock_layers(&target)` borrows the slice of `StockLayer` variants that target's renderer draws (empty for
a mod renderer or a target that takes no layers). No renderer object crosses the API.

`hide(target, stock_layer)` removes one stock layer from `target`'s renderer for this resource load
(mirroring removing a vanilla layer from a renderer's `layers` list in `AddLayers`): for example, hiding
`StockLayer::SnowGolemHead` on `minecraft:snow_golem` stops vanilla's carved pumpkin from drawing beneath a
mod's replacement head layer, and hiding `StockLayer::Armor(slot, pass)` on `PlayerModel::Wide` and
`PlayerModel::Slim` omits worn player armour everywhere that renderer draws (the world and screen previews).
`hide` refuses, by name, a target that can't take layers or a layer not in `stock_layers(&target)`. Hiding
the same layer twice is harmless.

For per-entity decisions (such as hiding armour only on selected players), hide the stock layer on the
target in `register_entity_layers`, add a mod layer to that target, and call
`stock.draw_stock_layer(layer)` inside `EntityRenderLayer::submit` for the entities that should still show
it. `draw_stock_layer` submits the stock layer with the entity's exact pose, lighting and visibility
conditions for that frame (an empty slot draws nothing and returns `Ok(())`). It returns `Err` for a layer
the renderer lacks or one that was not hidden on this target. Anything it drew rolls back with the rest of
`submit` on an `Err` return or a panic.

A layer attaches only to a living renderer, a mod's own included: `add` refuses a target that is no entity
type, a type whose renderer isn't a living one (a boat, a display, a projectile), `minecraft:player`,
whose layers go to a player model, and a living type that draws through a route of its own which doesn't
run layers yet (the armour stand, the ender dragon); the `Err` names the target, and the game logs it too. An
entity whose `render_living_pre` or `render_player_pre` a mod canceled draws none of its layers, as
NeoForge's canceled `RenderLivingEvent.Pre` skips them. Several mods may add to one target: their layers
draw in successful registration order. A panic or phase refusal discards only that listener's added
layers and stock-layer hides. Layers draw after the entity's model and its unhidden vanilla layers, each
from the entity's frame (at its feet, turned with its body, as `render_living_pre` draws). `entity.yaw` is
the body's turn and `entity.pitch` the head's. `model` lends the final body hierarchy for the callback,
including for nonhumanoids, or `None` if no body pose was produced (such as after a successful replacement
renderer). Read a named part or follow the hierarchy and compose its parent transforms to attach
geometry. A layer that returns an `Err` rolls back what it drew that frame, logs a warning, and stays
active for later draws while the other layers still run. A layer that panics rolls back what it drew that
frame and disables the mod's layer instance for the rest of the session, with an error log and an
in-game error toast naming the mod. Re-adding the same `Arc` on reload leaves it disabled regardless of
its target or position. A new instance starts enabled. The layers draw on the mobs (vanilla's and the
content mods' living types) and the players; the armour stand and the ender dragon take them once
their own draws run layers, in a follow-up of #860, which lifts their refusal."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        staged_surface = "layers",
        version = 6,
        context = "registration",
        neoforge = "EntityRenderersEvent.AddLayers"
    )]
    fn register_entity_layers(layers: &mut dyn EntityLayers);
}
