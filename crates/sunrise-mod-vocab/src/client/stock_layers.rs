//! Stock layers of living entity renderers (`StockLayer`) and the callback surface for drawing a
//! hidden stock layer from a mod's [`crate::EntityRenderLayer`] (`StockLayerDraw`).

use crate::Hand;

/// One of the four armour slots drawn by vanilla's `HumanoidArmorLayer`, distinct from the item's
/// equipment slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: these are the four armour slots drawn by HumanoidArmorLayer.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ArmorLayerSlot {
    /// The helmet layer.
    Head,
    /// The chestplate layer.
    Chest,
    /// The leggings layer.
    Legs,
    /// The boots layer.
    Feet,
}

/// One of the independent passes of a worn humanoid armour piece (`HumanoidArmorLayer` /
/// `EquipmentLayerRenderer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: base material, enchantment glint and armour trim are the stock armour passes.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ArmorLayerPass {
    /// The armour material, including its base texture layers.
    Base,
    /// The enchantment foil, whether carried by the base material or by the trim.
    Glint,
    /// The armour-trim material.
    Trim,
}

/// A stock layer built into a living entity renderer. Mod-added layers are not stock layers.
///
/// Each variant names the vanilla 26.3 `RenderLayer` class whose step it corresponds to.
/// [`crate::EntityLayers::stock_layers`] lists the variants a target's renderer supports,
/// [`crate::EntityLayers::hide`] removes one from that renderer for the current resource load,
/// and [`StockLayerDraw::draw_stock_layer`] draws a hidden stock layer conditionally from a mod's
/// own [`crate::EntityRenderLayer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: these are the stock layers of vanilla 26.3's living-entity renderers; a mod's own
// layers are never stock layers.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StockLayer {
    /// One pass of a worn humanoid armour piece (`HumanoidArmorLayer` / `EquipmentLayerRenderer`).
    Armor(ArmorLayerSlot, ArmorLayerPass),
    /// Elytra wings worn on the back (`WingsLayer`).
    Wings,
    /// A player's cape (`CapeLayer`).
    Cape,
    /// A non-armour item worn on the head, such as a skull, pumpkin or raid captain banner
    /// (`CustomHeadLayer`).
    CustomHead,
    /// An item held in a hand (`ItemInHandLayer` / `PlayerItemInHandLayer`).
    HeldItem(Hand),
    /// The carved pumpkin worn on a snow golem's head (`SnowGolemHeadLayer`).
    SnowGolemHead,
    /// Parrots perched on a player's shoulders (`ParrotOnShoulderLayer`).
    ParrotOnShoulder,
    /// Arrows stuck in a player's body (`ArrowLayer`).
    Arrow,
    /// Bee stingers stuck in a player's body (`BeeStingerLayer`).
    BeeStinger,
    /// The riptide spin-attack swirl around a player (`SpinAttackEffectLayer`).
    SpinAttackEffect,
    /// The extra ears drawn for the `deadmau5` player name (`Deadmau5EarsLayer`).
    Deadmau5Ears,
}

/// Draws a hidden stock layer for the entity currently submitted to
/// [`crate::EntityRenderLayer::submit`].
///
/// This is the Rust counterpart of subclassing a vanilla `RenderLayer` and calling `super.render`
/// per entity: hide the stock layer on the target in `register_entity_layers`, then call
/// [`Self::draw_stock_layer`] from your own layer for the entities that should still show it.
pub trait StockLayerDraw {
    /// Submits `layer` for the entity in hand with the same pose, lighting and visibility
    /// conditions the stock draw would have used this frame.
    ///
    /// An empty equipment slot or unmet draw condition draws nothing and returns `Ok(())`.
    /// Returns `Err` if the target's renderer lacks `layer` (including when a mod replaced the
    /// renderer) or if `layer` was not hidden on this target in `register_entity_layers`, since an
    /// unhidden layer has already drawn. Within a single `submit` call, geometry from
    /// `draw_stock_layer` is submitted after everything the mod layer emitted through `draw`,
    /// regardless of the order in which `draw` and `stock.draw_stock_layer` were called. Anything
    /// drawn by this call rolls back with the rest of `submit` if the layer returns `Err` or
    /// panics.
    fn draw_stock_layer(&mut self, layer: StockLayer) -> Result<(), String>;
}
