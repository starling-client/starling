//! The `entity_damage` area: invulnerability, damage cooldowns and hurting an entity. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_damage` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityDamageBackend {
    /// Side: server.
    /// `Entity.isPermanentlyInvulnerable`: the `Invulnerable` flag, which
    /// only damage that bypasses invulnerability (the void, `/kill`) or a
    /// creative player gets past. Every kind of entity keeps it, saved as
    /// `Invulnerable`; a player's is not saved (Sunrise's player save keeps no such
    /// field).
    fn entity_permanently_invulnerable(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// `Entity.invulnerableTime`: the entity-level invulnerability
    /// window, its own field on a mob and a player — never the living damage
    /// cooldown — and the saved `invulnerable_time` tag for the other kinds.
    /// Counted down by the base tick; `isTemporarilyInvulnerable` is it above
    /// zero. None for a stale handle or a kind that keeps none. On a client, the
    /// local player's own value or a kept entity's (a tracked player or mob),
    /// which the server doesn't send: the client's own copy. None for the other
    /// kinds and for an entity whose body the client doesn't keep.
    fn entity_invulnerable_time(&self, entity: EntityRef) -> Option<i32>;
    /// Side: both.
    /// `LivingEntity.damageCooldownTime`: 20 after a hit, counting down;
    /// while it is above 10 a hit does only what it deals beyond the last
    /// one. Mobs and players. On a client, the local player's own value
    /// (`LocalPlayer.hurtTo`: 20 when its health drops, 10 when it rises, what
    /// `Hud.extractPlayerHealth` blinks the hearts by) or a kept entity's (a
    /// tracked player or mob: 20 on its damage event), counted down by the
    /// client's tick. None for the other kinds and for an entity whose body the
    /// client doesn't keep.
    fn entity_damage_cooldown_time(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// `Entity.isInvulnerableTo` for a source of `damage_type`: the entity's own
    /// decision, the guard every mob damage hook runs after — dead or
    /// removed, fire immunity, or the `Invulnerable` flag nothing but
    /// `#bypasses_invulnerability` gets past. None for a stale handle or a
    /// kind with no such decision (the player's paths answer their refusals
    /// at hurt time).
    fn entity_invulnerable_to_damage(&self, entity: EntityRef, damage_type: &str) -> Option<bool>;
    /// Side: server.
    /// Whether `source`'s damage may hit `entity` through their teams
    /// (`Player.canHarmPlayer`, `Team.allowFriendlyFire`,
    /// `DamageSource.doTeamsAllowDamage`): allies without friendly fire
    /// cannot. Entities without a team always can. None for a stale handle.
    ///
    fn entity_entities_team_damage_allowed(
        &self,
        entity: EntityRef,
        source: EntityRef,
    ) -> Option<bool>;
    /// Side: server.
    /// `Entity.isInvulnerableToPiercingWeapon` (`isInvulnerable`): the
    /// `Invulnerable` flag; the temporary window `Entity.invulnerableTime`
    /// opens is not represented, so no hit answers through it. None
    /// for a stale handle or a kind without the flag.
    fn entity_invulnerable_to_piercing_weapon(&self, entity: EntityRef) -> Option<bool>;
}
/// The handles' writes of the `entity_damage` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityDamageBackendMut {
    /// Side: server.
    /// Roll this live entity's death loot table as `LivingEntity.dropFromLootTable`, but
    /// return the stacks without killing the entity, dropping items or experience, or
    /// taking its equipment. Only the entity's loot table is rolled, not its custom death
    /// drops. `None` uses its own table, including a mob's `DeathLootTable` override.
    ///
    /// The `ENTITY` context uses the entity's position as ORIGIN and `source`'s causing
    /// and direct entities as ATTACKING_ENTITY and DIRECT_ATTACKING_ENTITY. The credited
    /// player, if supplied, sets LAST_DAMAGE_PLAYER and luck; it need not be the attacker.
    /// There is no TOOL. The living target must be in the server level in place; stale
    /// references, unknown IDs and non-living targets are refused before drawing.
    ///
    /// This uses a mob's nonzero death-table seed if set, otherwise the named random
    /// sequence or the level random. The latter draws advance the live streams.
    ///
    /// The roll asks `enchanted_entity_loot` for its enchantment levels, as a death does
    /// (NeoForge's `EventHooks.getEntityLootEnchantmentLevel` fires for any entity loot
    /// with a damage source). A listener of that hook that rolls from inside it rolls
    /// again inside itself, so it must stop its own recursion.
    fn entity_roll_death_loot(
        &mut self,
        entity: EntityRef,
        table: Option<&Identifier>,
        source: LootDamageSource<'_>,
        last_damage_player: Option<PlayerRef>,
    ) -> Result<Vec<ItemStack>, DeathLootError>;
    /// Side: server.
    /// `Entity.setInvulnerable`: sets the `Invulnerable` flag, for every kind of entity.
    /// False for a stale handle.
    fn entity_set_invulnerable(&mut self, entity: EntityRef, invulnerable: bool) -> bool;
    /// Side: server.
    /// Sets `LivingEntity.damageCooldownTime`, as mods do to let rapid hits
    /// through (0). Mobs and players.
    fn entity_set_damage_cooldown_time(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// `Entity.setInvulnerableTime`: the write side of
    /// `entity_invulnerable_time`. False where the entity is gone or keeps
    /// no such field.
    fn entity_set_invulnerable_time(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// Hurts the entity for `amount` through `Entity.hurtServer`. Pass the
    /// exact registered `damage_type` id, vanilla or modded: the hurt source
    /// keeps that id regardless of the entities supplied (for example,
    /// `minecraft:explosion` does not become `minecraft:player_explosion`).
    /// `direct` dealt the damage (such as an arrow); `causing` is responsible
    /// (such as its shooter). The normal armor, cooldown, hooks and damage
    /// broadcast still apply. True when the damage was accepted, false if
    /// refused (including an unregistered type), and None for a stale target.
    /// Accepted does not mean health was lost: `hurtServer` can report success
    /// without any (a heart-bound creaking's reaction, a ghast's reflected
    /// large fireball on hurt cooldown).
    /// Stale direct or causing handles are treated as absent.
    /// A general entity refuses (`Some(false)`) until it has a hurt Part (#5084), and so
    /// does every kind that has no hurt path yet.
    fn entity_hurt(
        &mut self,
        entity: EntityRef,
        damage_type: &str,
        amount: f32,
        direct: Option<EntityRef>,
        causing: Option<EntityRef>,
    ) -> Option<bool>;
}
