//! The `living_equipment_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0157

pub trait Hooks {
    /// A mob's or an armor stand's equipment slot took a different stack
    /// (NeoForge's `LivingEquipmentChangeEvent`, posted in
    /// `LivingEntity.collectEquipmentChanges`): `entity` is its ID and `kind`
    /// its type (`minecraft:zombie`), `slot` the `EquipmentSlot` (`mainhand`,
    /// `offhand`, `feet`, `legs`, `chest`, `head`, `body`, `saddle`), `from`
    /// what the slot held when this was last heard (none for empty) and `to`
    /// what it holds now. Once for each slot that changed, in that order,
    /// after the mobs' tick, so a mob that joins the level wearing something
    /// counts from empty on its first tick. It only hears: change the mob with
    /// its own helpers. Players are heard by `native_on_equip`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LivingEquipmentChangeEvent",
        cancellable = false
    )]
    fn living_equipment_change();
}
