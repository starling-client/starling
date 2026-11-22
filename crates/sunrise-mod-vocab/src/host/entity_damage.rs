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
    /// creative player gets past. Mobs keep it on themselves (saved with
    /// them), players on their body (not saved: Sunrise's player save
    /// keeps no such field), the other kinds in the saved `Invulnerable`
    /// tag of their entity data — the tag the item damage path
    /// already refuses by.
    fn entity_permanently_invulnerable(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// `Entity.invulnerableTime`: the entity-level invulnerability
    /// window, its own field on a mob and a player — never the living damage
    /// cooldown — and the saved `invulnerable_time` tag for the other kinds.
    /// Counted down by the base tick; `isTemporarilyInvulnerable` is it above
    /// zero. None for a stale handle or a kind that keeps none.
    fn entity_invulnerable_time(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// `LivingEntity.damageCooldownTime`: 20 after a hit, counting down;
    /// while it is above 10 a hit does only what it deals beyond the last
    /// one. Mobs and players.
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
    /// `Entity.setInvulnerable`: sets a mob's `Invulnerable` flag, saved
    /// with it. False for the other kinds.
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
    fn entity_hurt(
        &mut self,
        entity: EntityRef,
        damage_type: &str,
        amount: f32,
        direct: Option<EntityRef>,
        causing: Option<EntityRef>,
    ) -> Option<bool>;
}
