//! The `living_death` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob or server player is about to die (NeoForge's `LivingDeathEvent`,
    /// vanilla's `LivingEntity.die`): its health has run out, and the loot,
    /// experience and death message have not run. `entity` is its id and
    /// `kind` its type (`minecraft:zombie`, `minecraft:player`). `source` and
    /// `weapon` are the complete killing blow, as `living_incoming_damage` saw
    /// it, including the causing player, direct projectile and weapon components.
    /// A server player's lethal fall is heard on the landing body before its fall distance is reset.
    /// A lethal pufferfish sting is heard before the fish applies that sting's poison or stings
    /// the next mob. A projectile's killing blow is heard before on-hit follow-ups;
    /// a mount or passenger's killing blow is heard before falling on to the next rider.
    /// Cancellation skips death and loot without changing the listener's health:
    /// heal it to keep it alive; at health <= 0 its body is removed after 20 ticks.
    /// Armor stands, boats and other non-mob entities are not told.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingDeathEvent",
        cancellable = true,
        version = 2
    )]
    fn living_death(
        entity: u32,
        kind: &str,
        source: &HurtSource<'_>,
        weapon: Option<&mod_api::ItemStack>,
        cancel: &mut bool,
    );
}
