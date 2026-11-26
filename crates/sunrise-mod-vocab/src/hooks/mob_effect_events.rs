//! The `mob_effect_events` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `MobEffectEvent.Applicable`, vanilla's `LivingEntity.canBeAffected`: asked wherever the game checks whether the resolved `EffectKey` `effect` (`minecraft:poison`) can land on the living entity `entity` (a [`LivingRef`], the player's own included), before `addEffect` accepts it. Unlike the effect hooks above this fires for every effect, vanilla's included, and only when an add is in flight. The first mod to answer decides: `Allow` or `Deny` overrides the vanilla tag check; an unanswered hook falls back to it. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "applicable",
        neoforge = "MobEffectEvent.Applicable",
        cancellable = false
    )]
    fn mob_effect_applicable(
        entity: LivingRef,
        effect: EffectKey,
        applicable: &mut Option<mod_api::EffectApplicable>,
    );
    /// NeoForge's `MobEffectEvent.Added`, vanilla's `LivingEntity.addEffect`: fired for every
    /// eligible effect (vanilla and mod) after `canBeAffected` / `mob_effect_applicable` accepts
    /// it and before `entity`'s active effects are mutated. `old_effect` is the existing active
    /// instance of the same effect, if any; `new_effect` is the incoming instance being applied;
    /// `source` is the entity that caused the effect, when known. Fires on first insertion,
    /// reapplications and unchanged updates alike; does not fire on `forceAddEffect`, world load
    /// or client sync, and is not cancellable. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "MobEffectEvent.Added",
        cancellable = false
    )]
    fn mob_effect_added(
        entity: LivingRef,
        old_effect: Option<&ActiveEffect>,
        new_effect: &ActiveEffect,
        source: Option<EntityRef>,
    );
    /// NeoForge's `MobEffectEvent.Expired`, vanilla `LivingEntity.tickEffects`: the resolved `EffectKey` `effect` (`minecraft:speed`, amplifier `amplifier`) on the living entity `entity` (a [`LivingRef`], the player's own included) counted down to nothing and is about to leave. Cancel and the effect stays, kept at no ticks and asked again next tick; unanswered, the expiry runs. Fires only on the tick an effect actually expires, never per tick. Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "MobEffectEvent.Expired",
        paper = "EntityPotionEffectEvent",
        cancellable = true
    )]
    fn mob_effect_expired(entity: LivingRef, effect: EffectKey, amplifier: i32, cancel: &mut bool);
    /// NeoForge's `MobEffectEvent.Remove`, vanilla `LivingEntity.removeEffect`: the resolved `EffectKey` `effect` (`minecraft:speed`, amplifier `amplifier`) on the living entity `entity` (a [`LivingRef`], the player's own included) is about to be removed by any path that walks the vanilla funnel: a command or milk for the player, a cure for a zombie villager, a mod's removal helper. Cancel and the effect stays. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "MobEffectEvent.Remove",
        paper = "EntityPotionEffectEvent",
        cancellable = true
    )]
    fn mob_effect_remove(entity: LivingRef, effect: EffectKey, amplifier: i32, cancel: &mut bool);
}
