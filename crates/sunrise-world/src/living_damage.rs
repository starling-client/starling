//! Inner living/player damage application after the outer hurt gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageStat {
    Absorbed,
    DealtAbsorbed,
    Taken,
}
/// What took a share of a hit before the health, as NeoForge's `DamageContainer.Reduction`
/// names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reduction {
    /// The damage cooldown: what the last hit already dealt (`lastHurt`), taken off a hit
    /// that lands within it.
    Invulnerability,
    /// Armor and toughness (`getDamageAfterArmorAbsorb`).
    Armor,
    /// Protection enchantments (`EnchantmentHelper.getDamageProtection`).
    Enchantments,
    /// Resistance (`getDamageAfterMagicAbsorb`'s mob effect step).
    MobEffects,
    /// The absorption hearts.
    Absorption,
    /// What the entity's own kind resists: a witch's magic resistance.
    InnateResistance,
}
const REDUCTIONS: usize = 6;
/// A per-hit reduction modifier callback (NeoForge's `IReductionFunction`).
/// Receives the live container at that reduction stage and the current reduction value
/// (vanilla's or the preceding modifier's output) and returns the new reduction value.
pub type ReductionModifier = Box<dyn FnMut(&DamageContainer, f32) -> f32>;
/// One hit's damage sequence from `hurtServer` to the health, as NeoForge's
/// `DamageContainer` carries it: the damage that came in, what blocking, the cooldown and
/// each reduction took, the damage `LivingDamageEvent.Pre` left and what the health lost.
/// Numeric values are 0 until the step that sets them has run; side effects are false until
/// `hurtServer` decides whether the hit is fresh.
pub struct DamageContainer {
    state: DamageState,
    live_reductions: bool,
    /// The modifiers registered for this hit, in the order they were added.
    reduction_modifiers: Vec<(Reduction, ReductionModifier)>,
}

/// The live hit values, moved through death when the owning level cannot run Post yet.
#[derive(Debug)]
pub struct DamageState {
    original: f32,
    new_damage: f32,
    blocked: f32,
    shield_damage: f32,
    post_attack_invulnerability_ticks: i32,
    should_cause_side_effects: bool,
    reductions: [f32; REDUCTIONS],
    inflicted: f32,
    health_damage: f32,
}

impl std::fmt::Debug for DamageContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DamageContainer")
            .field("state", &self.state)
            .field("reduction_modifiers_len", &self.reduction_modifiers.len())
            .finish()
    }
}

impl DamageContainer {
    /// A hit of `original` damage, as `hurtServer` was given it, with vanilla's 20 ticks of
    /// damage cooldown after it.
    pub fn new(original: f32) -> Self {
        Self {
            state: DamageState {
                original,
                new_damage: original,
                blocked: 0.0,
                shield_damage: 0.0,
                post_attack_invulnerability_ticks: 20,
                should_cause_side_effects: false,
                reductions: [0.0; REDUCTIONS],
                inflicted: 0.0,
                health_damage: 0.0,
            },
            live_reductions: false,
            reduction_modifiers: Vec::new(),
        }
    }
    /// Use NeoForge's damage-container accounting for a hit exposed to living damage hooks.
    pub fn enable_live_reductions(&mut self) {
        self.live_reductions = true;
    }
    /// Whether this hit uses NeoForge's accounting rather than the vanilla arithmetic.
    pub fn uses_live_reductions(&self) -> bool {
        self.live_reductions
    }
    /// Move the live hit values into the death continuation, releasing all reduction callbacks.
    pub fn into_state(self) -> DamageState {
        self.state
    }
    /// The damage sequence: vanilla's intermediate damage update without clamping.
    #[doc(hidden)]
    pub fn record_new_damage(&mut self, damage: f32) {
        self.state.new_damage = damage;
    }
    /// The damage sequence: record the selected cooldown or a checked listener write.
    #[doc(hidden)]
    pub fn record_post_attack_invulnerability_ticks(&mut self, ticks: i32) {
        self.state.post_attack_invulnerability_ticks = ticks;
    }
    /// The damage sequence: `hurtServer`'s fresh-hit decision before the damage hooks run.
    #[doc(hidden)]
    pub fn record_should_cause_side_effects(&mut self, fresh: bool) {
        self.state.should_cause_side_effects = fresh;
    }
    /// The damage sequence: what blocking stopped.
    #[doc(hidden)]
    pub fn record_block(&mut self, blocked: f32) {
        self.state.blocked = blocked;
    }
    /// The damage sequence: the durability the blocking item lost.
    #[doc(hidden)]
    pub fn record_shield_damage(&mut self, durability: f32) {
        self.state.shield_damage = durability;
    }
    /// Change what `reduction` takes off this hit, as NeoForge's
    /// `DamageContainer.addModifier` does. Add it from `living_incoming_damage` (or
    /// `living_damage_pre` for `Reduction::Absorption`): it applies when the guarded
    /// reduction site runs, to the amount it took, and the recorded reduction and the
    /// damage the sequence goes on with are the changed ones. Modifiers on one reduction
    /// apply in the order they were added, each receiving the live container and the
    /// preceding reduction value; the resulting damage is floored at `0.0`
    /// (`Math.max(new_damage - modified, 0.0F)`). A guarded step the hit does not enter
    /// (for example, no Resistance reduction or no Protection enchantments) runs no modifier;
    /// `Armor` and `Absorption` run at their enclosing `actuallyHurt` sites even with a zero
    /// starting reduction. Modifiers last for this hit.
    pub fn add_reduction_modifier(&mut self, reduction: Reduction, modifier: ReductionModifier) {
        self.enable_live_reductions();
        self.reduction_modifiers.push((reduction, modifier));
    }
    /// The damage sequence: what `reduction` took.
    #[doc(hidden)]
    pub fn record_reduction(&mut self, reduction: Reduction, amount: f32) {
        self.state.reductions[reduction as usize] = amount;
    }
    /// The registered modifiers applied in insertion order to `taken`; none when no
    /// modifier is registered for `reduction`.
    fn modified_reduction(&mut self, reduction: Reduction, taken: f32) -> Option<f32> {
        if !self
            .reduction_modifiers
            .iter()
            .any(|(r, _)| *r == reduction)
        {
            return None;
        }
        struct Restore<'a> {
            container: &'a mut DamageContainer,
            modifiers: Vec<(Reduction, ReductionModifier)>,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.container.reduction_modifiers = std::mem::take(&mut self.modifiers);
            }
        }
        let modifiers = std::mem::take(&mut self.reduction_modifiers);
        let mut guard = Restore {
            container: self,
            modifiers,
        };
        let mut modified = taken;
        for (r, modifier) in &mut guard.modifiers {
            if *r == reduction {
                modified = modifier(guard.container, modified);
            }
        }
        Some(modified)
    }
    /// The damage sequence: `reduction` took `taken` off `before`, leaving `after`. Records
    /// what it took and returns the damage the sequence goes on with. Call only at a guarded
    /// reduction site that actually ran (matching NeoForge's `DamageContainer.setReduction`);
    /// for an unhooked hit the vanilla values stand as they came.
    #[doc(hidden)]
    pub fn reduce(&mut self, reduction: Reduction, before: f32, taken: f32, after: f32) -> f32 {
        self.state.new_damage = before;
        let modified = self.modified_reduction(reduction, taken).unwrap_or(taken);
        self.record_reduction(reduction, modified);
        let next = if self.live_reductions {
            java_max_f32(before - modified, 0.0)
        } else {
            after
        };
        self.state.new_damage = next;
        next
    }
    /// Apply the absorption stage after Pre. Returns (damage to health, damage absorbed,
    /// remaining hearts). Vanilla computes spent hearts by subtraction; NeoForge takes the
    /// nominal share directly, even if no modifier was registered at this stage.
    pub fn absorb(&mut self, damage: f32, hearts: f32) -> (f32, f32, f32) {
        if self.live_reductions {
            let remaining = self.reduce(
                Reduction::Absorption,
                damage,
                java_min_f32(hearts, damage),
                damage,
            );
            let absorbed = java_min_f32(damage, self.reduction(Reduction::Absorption));
            (remaining, absorbed, java_max_f32(hearts - absorbed, 0.0))
        } else {
            let remaining = java_max_f32(damage - hearts, 0.0);
            let absorbed = damage - remaining;
            self.record_reduction(Reduction::Absorption, absorbed);
            self.record_new_damage(remaining);
            (remaining, absorbed, hearts - absorbed)
        }
    }
    /// The damage sequence: capture the damage Pre left, before absorption.
    #[doc(hidden)]
    pub fn capture_inflicted(&mut self) -> f32 {
        self.state.inflicted = self.state.new_damage;
        self.state.inflicted
    }
    /// The damage sequence: the health the hit took.
    #[doc(hidden)]
    pub fn record_health_damage(&mut self, damage: f32) {
        self.state.health_damage = damage;
    }
}

impl std::ops::Deref for DamageContainer {
    type Target = DamageState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DamageState {
    /// The damage the hit came in with (`getOriginalDamage`), before anything changed it.
    pub fn original(&self) -> f32 {
        self.original
    }
    /// The damage the sequence goes on with (`getNewDamage`).
    pub fn new_damage(&self) -> f32 {
        self.new_damage
    }
    /// The damage a blocking item stopped (`getBlockedDamage`).
    pub fn blocked(&self) -> f32 {
        self.blocked
    }
    /// The durability the raised blocking item lost (`getShieldDamage`).
    pub fn shield_damage(&self) -> f32 {
        self.shield_damage
    }
    /// The damage cooldown the entity has once the hit is applied
    /// (`getPostAttackInvulnerabilityTicks`): 20 for a fresh hit, what is left of it for a hit
    /// within it.
    pub fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.post_attack_invulnerability_ticks
    }
    /// Whether `hurtServer` decided this hit was fresh (`shouldCauseSideEffects`), so it
    /// causes hurt sounds, knockback and other fresh-hit effects. False for a stronger hit
    /// inside the damage cooldown unless its source bypasses the cooldown. This is recorded
    /// before `actuallyHurt`, independently of the final damage or modded cooldown ticks.
    pub fn should_cause_side_effects(&self) -> bool {
        self.should_cause_side_effects
    }
    /// What `reduction` took off the hit (`getReduction`).
    pub fn reduction(&self, reduction: Reduction) -> f32 {
        self.reductions[reduction as usize]
    }
    /// The damage `LivingDamageEvent.Pre` left (`getInflictedDamage`), before absorption.
    pub fn inflicted(&self) -> f32 {
        self.inflicted
    }
    /// The health the hit took (`getHealthDamage`).
    pub fn health_damage(&self) -> f32 {
        self.health_damage
    }
}

pub trait DamageWorld {
    type Error;
    type Attacker: Copy;
    fn invulnerable(&mut self) -> Result<bool, Self::Error>;
    fn after_armor(&mut self, damage: f32) -> Result<f32, Self::Error>;
    fn after_magic(
        &mut self,
        container: &mut DamageContainer,
        damage: f32,
    ) -> Result<f32, Self::Error>;
    fn absorption(&mut self) -> Result<f32, Self::Error>;
    fn set_absorption(&mut self, value: f32) -> Result<(), Self::Error>;
    fn server_player_attacker(&mut self) -> Result<Option<Self::Attacker>, Self::Error>;
    fn award_attacker(
        &mut self,
        attacker: Self::Attacker,
        stat: DamageStat,
        amount: i32,
    ) -> Result<(), Self::Error>;
    fn award_self(&mut self, stat: DamageStat, amount: i32) -> Result<(), Self::Error>;
    fn source_food_exhaustion(&mut self) -> Result<f32, Self::Error>;
    fn cause_food_exhaustion(&mut self, amount: f32) -> Result<(), Self::Error>;
    fn record_damage(&mut self, amount: f32) -> Result<(), Self::Error>;
    fn health(&mut self) -> Result<f32, Self::Error>;
    fn set_health(&mut self, value: f32) -> Result<(), Self::Error>;
    fn damage_event(&mut self) -> Result<(), Self::Error>;
    /// `LivingDamageEvent.Pre`: the armor and magic reductions are in `container`, and
    /// the absorption hearts have not taken their share; it may change the new damage. The
    /// default does nothing.
    fn damage_pre(&mut self, _container: &mut DamageContainer) -> Result<(), Self::Error> {
        Ok(())
    }
}
/// Java `Math.min(float, float)`: propagates `NaN` and prefers `-0.0` over `+0.0`.
pub fn java_min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() | b.to_bits())
    } else {
        a.min(b)
    }
}
/// Java `Math.max(float, float)`: propagates `NaN` and prefers `+0.0` over `-0.0`.
pub fn java_max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else {
        a.max(b)
    }
}
/// Java `Mth.clamp(float, float, float)`: `value < min ? min : Math.min(value, max)`.
pub fn java_clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else {
        java_min_f32(value, max)
    }
}
pub(crate) fn stat_amount(value: f32) -> i32 {
    let value = value * 10.0;
    // Java Math.round(float), including negative ties and integer saturation.
    let bits = value.to_bits() as i32;
    let shift = 149 - ((bits >> 23) & 0xff);
    if (0..32).contains(&shift) {
        let mut significand = (bits & 0x7f_ffff) | 0x80_0000;
        if bits < 0 {
            significand = -significand;
        }
        ((significand >> shift) + 1) >> 1
    } else {
        value as i32
    }
}
pub const STAT_LIMIT: f32 = 3.402_823_5e37;
pub fn actually_hurt<W: DamageWorld>(
    world: &mut W,
    container: &mut DamageContainer,
    damage: f32,
    player: bool,
) -> Result<(), W::Error> {
    if world.invulnerable()? {
        return Ok(());
    }
    actually_hurt_after_immunity_check(world, container, damage, player)
}
/// Caller must have performed this hit's native invulnerability query.
pub(crate) fn actually_hurt_after_immunity_check<W: DamageWorld>(
    world: &mut W,
    container: &mut DamageContainer,
    damage: f32,
    player: bool,
) -> Result<(), W::Error> {
    let after_armor = world.after_armor(damage)?;
    let after_armor = container.reduce(Reduction::Armor, damage, damage - after_armor, after_armor);
    actually_hurt_after_armor(world, container, after_armor, player)
}
pub(crate) fn actually_hurt_after_armor<W: DamageWorld>(
    world: &mut W,
    container: &mut DamageContainer,
    after_armor: f32,
    player: bool,
) -> Result<(), W::Error> {
    let magic_damage = world.after_magic(container, after_armor)?;
    container.record_new_damage(magic_damage);
    world.damage_pre(container)?;
    let damage = container.capture_inflicted();
    let (remaining, absorbed, hearts) = container.absorb(damage, world.absorption()?);
    world.set_absorption(hearts)?;
    if absorbed > 0.0 && absorbed < STAT_LIMIT {
        if player {
            world.award_self(DamageStat::Absorbed, stat_amount(absorbed))?;
        } else if let Some(attacker) = world.server_player_attacker()? {
            world.award_attacker(attacker, DamageStat::DealtAbsorbed, stat_amount(absorbed))?;
        }
    }
    if remaining == 0.0 {
        return Ok(());
    }
    if player {
        let exhaustion = world.source_food_exhaustion()?;
        world.cause_food_exhaustion(exhaustion)?;
    }
    world.record_damage(remaining)?;
    let health = world.health()?;
    world.set_health(health - remaining)?;
    if player {
        if remaining < STAT_LIMIT {
            world.award_self(DamageStat::Taken, stat_amount(remaining))?;
        }
    } else if !container.uses_live_reductions() {
        let absorption = world.absorption()?;
        world.set_absorption(absorption - remaining)?;
    }
    world.damage_event()?;
    container.record_health_damage(remaining);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    /// A living entity whose steps are simple enough to compute by hand: armor halves the
    /// hit, Resistance takes a quarter of what is left, protection takes half of that.
    struct Target {
        health: f32,
        absorption: f32,
        bypass_armor: bool,
        armor_wears: usize,
        self_stats: Vec<(DamageStat, i32)>,
        pre_damage: Vec<f32>,
        pre_override: Option<f32>,
    }
    impl Target {
        fn new(absorption: f32) -> Self {
            Self {
                health: 100.0,
                absorption,
                bypass_armor: false,
                armor_wears: 0,
                self_stats: Vec::new(),
                pre_damage: Vec::new(),
                pre_override: None,
            }
        }
    }
    impl DamageWorld for Target {
        type Error = ();
        type Attacker = ();
        fn invulnerable(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn after_armor(&mut self, damage: f32) -> Result<f32, ()> {
            Ok(if self.bypass_armor {
                damage
            } else {
                self.armor_wears += 1;
                damage * 0.5
            })
        }
        fn after_magic(&mut self, container: &mut DamageContainer, damage: f32) -> Result<f32, ()> {
            let after_effects = damage * 0.75;
            let after_effects = container.reduce(
                Reduction::MobEffects,
                damage,
                damage - after_effects,
                after_effects,
            );
            let after_enchantments = after_effects * 0.5;
            let after_enchantments = container.reduce(
                Reduction::Enchantments,
                after_effects,
                after_effects - after_enchantments,
                after_enchantments,
            );
            Ok(after_enchantments)
        }
        fn absorption(&mut self) -> Result<f32, ()> {
            Ok(self.absorption)
        }
        fn set_absorption(&mut self, value: f32) -> Result<(), ()> {
            self.absorption = value;
            Ok(())
        }
        fn server_player_attacker(&mut self) -> Result<Option<()>, ()> {
            Ok(None)
        }
        fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> Result<(), ()> {
            Ok(())
        }
        fn award_self(&mut self, stat: DamageStat, amount: i32) -> Result<(), ()> {
            self.self_stats.push((stat, amount));
            Ok(())
        }
        fn source_food_exhaustion(&mut self) -> Result<f32, ()> {
            Ok(0.0)
        }
        fn cause_food_exhaustion(&mut self, _: f32) -> Result<(), ()> {
            Ok(())
        }
        fn record_damage(&mut self, _: f32) -> Result<(), ()> {
            Ok(())
        }
        fn health(&mut self) -> Result<f32, ()> {
            Ok(self.health)
        }
        fn set_health(&mut self, value: f32) -> Result<(), ()> {
            self.health = value;
            Ok(())
        }
        fn damage_event(&mut self) -> Result<(), ()> {
            Ok(())
        }
        fn damage_pre(&mut self, container: &mut DamageContainer) -> Result<(), ()> {
            self.pre_damage.push(container.new_damage());
            if let Some(damage) = self.pre_override {
                container.record_new_damage(damage);
            }
            Ok(())
        }
    }
    fn hit(target: &mut Target, container: &mut DamageContainer, damage: f32) {
        actually_hurt(target, container, damage, true).unwrap();
    }
    fn reductions(c: &DamageContainer) -> [f32; 5] {
        [
            c.reduction(Reduction::Armor),
            c.reduction(Reduction::MobEffects),
            c.reduction(Reduction::Enchantments),
            c.reduction(Reduction::Absorption),
            c.reduction(Reduction::Invulnerability),
        ]
    }
    #[test]
    fn a_hit_with_no_modifier_takes_each_reduction_by_hand() {
        // 20: armor takes 10, Resistance 2.5 of the 10, protection half of the 7.5 left.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        hit(&mut target, &mut c, 20.0);
        assert_eq!(reductions(&c), [10.0, 2.5, 3.75, 0.0, 0.0]);
        assert_eq!(target.pre_damage, [3.75]);
        assert_eq!(c.health_damage(), 3.75);
        assert_eq!(target.health, 96.25);
        assert_eq!(c.original(), 20.0);
    }
    #[test]
    fn live_pre_damage_spends_the_nominal_absorption_without_an_absorption_modifier() {
        for damage in [33_554_432.0, f32::INFINITY] {
            let mut target = Target::new(1.0);
            target.pre_override = Some(damage);
            let mut container = DamageContainer::new(20.0);
            container.enable_live_reductions();
            hit(&mut target, &mut container, 20.0);
            assert_eq!(container.reduction(Reduction::Absorption), 1.0);
            assert_eq!(container.health_damage(), damage);
            assert_eq!(target.absorption, 0.0);
            assert!(target.self_stats.contains(&(DamageStat::Absorbed, 10)));
        }
    }
    #[test]
    fn several_modifiers_on_one_reduction_apply_in_the_order_added() {
        // Armor took 10: x0.5 then +2 is 7, +2 then x0.5 is 6.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v * 0.5),
        );
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v + 2.0),
        );
        hit(&mut target, &mut c, 20.0);
        // 13 goes on: Resistance takes 3.25, protection half of 9.75.
        assert_eq!(reductions(&c), [7.0, 3.25, 4.875, 0.0, 0.0]);
        assert_eq!(c.health_damage(), 4.875);
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v + 2.0),
        );
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v * 0.5),
        );
        hit(&mut target, &mut c, 20.0);
        // 14 goes on: Resistance takes 3.5, protection half of 10.5.
        assert_eq!(reductions(&c), [6.0, 3.5, 5.25, 0.0, 0.0]);
        assert_eq!(c.health_damage(), 5.25);
    }
    #[test]
    fn modifiers_on_different_reductions_each_change_their_own() {
        let (mut target, mut c) = (Target::new(3.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v.min(4.0)),
        );
        c.add_reduction_modifier(
            Reduction::MobEffects,
            Box::new(|_: &DamageContainer, v: f32| v + 1.0),
        );
        c.add_reduction_modifier(
            Reduction::Enchantments,
            Box::new(|_: &DamageContainer, v: f32| v + 1.0),
        );
        c.add_reduction_modifier(
            Reduction::Absorption,
            Box::new(|_: &DamageContainer, v: f32| v * 0.5),
        );
        hit(&mut target, &mut c, 20.0);
        // Armor 4 leaves 16; Resistance 4 + 1 leaves 11; protection half of 11 is 5.5, + 1 is
        // 6.5, leaving 4.5; the 3 hearts' 3 halved is 1.5, so 3 reaches the health.
        assert_eq!(reductions(&c), [4.0, 5.0, 6.5, 1.5, 0.0]);
        assert_eq!(target.pre_damage, [4.5]);
        assert_eq!(c.inflicted(), 4.5);
        assert_eq!(c.health_damage(), 3.0);
        assert_eq!(target.health, 97.0);
        assert_eq!(target.absorption, 1.5);
        assert_eq!(c.original(), 20.0);
    }
    #[test]
    fn a_changed_resistance_changes_what_the_enchantments_start_from() {
        // Resistance took 2.5 of 10: x2 is 5, so 5 goes on and protection takes half of it.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::MobEffects,
            Box::new(|_: &DamageContainer, v: f32| v * 2.0),
        );
        hit(&mut target, &mut c, 20.0);
        assert_eq!(reductions(&c), [10.0, 5.0, 2.5, 0.0, 0.0]);
        assert_eq!(c.health_damage(), 2.5);
    }
    #[test]
    fn unclamped_reductions_follow_neoforge_semantics() {
        // Over-limit armor reduction records `modified` verbatim while flooring resulting damage at 0.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v + 100.0),
        );
        hit(&mut target, &mut c, 20.0);
        assert_eq!(c.reduction(Reduction::Armor), 110.0);
        assert_eq!(c.new_damage(), 0.0);
        assert_eq!(c.health_damage(), 0.0);
        assert_eq!(target.health, 100.0);
        // Negative armor reduction increases the damage passed to later stages.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v - 20.0),
        );
        hit(&mut target, &mut c, 20.0);
        // Armor records -10, so 30 goes on: Resistance 7.5, protection half of 22.5 = 11.25.
        assert_eq!(reductions(&c), [-10.0, 7.5, 11.25, 0.0, 0.0]);
        assert_eq!(c.health_damage(), 11.25);
        assert_eq!(target.health, 88.75);
        // Over-limit absorption records `modified` verbatim, floors remaining damage at 0,
        // and deducts `damage.min(modified)` from hearts floored at 0.
        let (mut target, mut c) = (Target::new(2.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Absorption,
            Box::new(|_: &DamageContainer, v: f32| v + 50.0),
        );
        hit(&mut target, &mut c, 20.0);
        assert_eq!(c.reduction(Reduction::Absorption), 52.0);
        assert_eq!(target.absorption, 0.0);
        assert_eq!(c.new_damage(), 0.0);
        assert_eq!(c.health_damage(), 0.0);
        assert_eq!(target.health, 100.0);
        // Absorption modifier between hearts and damage: records 3.0 verbatim, leaves 0.75 health
        // damage, and deducts min(3.75, 3.0) = 3.0 from 2.0 hearts floored at 0.0.
        let (mut target, mut c) = (Target::new(2.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Absorption,
            Box::new(|_: &DamageContainer, _: f32| 3.0),
        );
        hit(&mut target, &mut c, 20.0);
        assert_eq!(c.reduction(Reduction::Absorption), 3.0);
        assert_eq!(target.absorption, 0.0);
        assert_eq!(c.new_damage(), 0.75);
        assert_eq!(c.health_damage(), 0.75);
        assert_eq!(target.health, 99.25);
    }
    #[test]
    fn a_nan_modifier_is_recorded_verbatim_without_fallback() {
        use std::cell::Cell;
        use std::rc::Rc;

        // Preceding NaN (from Armor) flows into Absorption callback input, leaves stored
        // absorption NaN, and awards no absorbed stat.
        let absorption_input = Rc::new(Cell::new(0.0_f32));
        let input_log = Rc::clone(&absorption_input);
        let (mut target, mut c) = (Target::new(3.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, _: f32| f32::NAN),
        );
        c.add_reduction_modifier(
            Reduction::Absorption,
            Box::new(move |_: &DamageContainer, taken: f32| {
                input_log.set(taken);
                taken
            }),
        );
        hit(&mut target, &mut c, 20.0);
        assert!(c.reduction(Reduction::Armor).is_nan());
        assert!(absorption_input.get().is_nan());
        assert!(c.reduction(Reduction::Absorption).is_nan());
        assert!(target.absorption.is_nan());
        assert!(c.new_damage().is_nan());
        assert!(
            !target
                .self_stats
                .iter()
                .any(|(stat, _)| *stat == DamageStat::Absorbed)
        );

        // NaN returned directly by Absorption modifier leaves stored absorption NaN and
        // awards no absorbed stat.
        let (mut target2, mut c2) = (Target::new(3.0), DamageContainer::new(20.0));
        c2.add_reduction_modifier(
            Reduction::Absorption,
            Box::new(|_: &DamageContainer, _: f32| f32::NAN),
        );
        hit(&mut target2, &mut c2, 20.0);
        assert!(c2.reduction(Reduction::Absorption).is_nan());
        assert!(target2.absorption.is_nan());
        assert!(c2.new_damage().is_nan());
        assert!(
            !target2
                .self_stats
                .iter()
                .any(|(stat, _)| *stat == DamageStat::Absorbed)
        );

        // Signed-zero and NaN helpers match Java Math.min / Math.max / Mth.clamp.
        assert_eq!(java_min_f32(-0.0, 0.0).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(java_min_f32(0.0, -0.0).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(java_max_f32(-0.0, 0.0).to_bits(), (0.0_f32).to_bits());
        assert_eq!(java_max_f32(0.0, -0.0).to_bits(), (0.0_f32).to_bits());
        assert!(java_min_f32(f32::NAN, 1.0).is_nan());
        assert!(java_min_f32(1.0, f32::NAN).is_nan());
        assert!(java_max_f32(f32::NAN, 1.0).is_nan());
        assert!(java_max_f32(1.0, f32::NAN).is_nan());
        assert!(java_clamp_f32(f32::NAN, 0.0, 20.0).is_nan());
    }
    #[test]
    fn reduction_closures_observe_live_container_stage() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (mut target, mut c) = (Target::new(3.0), DamageContainer::new(20.0));
        for &(stage, prev) in &[
            (Reduction::Armor, Reduction::Armor),
            (Reduction::MobEffects, Reduction::Armor),
            (Reduction::Enchantments, Reduction::MobEffects),
            (Reduction::Absorption, Reduction::Enchantments),
        ] {
            let log = Rc::clone(&seen);
            c.add_reduction_modifier(
                stage,
                Box::new(move |container: &DamageContainer, taken: f32| {
                    log.borrow_mut().push((
                        stage,
                        container.original(),
                        container.new_damage(),
                        container.reduction(prev),
                        taken,
                    ));
                    taken
                }),
            );
        }
        hit(&mut target, &mut c, 20.0);
        assert_eq!(
            *seen.borrow(),
            vec![
                (Reduction::Armor, 20.0, 20.0, 0.0, 10.0),
                (Reduction::MobEffects, 20.0, 10.0, 10.0, 2.5),
                (Reduction::Enchantments, 20.0, 7.5, 2.5, 3.75),
                (Reduction::Absorption, 20.0, 3.75, 3.75, 3.0),
            ]
        );
    }
    #[test]
    fn bypass_armor_still_runs_armor_modifier_with_zero_and_no_wear() {
        use std::cell::Cell;
        use std::rc::Rc;

        let seen = Rc::new(Cell::new(-1.0_f32));
        let seen_ref = Rc::clone(&seen);
        let mut target = Target::new(0.0);
        target.bypass_armor = true;
        let mut c = DamageContainer::new(20.0);
        c.add_reduction_modifier(
            Reduction::Armor,
            Box::new(move |_: &DamageContainer, v: f32| {
                seen_ref.set(v);
                v + 5.0
            }),
        );
        hit(&mut target, &mut c, 20.0);
        // LivingEntity.actuallyHurt still calls setReduction(ARMOR, 0.0) on #bypasses_armor,
        // without wearing equipment; the modifier takes 5 off 20, leaving 15 -> Resistance 3.75,
        // protection half of 11.25 = 5.625.
        assert_eq!(seen.get(), 0.0);
        assert_eq!(target.armor_wears, 0);
        assert_eq!(reductions(&c), [5.0, 3.75, 5.625, 0.0, 0.0]);
    }
    #[test]
    fn magic_stage_guards_run_modifiers_only_at_matching_sites() {
        use crate::combat_rules::ArmorWorld;
        use crate::damage_reduction::{Bypass, DamageReductionWorld, after_magic};
        use std::cell::RefCell;
        use std::rc::Rc;

        struct MagicWorld {
            bypass_effects: bool,
            bypass_resistance: bool,
            bypass_enchantments: bool,
            resistance_amp: Option<i32>,
            protection: f32,
        }
        impl ArmorWorld for MagicWorld {
            type Error = ();
            type Weapon = ();
            type ServerLevel = ();
            fn weapon(&mut self) -> Result<Option<()>, ()> {
                Ok(None)
            }
            fn server_level(&mut self) -> Result<Option<()>, ()> {
                Ok(Some(()))
            }
            fn modify_armor_effectiveness(&mut self, (): (), (): (), f: f32) -> Result<f32, ()> {
                Ok(f)
            }
        }
        impl DamageReductionWorld for MagicWorld {
            fn armor_value(&mut self) -> Result<i32, ()> {
                Ok(0)
            }
            fn toughness_attribute(&mut self) -> Result<f64, ()> {
                Ok(0.0)
            }
            fn bypasses(&mut self, bypass: Bypass) -> Result<bool, ()> {
                Ok(match bypass {
                    Bypass::Armor => false,
                    Bypass::Effects => self.bypass_effects,
                    Bypass::Resistance => self.bypass_resistance,
                    Bypass::Enchantments => self.bypass_enchantments,
                })
            }
            fn hurt_armor(&mut self, _: f32) -> Result<(), ()> {
                Ok(())
            }
            fn has_resistance(&mut self) -> Result<bool, ()> {
                Ok(self.resistance_amp.is_some())
            }
            fn resistance_amplifier(&mut self) -> Result<i32, ()> {
                Ok(self.resistance_amp.unwrap_or(0))
            }
            fn victim_is_server_player(&self) -> bool {
                true
            }
            fn attacker_is_server_player(&mut self) -> Result<bool, ()> {
                Ok(false)
            }
            fn award_resisted(&mut self, _: i32) -> Result<(), ()> {
                Ok(())
            }
            fn award_dealt_resisted(&mut self, _: i32) -> Result<(), ()> {
                Ok(())
            }
            fn damage_protection(&mut self, (): ()) -> Result<f32, ()> {
                Ok(self.protection)
            }
        }

        let run_case = |mut world: MagicWorld, damage: f32, override_reduction: Option<f32>| {
            let called = Rc::new(RefCell::new(Vec::<(Reduction, f32)>::new()));
            let mut c = DamageContainer::new(damage);
            for stage in [Reduction::MobEffects, Reduction::Enchantments] {
                let log = Rc::clone(&called);
                c.add_reduction_modifier(
                    stage,
                    Box::new(move |_: &DamageContainer, taken: f32| {
                        log.borrow_mut().push((stage, taken));
                        override_reduction.unwrap_or(taken + 1.0)
                    }),
                );
            }
            let out = after_magic(&mut world, &mut c, damage).unwrap();
            (out, called.borrow().clone(), c)
        };

        // Bypass::Effects skips both Resistance and Enchantments modifiers.
        let (out, called, _) = run_case(
            MagicWorld {
                bypass_effects: true,
                bypass_resistance: false,
                bypass_enchantments: false,
                resistance_amp: Some(0),
                protection: 8.0,
            },
            10.0,
            None,
        );
        assert_eq!(out, 10.0);
        assert!(called.is_empty());

        // Unenchanted (protection == 0.0) skips Enchantments modifier.
        let (_, called, _) = run_case(
            MagicWorld {
                bypass_effects: false,
                bypass_resistance: false,
                bypass_enchantments: false,
                resistance_amp: None,
                protection: 0.0,
            },
            10.0,
            None,
        );
        assert!(called.is_empty());

        // Zero-reduction Resistance (amp = -1 -> remainder = 25 -> resisted = 0.0), zero
        // damage, and resisted >= STAT_LIMIT all skip the MobEffects modifier.
        for (amp, dmg) in [(-1, 10.0), (0, 0.0), (0, STAT_LIMIT * 10.0)] {
            let (_, called, _) = run_case(
                MagicWorld {
                    bypass_effects: false,
                    bypass_resistance: false,
                    bypass_enchantments: false,
                    resistance_amp: Some(amp),
                    protection: 0.0,
                },
                dmg,
                None,
            );
            assert!(called.is_empty(), "amp={amp} dmg={dmg}");
        }

        // Present stages still run even when the modifier zeroes out their reduction.
        let (out, called, c) = run_case(
            MagicWorld {
                bypass_effects: false,
                bypass_resistance: false,
                bypass_enchantments: false,
                resistance_amp: Some(0),
                protection: 12.5,
            },
            10.0,
            Some(0.0),
        );
        assert_eq!(out, 10.0);
        assert_eq!(
            called,
            vec![(Reduction::MobEffects, 2.0), (Reduction::Enchantments, 6.0)]
        );
        assert_eq!(c.reduction(Reduction::MobEffects), 0.0);
        assert_eq!(c.reduction(Reduction::Enchantments), 0.0);

        // Resistance V's vanilla remainder is zero, even when the modifier restores
        // all 10 damage. The guarded protection site must not run.
        let (out, called, _) = run_case(
            MagicWorld {
                bypass_effects: false,
                bypass_resistance: false,
                bypass_enchantments: false,
                resistance_amp: Some(4),
                protection: 12.5,
            },
            10.0,
            Some(0.0),
        );
        assert_eq!(out, 10.0);
        assert_eq!(called, [(Reduction::MobEffects, 10.0)]);

        // Resistance I leaves 8 locally, and protection halves that to 4. If the
        // modifier drains the live value to zero, the enchantment reduction starts at -4.
        let (out, called, _) = run_case(
            MagicWorld {
                bypass_effects: false,
                bypass_resistance: false,
                bypass_enchantments: false,
                resistance_amp: Some(0),
                protection: 12.5,
            },
            10.0,
            Some(10.0),
        );
        assert_eq!(out, 0.0);
        assert_eq!(
            called,
            [
                (Reduction::MobEffects, 2.0),
                (Reduction::Enchantments, -4.0)
            ]
        );
    }
    #[test]
    fn non_player_absorption_modifier_does_not_double_debit_absorption() {
        // 20.0 -> Armor 10.0 -> MobEffects 2.5 -> Enchantments 3.75 -> inflicted 3.75.
        // With 4.0 hearts and a modifier returning 1.5, 2.25 reaches health and hearts stay 2.5
        // (no second vanilla non-player subtraction of `remaining`).
        let (mut target, mut c) = (Target::new(4.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(
            Reduction::Absorption,
            Box::new(|_: &DamageContainer, _: f32| 1.5),
        );
        actually_hurt(&mut target, &mut c, 20.0, false).unwrap();
        assert_eq!(c.reduction(Reduction::Absorption), 1.5);
        assert_eq!(c.health_damage(), 2.25);
        assert_eq!(target.health, 97.75);
        assert_eq!(target.absorption, 2.5);
    }
    #[test]
    fn modifiers_last_for_their_own_hit() {
        let mut target = Target::new(0.0);
        let mut first = DamageContainer::new(20.0);
        first.add_reduction_modifier(
            Reduction::Armor,
            Box::new(|_: &DamageContainer, v: f32| v + 5.0),
        );
        hit(&mut target, &mut first, 20.0);
        assert_eq!(first.reduction(Reduction::Armor), 15.0);
        let mut second = DamageContainer::new(20.0);
        hit(&mut target, &mut second, 20.0);
        assert_eq!(reductions(&second), [10.0, 2.5, 3.75, 0.0, 0.0]);
    }
    #[test]
    fn untouched_pre_preserves_non_finite_and_negative_damage() {
        for damage in [
            f32::NEG_INFINITY,
            -1.0,
            -0.0,
            f32::INFINITY,
            f32::NAN,
            f32::MAX,
        ] {
            let mut container = DamageContainer::new(damage);
            assert_eq!(container.capture_inflicted().to_bits(), damage.to_bits());
        }
    }
}
