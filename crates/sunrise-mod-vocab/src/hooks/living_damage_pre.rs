//! The `living_damage_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingDamageEvent.Pre`: `LivingEntity.actuallyHurt` of entity `entity` of
    /// type `kind` (`minecraft:player`, `minecraft:zombie`) by `source` (named as
    /// `native_living_incoming_damage` names it) has taken armor and magic, and the absorption
    /// hearts are next: `container` is a live mutable borrow of the hit's damage container;
    /// it holds the damage it started with, what it is now (`new_damage`, which may be 0) and
    /// each reduction so far (armor, enchantments, mob effects). Change its
    /// `new_damage` to change what is inflicted before absorption, not what `lastHurt` remembers
    /// for the cooldown. Set its post-attack invulnerability ticks to change the cooldown itself,
    /// or use `add_reduction_modifier` for subsequent stages (such as `Reduction::Absorption`).
    /// The setters refuse negative damage, NaN damage and negative ticks without changing the hit.
    /// Pre fires for players, mobs and the ender dragon; a wolf whose armor takes the hit and
    /// armor stands do not fire it. The host is the hit's (`HitHost`): its blocks, random and own
    /// attachments, level sounds, and the own random of the entities the hit lends; the hurt entity
    /// is in the middle of its hit.
    #[notes = r#"## Combat hook ownership

The six combat hooks (`living_incoming_damage`, `damage_pre`, `damage_post`, `living_knockback`, `living_use_totem`, `living_shield_block`) are lent to each hit by the game that runs it, not installed once per process. A mod sees no difference: order, arguments and cancel rules are unchanged.

**Migration (`living_use_totem`, `living_shield_block`, now version 2):** both now take the hit's level host as their first argument, as `living_incoming_damage` does: its blocks, random and own attachments, with no writes beyond them. Add the parameter (`level: &mut dyn LevelRandomHost`); the event, its fields, its order and its cancel rules are unchanged. `living_shield_block` fires inside `hurtServer` before the armor and enchantment reductions and sees the hit's `damage` as it came in; `living_use_totem` fires after health reached 0 and before `die`. Tick-time effect: one extra pointer in the hit and the same `listened` early-outs. Diverges from upstream: the two hooks gain the level host that upstream's catalog rows did not have.

**Attacker and held stacks (`living_knockback`, the weapon of `living_incoming_damage`):** the attacker's id and the stacks it holds come from the hit that started the attack, not from process state, so a hit nested inside another (a thorns reply, a guest's attack during the host's) names its own attacker and nothing is left behind afterwards. No event field, order, random draw or cancel rule changes, so neither hook's version does. The game copies the held stacks only while a mod listens; without one the hit carries the attacker id and two empty slots, no allocation.

**Sounds and an entity's random (`living_incoming_damage`, `living_damage_pre`, `living_damage_post`, `living_shield_block`, `living_use_totem`, now one version up):** these five take the hit's host as `&mut dyn HitHost` (a `LevelRandomHost` that also plays level sounds and draws from an entity's own random), so a listener can play a hit sound with a pitch drawn from the attacker. `host.play_sound_at(sound, at, source, volume, pitch)` is `Level.playSound(null, x, y, z, ...)` and `host.play_sound_from(entity, sound, source, volume, pitch)` is `Level.playSound(null, entity, ...)`: every player in range hears it, the entity's own player too, and a cancelled hit still plays what was queued. The sound's seed is drawn at the call from the level's sound-seed stream, never from the level random. `entity.next_float(host)`, `next_double`, `next_bool` and `next_int(host, bound)` draw from the entity's actual random stream, in listener order, and the stream keeps the draws. A hit lends its attacker where the game applies a melee hit (the attacking player or mob, for player → mob, mob → mob and mob → player) and no entity at any other damage site (environment, projectiles, explosions, potions and the like); the attacking mob's own random is the one its AI draws from, and a hit built inside a mob's own tick lends none. Any other entity, a stale handle, a client and an out-of-range `bound` answer `None` (`false` for a sound). **Migration:** change the host parameter's type from `&mut dyn LevelRandomHost` to `&mut dyn HitHost`; `with_random`, attachments and the event are unchanged. `entity_invulnerability_check` is asked on both sides and as a query, so its host stays read-only and it gets none of this. Tick-time effect: one pointer in the hit; with no listener nothing is looked up, drawn or allocated."#]
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        host = "hit",
        neoforge = "LivingDamageEvent.Pre",
        version = 3,
        target = "sunrise_client::game::native_hooks::living_damage_pre::HitLevel<'_>::native_living_damage_pre"
    )]
    fn living_damage_pre(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        container: mod_api::DamageContainer<'_>,
    );
}
