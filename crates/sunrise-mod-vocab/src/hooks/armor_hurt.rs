//! The `armor_hurt` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Armor is about to take durability damage from a hit (NeoForge's
    /// `ArmorHurtEvent`, vanilla's `LivingEntity.doHurtEquipment`): once per
    /// hit, for the pieces that would wear, and first for the helmet alone when a
    /// falling block hits a player (`hurtHelmet`). `entity` is the wearer's entity id,
    /// `player` its UUID (None for a mob), `kind` its type and `source` the hit's
    /// damage type (`causing` and `direct` are 0: the hit does not carry them here). Each entry has
    /// its `slot`, the `stack`, the `damage` vanilla worked out and a
    /// `new_damage` a mod may change (0 for none; Unbreaking is applied
    /// after). Canceling means no armor damage. A server player's `feet`,
    /// `legs`, `chest` and `head`; a mob's `body` (a horse's armor).
    /// Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "ArmorHurtEvent",
        cancellable = true
    )]
    fn armor_hurt(
        player: Option<u128>,
        entity: u32,
        kind: &str,
        source: &HurtSource<'_>,
        entries: &mut Vec<mod_api::ArmorHurt>,
        cancel: &mut bool,
    );
}
