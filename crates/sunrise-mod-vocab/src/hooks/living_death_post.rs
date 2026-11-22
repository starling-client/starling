//! The `living_death_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob or server player's death has finished (Fabric's
    /// `ServerLivingEntityEvents.AFTER_DEATH`, vanilla's `LivingEntity.die`
    /// at its `broadcastEntityEvent`): the loot, equipment and experience
    /// dropped and the death message went out. `entity` is its id;
    /// `damage_type` names the killing blow (`minecraft:player_attack`) and
    /// `attacker` is the mob that dealt it, 0 for none or a player. It only
    /// hears. The tail of `living_death`, a separate moment, and not fired
    /// when that hook cancelled the death. On a server player's lethal landing this is
    /// still inside the hurt, before `HIT_GROUND` and the fall reset. On a pufferfish sting it
    /// is before that sting's poison and the next sting. Projectile and mounted-fall deaths
    /// likewise finish before their on-hit or next-rider follow-ups. Server-side, once per death, for every
    /// ordinary living entity. The ender dragon uses a separate death
    /// sequence and fires neither living death hook. Armor stands, boats and other non-mob entities
    /// are not told.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        fabric = "ServerLivingEntityEvents.AFTER_DEATH",
        neoforge = "none (LivingDeathEvent fires before the death)"
    )]
    fn living_death_post(entity: u32, damage_type: &str, attacker: u32);
}
