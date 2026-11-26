//! Layers on living entity renderers, as a client mod inspects, hides or adds them in
//! `register_entity_layers` (NeoForge's `EntityRenderersEvent.AddLayers` and `RenderLayer`).
//!
//! A layer draws over a living entity of one type, or over every player of one player model, after
//! its model and the unhidden stock layers and mod layers before it. The game asks for the layers
//! and stock-layer hides on every resource load, as the renderers are built again, and resolves
//! each target to the slot its draw reads, so a frame hands an entity to its own type's layers and
//! hides and to no other mod.
use crate::{BlockEntityDrawContext, EntityRenderState, FrameModel, StockLayer, StockLayerDraw};
use std::sync::Arc;

/// A player model, as a player's skin chooses it: each has a renderer of its own, and a layer added
/// to one draws on every player using it (NeoForge's `PlayerModelType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PlayerModel {
    /// Steve's arms, four pixels wide.
    Wide,
    /// Alex's arms, three pixels wide.
    Slim,
}

/// What a layer is added to.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LayerTarget {
    /// Every entity of a living entity type (`minecraft:zombie`, `mymod:golem`).
    Entity(String),
    /// Every player of a player model.
    Player(PlayerModel),
}

/// A mod's layer: NeoForge's `RenderLayer`, held by the game for every entity of its target.
/// Entities of a target share one layer, so what differs between them comes from
/// [`EntityRenderState`].
///
/// A frame has no `Host`: drawing reads nothing of the world but what the state, the model and
/// `draw` give. A layer that needs more reads it in a hook and keeps what it read.
pub trait EntityRenderLayer: Send + Sync + 'static {
    /// `RenderLayer.submit`: draw over `entity` with `draw`'s helpers (`quad`, `cube`, `block`,
    /// `item`, `model_part`), after its model, its unhidden stock layers, and the mod layers
    /// before this one. The pose stack starts in the entity's frame: at its feet, turned with its
    /// body, +z the way it faces, +x its left, +y up, in blocks (`entity.yaw` is the body's turn,
    /// `entity.pitch` the head's). `model` borrows the final posed body hierarchy for this
    /// callback, as in the Post hooks. Follow its parent links when attaching geometry; a layer
    /// drawing on the body can honor each part's effective visibility, while an independent layer
    /// need not. `None` means the draw produced no body pose (for example, a successful
    /// replacement renderer), not that the body is nonhumanoid. `draw` also has the partial tick,
    /// the game time and the entity's light. `stock` lets the layer draw a stock layer that was
    /// hidden on this target in `register_entity_layers` (`StockLayerDraw::draw_stock_layer`),
    /// conditionally for the entity in hand. Within one `submit` call, any geometry produced by
    /// `stock.draw_stock_layer` is submitted after everything the mod layer emitted through
    /// `draw`, regardless of the order in which `draw` and `stock.draw_stock_layer` were called.
    /// In the world, called once a frame for each entity of the target in view, but not when its
    /// `render_living_pre` or `render_player_pre` was canceled. A frame is drawn afresh:
    /// `draw.keep()` does nothing here. `Ok(())` keeps what `draw` and `stock` emitted. On an
    /// `Err`, everything this layer drew this frame (including any `stock.draw_stock_layer`
    /// geometry) is rolled back, the error is logged as a warning, and the layer stays active for
    /// later draws. On a panic, everything this layer drew this frame is rolled back and this
    /// mod's layer instance is disabled for the rest of the session, with an error log and an
    /// in-game error toast naming the mod. Re-adding the same `Arc` on reload does not re-enable
    /// it.
    fn submit(
        &self,
        entity: &EntityRenderState,
        model: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
        stock: &mut dyn StockLayerDraw,
    ) -> Result<(), String>;
}

/// The layer registry `register_entity_layers` hands out: NeoForge's
/// `EntityRenderersEvent.AddLayers`, with what it tells of the renderers this load built.
pub trait EntityLayers {
    /// `getEntityTypes`: every entity type that has a renderer this load, vanilla's and the mods'
    /// (`minecraft:oak_boat`, `mymod:bolt`), sorted. A player has none of its own: its renderers
    /// are the player models'.
    fn entity_types(&self) -> &[String];
    /// Whether `entity_type`'s renderer is a living entity's, the only kind that takes layers.
    /// False for a type with no renderer.
    fn is_living(&self, entity_type: &str) -> bool;
    /// `getSkins`: the player models, each with a renderer of its own.
    fn player_models(&self) -> &[PlayerModel];
    /// Whether a mod registered `entity_type`'s renderer (`register_entity_renderers`), rather than
    /// it being the game's own.
    fn has_mod_renderer(&self, entity_type: &str) -> bool;
    /// The stock layers that `target`'s renderer draws (mirroring `AddLayers.getRenderer` and its
    /// `layers` list). Empty for a mod-replaced renderer or a target that takes no layers. Mod
    /// layers are not listed here.
    fn stock_layers(&self, target: &LayerTarget) -> &[StockLayer];
    /// Removes `layer` from `target`'s renderer for this resource load (mirroring removing a
    /// vanilla layer from a renderer's `layers` list in `AddLayers`).
    ///
    /// A hidden stock layer is omitted from every draw through that renderer (the world and screen
    /// previews). A mod layer on the same target may still draw it for individual entities via
    /// [`StockLayerDraw::draw_stock_layer`]. Refuses, by name, the same targets [`Self::add`]
    /// refuses and any layer not in [`Self::stock_layers`] for `target`. Hiding the same layer
    /// more than once is harmless and returns `Ok(())`. Hides do not carry over to the next
    /// resource load, and a listener that panics or is phase-refused discards its hides along with
    /// its added layers.
    fn hide(&mut self, target: LayerTarget, layer: StockLayer) -> Result<(), String>;
    /// `addLayer`: `layer` draws over every entity of `target`, after its model, its unhidden
    /// stock layers and the mod layers added to it before, whichever mod added them. Panic
    /// disablement follows the mod's `Arc` allocation, not its target or order. A new layer
    /// instance starts enabled. A target that can't take layers is refused: not an entity type, a
    /// type whose renderer isn't a living one, `minecraft:player` (whose layers go to a player
    /// model), and a living type that draws through a route of its own that doesn't run layers yet
    /// (the armour stand, the ender dragon). The `Err` names it, and the game logs it too.
    fn add(&mut self, target: LayerTarget, layer: Arc<dyn EntityRenderLayer>)
    -> Result<(), String>;
}
