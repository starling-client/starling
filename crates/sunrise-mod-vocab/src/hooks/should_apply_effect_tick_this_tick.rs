//! Per-Part cadence of a registered effect.
// catalog-order: 0062

pub trait Hooks {
    /// `MobEffect.shouldApplyEffectTickThisTick` for this Part's effect.
    /// Each Part starts with `apply = false`; only that Part's answer decides
    /// whether its `apply_effect_tick` runs. `tick_count` is the remaining
    /// duration, or the holder's tick count for an infinite effect. For an
    /// effect declared by a `served_by` content package, its serving native
    /// host is the owning package (admitted without an extra dependency, and
    /// its Parts run first). Server only.
    #[hook(
        side = "server",
        scope = "effect",
        key = "effect",
        target = "mod_api::parts::effects::dispatch_mob_effect_tick",
        version = 4,
        contract = "listen",
        host = "read",
        neoforge = "none (vanilla MobEffect.shouldApplyEffectTickThisTick)"
    )]
    fn should_apply_effect_tick_this_tick(
        effect: EffectKey,
        tick_count: i32,
        amplifier: i32,
        apply: &mut bool,
        #[engine] make_call: impl FnOnce(EffectKey, i32) -> ApplyEffectTickCall<'_>,
    );
}
