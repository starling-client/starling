//! The `living_death` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob or server player is about to die (NeoForge's `LivingDeathEvent`,
    /// vanilla's `LivingEntity.die`): its health has run out, and the loot,
    /// experience and death message have not run. `entity` is its id and
    /// `kind` its type (`minecraft:zombie`, `minecraft:player`); `damage_type`
    /// names the last blow (`minecraft:player_attack`) and `attacker` is the
    /// mob that dealt it, 0 for none or a player. Set `cancel` to keep it alive:
    /// Horizon decides death by health, so it stays at 1 health. The hook
    /// fires once per death attempt. Armor stands, boats and other non-mob
    /// entities are not told.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingDeathEvent",
        cancellable = true
    )]
    fn living_death();
}
