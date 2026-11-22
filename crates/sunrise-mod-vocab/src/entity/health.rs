//! API-owned health and damage values and live damage container views.

/// What took a share of a hit before the health, as NeoForge's `DamageContainer.Reduction`
/// names it.
// no wire: preserves the compact u8 reduction tag; the structural enum derive uses u32 tags.
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

/// Engine backing for read-only live damage state ([`DamageContainerView`]).
// hidden: the engine owns the live hit state; listeners read through DamageContainerView.
// no wire: engine-implemented live damage bridge.
#[doc(hidden)]
pub trait DamageContainerRead {
    /// The damage the hit came in with (`getOriginalDamage`), before anything changed it.
    fn original(&self) -> f32;
    /// The damage the sequence currently goes on with (`getNewDamage`).
    fn new_damage(&self) -> f32;
    /// The damage a blocking item stopped (`getBlockedDamage`).
    fn blocked(&self) -> f32;
    /// The durability the raised blocking item lost (`getShieldDamage`).
    fn shield_damage(&self) -> f32;
    /// The damage cooldown the entity has once the hit is applied
    /// (`getPostAttackInvulnerabilityTicks`).
    fn post_attack_invulnerability_ticks(&self) -> i32;
    /// Whether `hurtServer` decided this hit causes fresh-hit side effects
    /// (`shouldCauseSideEffects`).
    fn should_cause_side_effects(&self) -> bool;
    /// The damage `LivingDamageEvent.Pre` left (`getInflictedDamage`), before absorption.
    fn inflicted(&self) -> f32;
    /// The health the hit took (`getHealthDamage`).
    fn health_damage(&self) -> f32;
    /// The amount taken by one reduction kind (`getReduction`).
    fn reduction(&self, reduction: Reduction) -> f32;
}

/// Engine backing for mutable live damage state ([`DamageContainer`]).
// hidden: the engine owns the live hit state; listeners mutate through DamageContainer.
// no wire: engine-implemented live damage bridge.
#[doc(hidden)]
pub trait DamageContainerBackend: DamageContainerRead {
    /// Change `new_damage` on the live hit container.
    fn set_new_damage(&mut self, value: f32);
    /// Change `post_attack_invulnerability_ticks` on the live hit container.
    fn set_post_attack_invulnerability_ticks(&mut self, value: i32);
    /// Append an engine-owned reduction callback for `reduction`, attributed to `owner`.
    fn add_reduction_modifier(
        &mut self,
        reduction: Reduction,
        owner: Option<&'static str>,
        modifier: Box<dyn FnMut(DamageContainerView<'_>, f32) -> f32>,
    );
}

const REDUCTION_CALLBACK: &str = "living_damage_reduction";
const REDUCTION_KEY: crate::runtime::Callback = crate::runtime::Callback::DamageReduction;

/// Attribute the stored reduction callback to the mod that registered it and dispatch it
/// through the same failure boundary as hook listeners.
// hidden: engine-side live damage bridge helper, not part of the mod surface.
#[doc(hidden)]
pub fn wrap_reduction_modifier(
    owner: Option<&'static str>,
    modifier: Box<dyn FnMut(DamageContainerView<'_>, f32) -> f32>,
) -> Box<dyn FnMut(DamageContainerView<'_>, f32) -> f32> {
    let mut callback = OwnedReduction {
        owner: owner.unwrap_or("unknown"),
        owner_index: owner.and_then(crate::__private::mod_index),
        modifier: Some(modifier),
    };
    Box::new(move |view, amount| callback.run(view, amount))
}

struct OwnedReduction {
    owner: &'static str,
    owner_index: Option<usize>,
    modifier: Option<Box<dyn FnMut(DamageContainerView<'_>, f32) -> f32>>,
}

impl OwnedReduction {
    fn run(&mut self, view: DamageContainerView<'_>, amount: f32) -> f32 {
        if self.owner_index.is_some_and(|i| !crate::mod_index_on(i))
            || crate::runtime::poisoned(REDUCTION_KEY, self.owner)
        {
            return amount;
        }
        let modifier = self
            .modifier
            .as_mut()
            .expect("reduction callback is live until dropped");
        let _guard = crate::mark_mod(self.owner, REDUCTION_CALLBACK);
        crate::runtime::invoke(REDUCTION_KEY, REDUCTION_CALLBACK, self.owner, || {
            modifier(view, amount)
        })
        .unwrap_or(amount)
    }
}

impl Drop for OwnedReduction {
    fn drop(&mut self) {
        // Captures can run mod code when dropped, even if the hit was cancelled,
        // the stage never ran, or the mod's callback is already disabled.
        let _guard = crate::mark_mod(self.owner, REDUCTION_CALLBACK);
        crate::runtime::invoke(REDUCTION_KEY, REDUCTION_CALLBACK, self.owner, || {
            drop(self.modifier.take());
        });
    }
}

/// Read-only live view of one hit's damage sequence (NeoForge's `DamageContainer` as seen by
/// `IReductionFunction` and `LivingDamageEvent.Post`).
// exhaustive: only the engine constructs a view.
// no wire: live hit-scoped borrow of engine damage state, not a serialized payload.
#[derive(Clone, Copy)]
pub struct DamageContainerView<'a> {
    backend: &'a dyn DamageContainerRead,
}

impl<'a> DamageContainerView<'a> {
    /// Construct a read-only view around the engine's live hit container.
    // hidden: only the engine constructs a view around its live hit container.
    #[doc(hidden)]
    pub fn new(backend: &'a dyn DamageContainerRead) -> Self {
        Self { backend }
    }

    /// Read the hit's `original` damage (`getOriginalDamage`), before anything changed it.
    pub fn original(&self) -> f32 {
        self.backend.original()
    }
    /// Read the damage the sequence currently goes on with (`getNewDamage`).
    pub fn new_damage(&self) -> f32 {
        self.backend.new_damage()
    }
    /// Read the damage a blocking item stopped (`getBlockedDamage`).
    pub fn blocked(&self) -> f32 {
        self.backend.blocked()
    }
    /// Read the durability the raised blocking item lost (`getShieldDamage`).
    pub fn shield_damage(&self) -> f32 {
        self.backend.shield_damage()
    }
    /// Read the post-attack invulnerability ticks (`getPostAttackInvulnerabilityTicks`).
    pub fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.backend.post_attack_invulnerability_ticks()
    }
    /// Read whether `hurtServer` decided this hit causes fresh-hit side effects
    /// (`shouldCauseSideEffects`).
    pub fn should_cause_side_effects(&self) -> bool {
        self.backend.should_cause_side_effects()
    }
    /// Read the damage `LivingDamageEvent.Pre` left (`getInflictedDamage`), before absorption.
    pub fn inflicted(&self) -> f32 {
        self.backend.inflicted()
    }
    /// Read the health the hit took (`getHealthDamage`).
    pub fn health_damage(&self) -> f32 {
        self.backend.health_damage()
    }
    /// Read the amount taken by one reduction kind (`getReduction`).
    pub fn reduction(&self, reduction: Reduction) -> f32 {
        self.backend.reduction(reduction)
    }
}

impl std::fmt::Debug for DamageContainerView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DamageContainerView")
            .field("original", &self.original())
            .field("new_damage", &self.new_damage())
            .field("blocked", &self.blocked())
            .field("shield_damage", &self.shield_damage())
            .field(
                "post_attack_invulnerability_ticks",
                &self.post_attack_invulnerability_ticks(),
            )
            .field(
                "should_cause_side_effects",
                &self.should_cause_side_effects(),
            )
            .field(
                "reductions",
                &[
                    self.reduction(Reduction::Invulnerability),
                    self.reduction(Reduction::Armor),
                    self.reduction(Reduction::Enchantments),
                    self.reduction(Reduction::MobEffects),
                    self.reduction(Reduction::Absorption),
                    self.reduction(Reduction::InnateResistance),
                ],
            )
            .field("inflicted", &self.inflicted())
            .field("health_damage", &self.health_damage())
            .finish()
    }
}

/// Why a live damage-container write was refused. A refused write leaves the value unchanged.
// no wire: refusal from a live hit-scoped operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DamageWriteRefusal {
    /// The requested damage was negative or NaN.
    InvalidDamage,
    /// The requested cooldown was negative.
    NegativeCooldown,
}

impl std::fmt::Display for DamageWriteRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidDamage => "damage must be nonnegative and not NaN",
            Self::NegativeCooldown => "damage cooldown ticks must be nonnegative",
        })
    }
}

impl std::error::Error for DamageWriteRefusal {}

/// Live mutable borrow of one hit's damage sequence (NeoForge's `DamageContainer` as seen by
/// `LivingIncomingDamageEvent` and `LivingDamageEvent.Pre`). Sequencing remains owned by the engine.
// exhaustive: only the engine constructs a live view.
// no wire: live hit-scoped mutable borrow of engine damage state, not a serialized payload.
pub struct DamageContainer<'a> {
    backend: &'a mut dyn DamageContainerBackend,
}

impl<'a> DamageContainer<'a> {
    /// Construct a mutable live view around the engine's hit container.
    // hidden: only the engine constructs a live view around its hit container.
    #[doc(hidden)]
    pub fn new(backend: &'a mut dyn DamageContainerBackend) -> Self {
        Self { backend }
    }

    /// Borrow a read-only view of this live damage container.
    pub fn as_view(&self) -> DamageContainerView<'_> {
        DamageContainerView {
            backend: &*self.backend,
        }
    }

    /// Read the hit's `original` damage (`getOriginalDamage`), before anything changed it.
    pub fn original(&self) -> f32 {
        self.backend.original()
    }
    /// Read the damage the sequence currently goes on with (`getNewDamage`).
    pub fn new_damage(&self) -> f32 {
        self.backend.new_damage()
    }
    /// Read the damage a blocking item stopped (`getBlockedDamage`).
    pub fn blocked(&self) -> f32 {
        self.backend.blocked()
    }
    /// Read the durability the raised blocking item lost (`getShieldDamage`).
    pub fn shield_damage(&self) -> f32 {
        self.backend.shield_damage()
    }
    /// Read the post-attack invulnerability ticks (`getPostAttackInvulnerabilityTicks`).
    pub fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.backend.post_attack_invulnerability_ticks()
    }
    /// Read whether `hurtServer` decided this hit causes fresh-hit side effects
    /// (`shouldCauseSideEffects`).
    pub fn should_cause_side_effects(&self) -> bool {
        self.backend.should_cause_side_effects()
    }
    /// Read the damage `LivingDamageEvent.Pre` left (`getInflictedDamage`), before absorption.
    pub fn inflicted(&self) -> f32 {
        self.backend.inflicted()
    }
    /// Read the health the hit took (`getHealthDamage`).
    pub fn health_damage(&self) -> f32 {
        self.backend.health_damage()
    }
    /// Read the amount taken by one reduction kind (`getReduction`).
    pub fn reduction(&self, reduction: Reduction) -> f32 {
        self.backend.reduction(reduction)
    }

    /// Change the live damage. A negative or NaN value returns
    /// [`DamageWriteRefusal::InvalidDamage`] and leaves the previous value unchanged.
    pub fn set_new_damage(&mut self, value: f32) -> Result<(), DamageWriteRefusal> {
        if value.is_nan() || value < 0.0 {
            let refusal = DamageWriteRefusal::InvalidDamage;
            log::warn!(
                "native refusal: {:?}: {refusal}",
                crate::content::crash::current_native_call()
            );
            return Err(refusal);
        }
        self.backend.set_new_damage(value);
        Ok(())
    }
    /// Change the live post-attack cooldown. Negative ticks return
    /// [`DamageWriteRefusal::NegativeCooldown`] and leave the previous value unchanged.
    pub fn set_post_attack_invulnerability_ticks(
        &mut self,
        value: i32,
    ) -> Result<(), DamageWriteRefusal> {
        if value < 0 {
            let refusal = DamageWriteRefusal::NegativeCooldown;
            log::warn!(
                "native refusal: {:?}: {refusal}",
                crate::content::crash::current_native_call()
            );
            return Err(refusal);
        }
        self.backend.set_post_attack_invulnerability_ticks(value);
        Ok(())
    }

    /// Append a per-hit reduction modifier callback, applied by the engine in registration order
    /// when `reduction` runs (NeoForge's `DamageContainer.addModifier` / `IReductionFunction`).
    ///
    /// Each callback receives a read-only view of the live container at that reduction stage and
    /// the current reduction value (vanilla's or the preceding modifier's output), and returns the
    /// new reduction value. A panic in the callback disables this mod's reduction callbacks for
    /// the rest of the session and leaves the incoming reduction value unchanged.
    pub fn add_reduction_modifier(
        &mut self,
        reduction: Reduction,
        modifier: impl FnMut(DamageContainerView<'_>, f32) -> f32 + 'static,
    ) {
        let owner = crate::content::crash::current_native_mod();
        self.backend
            .add_reduction_modifier(reduction, owner, Box::new(modifier));
    }
}

impl std::fmt::Debug for DamageContainer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DamageContainer")
            .field("original", &self.original())
            .field("new_damage", &self.new_damage())
            .field("blocked", &self.blocked())
            .field("shield_damage", &self.shield_damage())
            .field(
                "post_attack_invulnerability_ticks",
                &self.post_attack_invulnerability_ticks(),
            )
            .field(
                "should_cause_side_effects",
                &self.should_cause_side_effects(),
            )
            .field(
                "reductions",
                &[
                    self.reduction(Reduction::Invulnerability),
                    self.reduction(Reduction::Armor),
                    self.reduction(Reduction::Enchantments),
                    self.reduction(Reduction::MobEffects),
                    self.reduction(Reduction::Absorption),
                    self.reduction(Reduction::InnateResistance),
                ],
            )
            .field("inflicted", &self.inflicted())
            .field("health_damage", &self.health_damage())
            .finish()
    }
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A hit's `DamageSource` for `HURT_SERVER`: its type (`minecraft:fall`),
/// the entity ids of `getEntity` (`causing`) and `getDirectEntity`
/// (`direct`), 0 for none or not known, and where it comes from
/// (`getSourcePosition`), none when not known.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct HurtSource<'a> {
    pub damage_type: &'a str,
    pub causing: i32,
    /// The projectile's entity id when the damage type names one (an arrow, a thrown
    /// entity, an owned wither skull), including an ownerless projectile; the attacker
    /// itself for melee when known. 0 when the source names none, including `magic()`
    /// dealt during a projectile's impact (an ownerless wither skull, a tipped arrow's
    /// instant harm).
    pub direct: i32,
    pub position: Option<[f64; 3]>,
}
/// Which entry of vanilla's `LivingEntity.knockback` a push came through, as
/// `living_knockback` tells its listener. The short entry (power, direction) only delegates to
/// the full one (which also takes whether the push comes from an effect), so one push through
/// the short entry reaches the hook once, as `Short`; a call of the full entry itself is `Full`.
/// A listener that changes the strength where a mixin on both entries would change it twice
/// for a short-entry call and once for a full-entry call applies its change twice on `Short`.
// exhaustive: vanilla has exactly these two entries, `knockback(power, x, z)` and the full one
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
pub enum KnockbackEntry {
    /// The push entered through `knockback(power, x, z)`, which then calls the full entry.
    Short,
    /// The push entered the full entry directly.
    Full,
}
/// Where the entity a hit lands on stands and looks (`getViewVector`), for
/// `HURT_SERVER`.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Victim {
    pub position: [f64; 3],
    pub view: [f64; 3],
}
/// What a player's raised blocking item does to a hit, as `living_shield_block`
/// may change it (`LivingShieldBlockEvent`).
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ShieldBlock {
    /// The player counts as blocking.
    /// The damage sequence's `blocked` value.
    pub blocked: bool,
    /// The damage the item stops, kept within 0 and the hit's damage.
    pub blocked_damage: f32,
    /// The durability the item loses (`ItemDamageFunction` of what it stops). Below 0,
    /// vanilla's `ItemDamageFunction` of the final `blocked_damage` applies.
    /// The damage sequence's `shield_damage` value.
    pub shield_damage: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    type ReductionModifier = Box<dyn FnMut(DamageContainerView<'_>, f32) -> f32>;

    struct FakeDamageBackend {
        original: f32,
        new_damage: f32,
        blocked: f32,
        shield_damage: f32,
        post_attack_invulnerability_ticks: i32,
        should_cause_side_effects: bool,
        reductions: [f32; 6],
        inflicted: f32,
        health_damage: f32,
        modifiers: Vec<(Reduction, ReductionModifier)>,
    }

    impl FakeDamageBackend {
        fn new(original: f32) -> Self {
            Self {
                original,
                new_damage: original,
                blocked: 0.0,
                shield_damage: 0.0,
                post_attack_invulnerability_ticks: 20,
                should_cause_side_effects: true,
                reductions: [0.0; 6],
                inflicted: 0.0,
                health_damage: 0.0,
                modifiers: Vec::new(),
            }
        }

        fn apply_reduction(&mut self, reduction: Reduction, initial: f32) -> f32 {
            let mut modifiers = std::mem::take(&mut self.modifiers);
            let mut amount = initial;
            for (kind, modifier) in &mut modifiers {
                if *kind == reduction {
                    let view = DamageContainerView::new(self);
                    amount = modifier(view, amount);
                }
            }
            self.modifiers = modifiers;
            self.reductions[reduction as usize] = amount;
            self.new_damage = (self.new_damage - amount).max(0.0);
            amount
        }
    }

    impl DamageContainerRead for FakeDamageBackend {
        fn original(&self) -> f32 {
            self.original
        }
        fn new_damage(&self) -> f32 {
            self.new_damage
        }
        fn blocked(&self) -> f32 {
            self.blocked
        }
        fn shield_damage(&self) -> f32 {
            self.shield_damage
        }
        fn post_attack_invulnerability_ticks(&self) -> i32 {
            self.post_attack_invulnerability_ticks
        }
        fn should_cause_side_effects(&self) -> bool {
            self.should_cause_side_effects
        }
        fn inflicted(&self) -> f32 {
            self.inflicted
        }
        fn health_damage(&self) -> f32 {
            self.health_damage
        }
        fn reduction(&self, reduction: Reduction) -> f32 {
            self.reductions[reduction as usize]
        }
    }

    impl DamageContainerBackend for FakeDamageBackend {
        fn set_new_damage(&mut self, value: f32) {
            self.new_damage = value;
        }
        fn set_post_attack_invulnerability_ticks(&mut self, value: i32) {
            self.post_attack_invulnerability_ticks = value;
        }
        fn add_reduction_modifier(
            &mut self,
            reduction: Reduction,
            owner: Option<&'static str>,
            modifier: Box<dyn FnMut(DamageContainerView<'_>, f32) -> f32>,
        ) {
            let wrapped = wrap_reduction_modifier(owner, modifier);
            self.modifiers.push((reduction, wrapped));
        }
    }

    struct DummyMod;
    #[mod_api::__subscriptions]
    impl crate::Hooks for DummyMod {}

    #[test]
    fn live_view_and_container_read_write_and_run_reduction_modifiers() {
        let mut backend = FakeDamageBackend::new(12.0);
        backend.blocked = 1.5;
        backend.shield_damage = 2.0;
        backend.should_cause_side_effects = false;
        backend.inflicted = 7.0;
        backend.health_damage = 5.0;
        backend.reductions[Reduction::Invulnerability as usize] = 0.5;

        let observed = Arc::new(Mutex::new(Vec::new()));
        let observed_first = observed.clone();
        {
            let mut container = DamageContainer::new(&mut backend);
            let view = container.as_view();
            let copied = view;
            assert_eq!(container.original(), 12.0);
            assert_eq!(copied.original(), 12.0);
            assert_eq!(copied.new_damage(), 12.0);
            assert_eq!(copied.blocked(), 1.5);
            assert_eq!(copied.shield_damage(), 2.0);
            assert_eq!(copied.post_attack_invulnerability_ticks(), 20);
            assert!(!copied.should_cause_side_effects());
            assert_eq!(copied.inflicted(), 7.0);
            assert_eq!(copied.health_damage(), 5.0);
            assert_eq!(copied.reduction(Reduction::Invulnerability), 0.5);
            assert!(format!("{copied:?}").contains("DamageContainerView"));
            assert!(format!("{container:?}").contains("DamageContainer"));

            assert_eq!(container.set_new_damage(10.0), Ok(()));
            assert_eq!(container.set_post_attack_invulnerability_ticks(15), Ok(()));
            for invalid in [-1.0, f32::NAN] {
                assert_eq!(
                    container.set_new_damage(invalid),
                    Err(DamageWriteRefusal::InvalidDamage)
                );
                assert_eq!(container.new_damage(), 10.0);
            }
            assert_eq!(
                container.set_post_attack_invulnerability_ticks(-1),
                Err(DamageWriteRefusal::NegativeCooldown)
            );
            assert_eq!(container.post_attack_invulnerability_ticks(), 15);
            assert_eq!(container.new_damage(), 10.0);
            assert_eq!(container.as_view().new_damage(), 10.0);
            assert_eq!(container.post_attack_invulnerability_ticks(), 15);

            container.add_reduction_modifier(Reduction::Armor, move |live, amount| {
                observed_first
                    .lock()
                    .unwrap()
                    .push((live.original(), live.new_damage(), amount));
                amount + 1.0
            });
            container.add_reduction_modifier(Reduction::Armor, |_live, amount| amount * 0.5);
        }

        assert_eq!(backend.new_damage, 10.0);
        assert_eq!(backend.post_attack_invulnerability_ticks, 15);
        let applied = backend.apply_reduction(Reduction::Armor, 3.0);
        assert_eq!(applied, 2.0);
        assert_eq!(backend.reduction(Reduction::Armor), 2.0);
        assert_eq!(backend.new_damage, 8.0);
        assert_eq!(*observed.lock().unwrap(), vec![(12.0, 10.0, 3.0)]);
    }

    #[test]
    fn panicking_reduction_callback_disables_only_its_mod_and_reports_fault() {
        crate::testing::with_dispatch(
            vec![
                crate::ModEntry::new("bad_mod", Box::new(DummyMod)),
                crate::ModEntry::new("good_mod", Box::new(DummyMod)),
            ]
            .into(),
            || {
                let bad_runs = Arc::new(AtomicUsize::new(0));
                let bad_second_runs = Arc::new(AtomicUsize::new(0));
                let good_runs = Arc::new(AtomicUsize::new(0));
                let bad_runs_cb = bad_runs.clone();
                let bad_second_cb = bad_second_runs.clone();
                let good_runs_cb = good_runs.clone();

                let mut backend = FakeDamageBackend::new(20.0);
                {
                    let mut container = DamageContainer::new(&mut backend);
                    {
                        let _guard = crate::mark_mod("bad_mod", "living_incoming_damage");
                        container.add_reduction_modifier(
                            Reduction::Armor,
                            move |_view, _amount| {
                                bad_runs_cb.fetch_add(1, Ordering::Relaxed);
                                panic!("boom in bad_mod");
                            },
                        );
                        container.add_reduction_modifier(Reduction::Armor, move |_view, amount| {
                            bad_second_cb.fetch_add(1, Ordering::Relaxed);
                            amount + 100.0
                        });
                    }
                    {
                        let _guard = crate::mark_mod("good_mod", "living_incoming_damage");
                        container.add_reduction_modifier(Reduction::Armor, move |_view, amount| {
                            let call = crate::content::crash::current_native_call().unwrap();
                            assert_eq!(call.id, "good_mod");
                            assert_eq!(call.hook, "living_damage_reduction");
                            good_runs_cb.fetch_add(1, Ordering::Relaxed);
                            amount + 2.0
                        });
                    }
                }

                // First hit: bad_mod panics and is poisoned; bad_mod's second callback is skipped;
                // good_mod still runs on the original reduction value.
                assert_eq!(backend.apply_reduction(Reduction::Armor, 4.0), 6.0);
                assert_eq!(bad_runs.load(Ordering::Relaxed), 1);
                assert_eq!(bad_second_runs.load(Ordering::Relaxed), 0);
                assert_eq!(good_runs.load(Ordering::Relaxed), 1);
                assert!(crate::runtime::poisoned(REDUCTION_KEY, "bad_mod"));
                assert!(!crate::runtime::poisoned(REDUCTION_KEY, "good_mod"));

                // Second hit: bad_mod remains disabled for the session while good_mod keeps running.
                assert_eq!(backend.apply_reduction(Reduction::Armor, 4.0), 6.0);
                assert_eq!(bad_runs.load(Ordering::Relaxed), 1);
                assert_eq!(bad_second_runs.load(Ordering::Relaxed), 0);
                assert_eq!(good_runs.load(Ordering::Relaxed), 2);

                // Newly registered callbacks from bad_mod are also skipped for this session.
                let fresh_bad_runs = Arc::new(AtomicUsize::new(0));
                let fresh_bad_cb = fresh_bad_runs.clone();
                let mut second_hit = FakeDamageBackend::new(10.0);
                {
                    let mut container = DamageContainer::new(&mut second_hit);
                    let _guard = crate::mark_mod("bad_mod", "living_incoming_damage");
                    container.add_reduction_modifier(Reduction::Armor, move |_view, amount| {
                        fresh_bad_cb.fetch_add(1, Ordering::Relaxed);
                        amount + 5.0
                    });
                }
                assert_eq!(second_hit.apply_reduction(Reduction::Armor, 3.0), 3.0);
                assert_eq!(fresh_bad_runs.load(Ordering::Relaxed), 0);

                // Disabling good_mod skips its callback without poisoning it.
                assert!(crate::set_mod_enabled("good_mod", false));
                assert_eq!(backend.apply_reduction(Reduction::Armor, 4.0), 4.0);
                assert_eq!(good_runs.load(Ordering::Relaxed), 2);
                assert!(crate::set_mod_enabled("good_mod", true));
                assert_eq!(backend.apply_reduction(Reduction::Armor, 4.0), 6.0);
                assert_eq!(good_runs.load(Ordering::Relaxed), 3);

                let faults = crate::__private::take_hook_panics();
                assert_eq!(
                    faults,
                    vec![crate::__private::HookPanicReport {
                        mod_id: "bad_mod",
                        hook: "living_damage_reduction",
                    }]
                );
                assert!(crate::__private::take_hook_panics().is_empty());
            },
        );
    }

    #[test]
    fn anonymous_panicking_reduction_callback_disables_itself_and_deduplicates_faults() {
        crate::testing::with_dispatch(Vec::new().into(), || {
            let runs = Arc::new(AtomicUsize::new(0));
            let runs_cb = runs.clone();
            let mut backend = FakeDamageBackend::new(10.0);
            {
                let mut container = DamageContainer::new(&mut backend);
                container.add_reduction_modifier(Reduction::Armor, move |_view, _amount| {
                    runs_cb.fetch_add(1, Ordering::Relaxed);
                    panic!("{}", String::from("anon boom"));
                });
            }

            assert_eq!(backend.apply_reduction(Reduction::Armor, 2.0), 2.0);
            assert_eq!(runs.load(Ordering::Relaxed), 1);
            assert_eq!(backend.apply_reduction(Reduction::Armor, 2.0), 2.0);
            assert_eq!(runs.load(Ordering::Relaxed), 1);

            let faults = crate::__private::take_hook_panics();
            assert_eq!(
                faults,
                vec![crate::__private::HookPanicReport {
                    mod_id: "unknown",
                    hook: "living_damage_reduction",
                }]
            );
        });
    }
}
