//! The `living_death_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob or server player's death has finished (Fabric's
    /// `ServerLivingEntityEvents.AFTER_DEATH`, vanilla's `LivingEntity.die`
    /// at its `broadcastEntityEvent`): the loot, equipment and experience
    /// dropped (the ender dragon emits its experience orbs later during its
    /// death tick) and the death message went out. `entity` and `kind` identify the
    /// victim. `player` is the victim's UUID for a player death, and `message_key`
    /// is present when the player death recorded a known translation key.
    /// `damage_type` names the killing blow and `attacker` is the recorded
    /// numeric attacker (0 when absent). `credited_player` is the UUID of the
    /// player credited for a mob kill; it may differ from the source entity. It only
    /// hears. The tail of `living_death`, a separate moment, and not fired
    /// when that hook cancelled the death. On a server player's lethal landing this is
    /// still inside the hurt, before `HIT_GROUND` and the fall reset. On a pufferfish sting it
    /// is before that sting's poison and the next sting. Projectile and mounted-fall deaths
    /// likewise finish before their on-hit or next-rider follow-ups. Server-side, once per death, for every
    /// ordinary living entity, and for the ender dragon when its killing blow lands (it then
    /// starts its dying flight). Armor stands, boats and other non-mob entities
    /// are not told.
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        version = 2,
        fabric = "ServerLivingEntityEvents.AFTER_DEATH",
        neoforge = "none (LivingDeathEvent fires before the death)"
    )]
    fn living_death_post(
        entity: u32,
        kind: &str,
        damage_type: &str,
        attacker: u32,
        player: Option<u128>,
        credited_player: Option<u128>,
        message_key: Option<&str>,
    );
}
