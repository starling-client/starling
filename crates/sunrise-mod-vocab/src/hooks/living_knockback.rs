//! The `living_knockback` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingKnockBackEvent`: `LivingEntity.knockback` is about to push entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) `strength` away from the
    /// direction (`x`, `z`), on the raw strength, before the entity's knockback resistance takes
    /// its share and its motion changes. Vanilla's direction points at the source of a hit (the
    /// attacker's side), and the entity is pushed the other way. `source` is the push's
    /// `DamageSource`, named as `native_living_incoming_damage` names a hit's. `attacker` is the
    /// attacking entity's network id, falling back to the victim's last hurt-by mob, or 0 when
    /// unknown; `attacker_main` and `attacker_off` are what it holds (a player's selected and
    /// off-hand stacks, a mob's main hand), none when unknown. `entry` is `Short` when the push
    /// came in through `knockback(power, x, z)`, which only delegates to the full entry, and `Full`
    /// when the full entry was called directly. The hook fires once per call of the full entry
    /// either way, so a listener that wants its change applied twice for a short-entry push, as
    /// a mixin on both entries would, applies it twice when `entry` is `Short`. Change
    /// `strength`, `x` or `z`, or
    /// set `canceled`, and the push follows them: canceled, the entity's motion is left alone. A
    /// `strength` of 0 or less after the hook is no push, and a direction too short to tell
    /// draws a random one after it, as in vanilla. A value a mod leaves that is not a finite
    /// number is put back as it was. It fires for the players, mobs and armor stands the game
    /// knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a
    /// shield's push on its holder, a goat's ram and a mob's melee push. It does not fire for a
    /// sulfur cube with a block that a hit with a causing entity knocks back
    /// (`SulfurCube.knockback` does not call `super` there), for the pushes vanilla doesn't make
    /// with `LivingEntity.knockback` (the ender dragon's wings, Punch arrows, explosions, wind
    /// charges, a mace smash's area, a hoglin's throw), nor for the ones Sunrise makes another
    /// way (the ender dragon's). The level is the hit's: its blocks, random and own attachments.
    /// `kill_credit` is the pushed living entity's kill credit at this pre-motion moment:
    /// a remembered player takes precedence over its last hurting mob, even when that player
    /// cannot resolve (then 0, without falling back). `kill_credit_main` is that entity's
    /// current main-hand stack, or none. This is not the immediate damage-source attacker.
    /// Accepted-hit memories, including a tame wolf's owner, are committed before this read.
    /// Both are additions: NeoForge's `LivingKnockBackEvent` doesn't carry them, and mods
    /// there read the pushed entity's kill credit and its holder's main hand themselves.
    #[notes = "Version 2 adds the pushed entity's kill credit and its current main-hand stack; remembered player precedence and expiry match LivingRef::kill_credit."]
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        version = 2,
        contract = "cancel",
        out = "canceled",
        host = "level",
        neoforge = "LivingKnockBackEvent",
        target = "sunrise_client::game::native_hooks::base::HitLevel<'_>::native_living_knockback"
    )]
    fn living_knockback(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        attacker: i32,
        attacker_main: Option<&ItemStack>,
        attacker_off: Option<&ItemStack>,
        kill_credit: i32,
        kill_credit_main: Option<&ItemStack>,
        entry: KnockbackEntry,
        strength: &mut f64,
        x: &mut f64,
        z: &mut f64,
        canceled: &mut bool,
    );
}
