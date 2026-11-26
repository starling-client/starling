//! The typed inside-block effects and callbacks an `entity_inside` part queues.
//!
//! Vanilla's `Block.entityInside` runs mid-walk and queues its five
//! `InsideBlockEffectType`s and the `runBefore`/`runAfter` callbacks around them
//! (`InsideBlockEffectApplier.StepBasedCollector`). Callbacks run when the collector
//! flushes (`apply_and_clear`), each lent an [`InsideEntityView`] over the swept entity
//! and its live level, stopping as soon as the entity dies.
use crate::{BlockState, DamageTypeKey, EffectKey, EntityRef};

/// Exclusive loan of the swept entity and its live level, lent to queued `run_before`
/// and `run_after` callbacks when `InsideBlockEffectApplier.StepBasedCollector.applyAndClear`
/// flushes them.
// exhaustive: only the engine constructs a view.
// no wire: an exclusive loan of the swept entity and live level during apply_and_clear, not a payload.
pub struct InsideEntityView<'a> {
    access: &'a mut dyn InsideEntityAccess,
}

impl<'a> InsideEntityView<'a> {
    // hidden: the engine constructs the view when flushing queued inside-block callbacks.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn InsideEntityAccess) -> Self {
        Self { access }
    }

    /// The swept entity's numeric id (`Entity.getId`).
    pub fn id(&self) -> u32 {
        self.access.id()
    }

    /// The swept entity's registry type id (for example `"minecraft:player"`).
    pub fn kind(&self) -> &str {
        self.access.kind()
    }

    /// Whether the swept entity is still alive (`Entity.isAlive`).
    pub fn is_alive(&self) -> bool {
        self.access.is_alive()
    }

    /// The swept entity's current world position (`Entity.position`).
    pub fn position(&self) -> [f64; 3] {
        self.access.position()
    }

    /// The swept entity's current velocity (`Entity.getDeltaMovement`).
    pub fn velocity(&self) -> [f64; 3] {
        self.access.velocity()
    }

    /// Remaining fire ticks (`Entity.getRemainingFireTicks`); positive values mean burning.
    pub fn fire_ticks(&self) -> i32 {
        self.access.fire_ticks()
    }

    /// Whether the swept entity is on fire (`Entity.isOnFire`), excluding fire-immune entities.
    pub fn is_on_fire(&self) -> bool {
        self.access.is_on_fire()
    }

    /// The swept entity's accumulated fall distance (`Entity.fallDistance`).
    pub fn fall_distance(&self) -> f64 {
        self.access.fall_distance()
    }

    /// Read the block state at `pos` in the swept entity's live level, or `None` if unloaded
    /// or outside world height.
    pub fn block_state(&self, pos: [i32; 3]) -> Option<BlockState> {
        self.access.block_state(pos)
    }

    /// Add `delta` to the swept entity's velocity (`Entity.push`). Returns `false` if any
    /// component is non-finite.
    pub fn push(&mut self, delta: [f64; 3]) -> bool {
        self.access.push(delta)
    }

    /// Replace the swept entity's velocity (`Entity.setDeltaMovement`). Returns `false` if
    /// any component is non-finite.
    pub fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        self.access.set_velocity(velocity)
    }

    /// Apply `Entity.makeStuckInBlock` with `speed_multiplier`, resetting fall distance as
    /// vanilla stuck blocks do. Returns `false` if any component is non-finite.
    pub fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool {
        self.access.make_stuck_in_block(speed_multiplier)
    }

    /// Reset the swept entity's accumulated fall distance (`Entity.resetFallDistance`).
    pub fn reset_fall_distance(&mut self) {
        self.access.reset_fall_distance();
    }

    /// Set remaining fire ticks through the swept entity's `setRemainingFireTicks`.
    /// An invulnerable player keeps at most one positive tick.
    pub fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        self.access.set_remaining_fire_ticks(ticks);
    }

    /// Apply `LivingEntity.addEffect` now. Return whether the active effect changed;
    /// a non-living entity, an immune entity, or an unchanged effect returns `false`.
    pub fn add_effect(&mut self, effect: EffectKey, duration: i32, amplifier: u8) -> bool {
        self.access.add_effect(effect, duration, amplifier)
    }

    /// Hurt the swept entity with `damage_type` and `amount` (`Entity.hurt`), with no
    /// direct or causing entity source.
    pub fn hurt(&mut self, damage_type: &str, amount: f32) -> bool {
        self.hurt_with_source(damage_type, amount, None, None)
    }

    /// Hurt the swept entity with a registered `damage_type`, `amount`, and optional
    /// direct (such as a projectile) and causing (such as its shooter) entities.
    /// Explicit handles are resolved in the swept entity's level when called, including
    /// the swept entity itself. An unresolvable handle refuses this hit and returns `false`;
    /// it is not treated as an absent source.
    pub fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<EntityRef>,
        causing: Option<EntityRef>,
    ) -> bool {
        self.access
            .hurt_with_source(damage_type, amount, direct, causing)
    }

    /// Replace the block at `pos` through `Level.setBlockAndUpdate`. Returns `false`
    /// for an unchanged state or a refused write.
    pub fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool {
        self.access.set_block(pos, state)
    }

    /// Destroy the block at `pos` through `Level.destroyBlock`, preserving any
    /// fluid it held. Returns whether a block broke; `drop` requests its loot.
    pub fn destroy_block(&mut self, pos: [i32; 3], drop: bool) -> bool {
        self.access.destroy_block(pos, drop)
    }
}

/// Engine backing for [`InsideEntityView`] and queued [`InsideEffect`] flushing.
// hidden: engine backend behind InsideEntityView; mods call InsideEntityView methods.
#[doc(hidden)]
pub trait InsideEntityAccess {
    fn id(&self) -> u32;
    fn kind(&self) -> &str;
    fn is_alive(&self) -> bool;
    fn position(&self) -> [f64; 3];
    fn velocity(&self) -> [f64; 3];
    fn fire_ticks(&self) -> i32;
    fn is_on_fire(&self) -> bool;
    fn fall_distance(&self) -> f64;
    fn block_state(&self, pos: [i32; 3]) -> Option<BlockState>;
    fn push(&mut self, delta: [f64; 3]) -> bool;
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool;
    fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool;
    fn reset_fall_distance(&mut self);
    fn set_remaining_fire_ticks(&mut self, ticks: i32);
    fn add_effect(&mut self, effect: EffectKey, duration: i32, amplifier: u8) -> bool;
    fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<EntityRef>,
        causing: Option<EntityRef>,
    ) -> bool;
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool;
    fn destroy_block(&mut self, pos: [i32; 3], drop: bool) -> bool;
    fn apply_effect(&mut self, effect: InsideEffect);
    fn apply_builtin(&mut self, _action: BuiltinInsideAction) {}
}

/// Engine-internal vanilla `runBefore` / `runAfter` action queued by built-in
/// blocks (`PowderSnowBlock`, `LayeredCauldronBlock`, `BaseFireBlock`) so they
/// share the same step plan, callback ordering and alive check as mod callbacks.
// hidden: engine-internal built-in action for vanilla inside-block steps.
// no wire: engine-internal flush action, never crosses the mod wire boundary.
// exhaustive: closed set of vanilla built-in inside-block actions matched by the engine.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuiltinInsideAction {
    /// `PowderSnowBlock.entityInside` `runBefore(EXTINGUISH)`: melt powder snow if burning.
    PowderSnowMelt([i32; 3]),
    /// `LayeredCauldronBlock.entityInside` `runBefore(EXTINGUISH)`: lower a water or powder-snow
    /// cauldron if burning.
    LayeredCauldronExtinguish { pos: [i32; 3], state: u32 },
    /// `BaseFireBlock.entityInside` `runAfter(FIRE_IGNITE)`: deal `minecraft:in_fire` damage.
    FireHurt(f32),
    /// `LavaFluid.entityInside` and `LavaCauldronBlock.entityInside`: run `Entity.lavaHurt` after ignition.
    LavaHurt,
}

/// Vanilla `InsideBlockEffectType`: what a block does to the entity inside it, beyond
/// the writes it makes directly. The collector flushes them per step in this order.
///
/// The effects apply to the swept entity inside its own tick: `Freeze` is
/// `setIsInPowderSnow` plus a frozen tick, `ClearFreeze` thaws, `FireIgnite` is
/// `BaseFireBlock.fireIgnite` (with the level's random draw for a player), `LavaIgnite`
/// ignites for 15 seconds, `Extinguish` is `clearFire`.
// exhaustive: the closed serialized effect vocabulary and case order are a link contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum InsideEffect {
    /// `InsideBlockEffectType.FREEZE`: powder snow's frost, one frozen tick a step.
    Freeze,
    /// `InsideBlockEffectType.CLEAR_FREEZE`: fire and lava thaw what powder snow froze.
    ClearFreeze,
    /// `InsideBlockEffectType.FIRE_IGNITE`: `BaseFireBlock.fireIgnite`, 8 seconds.
    FireIgnite,
    /// `InsideBlockEffectType.LAVA_IGNITE`: `Entity.lavaIgnite`, 15 seconds.
    LavaIgnite,
    /// `InsideBlockEffectType.EXTINGUISH`: water and powder snow put the fire out.
    Extinguish,
}

/// The order `StepBasedCollector` flushes a step's effects in, `InsideBlockEffectType.values()`.
const EFFECT_ORDER: [InsideEffect; 5] = [
    InsideEffect::Freeze,
    InsideEffect::ClearFreeze,
    InsideEffect::FireIgnite,
    InsideEffect::LavaIgnite,
    InsideEffect::Extinguish,
];

/// A typed write to a stepped-on entity (`BlockPart::step_on`).
// exhaustive: the closed serialized write vocabulary and case order are a link contract.
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
pub enum EntityWrite {
    /// Add to the entity's movement (`entity.push`).
    Push([f64; 3]),
    /// Replace the entity's movement (`setDeltaMovement`).
    SetMovement([f64; 3]),
    /// `makeStuckInBlock`: scale the entity's movement by these factors until it leaves.
    /// Resets its fall distance, as every stuck block does.
    StuckIn([f64; 3]),
    /// Hurt the contacted entity without a causing or direct entity. A missing type in
    /// the current level does not apply damage; it never becomes generic damage.
    Hurt {
        /// The type requested at registration (`Registrar::damage_type_key`).
        damage_type: DamageTypeKey,
        /// The damage dealt.
        amount: f32,
    },
    /// `resetFallDistance`.
    ResetFallDistance,
    /// `setRemainingFireTicks`: fire's and lava's ignition, or water's extinguish (a
    /// negative or zero value).
    SetFireTicks(i32),
    /// `LivingEntity.addEffect(MobEffectInstance(effect, duration, amplifier))`. Applies
    /// when the swept entity is living; a kind that takes no effects skips it.
    AddEffect {
        /// The effect to add, a key requested once at registration.
        effect: EffectKey,
        /// How long the effect lasts, in ticks.
        duration: i32,
        /// The effect's amplifier, 0 for the plain effect.
        amplifier: u8,
    },
}

#[derive(Clone, Copy)]
struct QueuedCallback {
    owner: Option<&'static str>,
    storage: usize,
    slot: usize,
}

impl QueuedCallback {
    fn invoke(self, call: impl FnOnce()) {
        let _mark = self
            .owner
            .map(|id| crate::runtime::mark_mod(id, "entity_inside"));
        let _ = crate::runtime::invoke(
            crate::HookId::EntityInside,
            "entity_inside",
            self.owner.unwrap_or("unknown"),
            call,
        );
    }

    fn dispose(self, captures: &mut [Box<dyn CaptureStorage>]) {
        self.invoke(|| captures[self.storage].dispose(self.slot));
    }
}

// Type erasure lives at the reusable bucket, not at every closure. Each
// callback enters and leaves its slot independently, including on unwind.
trait CaptureStorage {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn call(&mut self, slot: usize, view: &mut InsideEntityView<'_>);
    fn dispose(&mut self, slot: usize);
    fn clear(&mut self);
}

impl<F: FnMut(&mut InsideEntityView<'_>) + 'static> CaptureStorage for Vec<Option<F>> {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn call(&mut self, slot: usize, view: &mut InsideEntityView<'_>) {
        self[slot].as_mut().expect("queued inside callback")(view);
    }

    fn dispose(&mut self, slot: usize) {
        // Remove the capture first: its destructor is allowed to panic.
        drop(self[slot].take());
    }

    fn clear(&mut self) {
        debug_assert!(self.iter().all(Option::is_none));
        Vec::clear(self);
    }
}

enum Action {
    Effect(InsideEffect),
    Builtin(BuiltinInsideAction),
    Callback(QueuedCallback),
}

impl Action {
    fn dispose(self, captures: &mut [Box<dyn CaptureStorage>]) {
        if let Self::Callback(callback) = self {
            callback.dispose(captures);
        }
    }
}

#[derive(Default)]
struct SweepScratch {
    before: [Vec<Action>; 5],
    after: [Vec<Action>; 5],
    planned: Vec<Action>,
    by_type: std::collections::HashMap<std::any::TypeId, usize>,
    captures: Vec<Box<dyn CaptureStorage>>,
}

impl SweepScratch {
    fn queue<F: FnMut(&mut InsideEntityView<'_>) + 'static>(
        &mut self,
        owner: Option<&'static str>,
        callback: F,
    ) -> Action {
        let storage = *self
            .by_type
            .entry(std::any::TypeId::of::<F>())
            .or_insert_with(|| {
                let index = self.captures.len();
                self.captures.push(Box::<Vec<Option<F>>>::default());
                index
            });
        let slots = self.captures[storage]
            .as_any_mut()
            .downcast_mut::<Vec<Option<F>>>()
            .expect("inside callback bucket matches its type ID");
        let slot = slots.len();
        slots.push(Some(callback));
        Action::Callback(QueuedCallback {
            owner,
            storage,
            slot,
        })
    }

    fn discard(&mut self) {
        for action in self.planned.drain(..).chain(
            self.before
                .iter_mut()
                .chain(&mut self.after)
                .flat_map(|slot| slot.drain(..)),
        ) {
            action.dispose(&mut self.captures);
        }
        for buffer in &mut self.captures {
            buffer.clear();
        }
    }
}

thread_local! {
    // One reusable collector per tick thread, lent exclusively to one sweep.
    // Nested sweeps can take their own storage; no callback runs under the slot's borrow.
    static SPARE_SWEEP: std::cell::RefCell<Option<Box<SweepScratch>>> = const { std::cell::RefCell::new(None) };
}

/// `InsideBlockEffectApplier.StepBasedCollector`: the effects and owned callbacks of one
/// block sweep. The walk advances a step per block it reaches; a step's effects flush
/// once each, in [`InsideEffect`] order, with the callbacks queued via `run_before`
/// before their effect and those queued via `run_after` after it. Steps flush in the
/// order the walk reached them, and `apply_and_clear` runs the ordered plan while the
/// swept entity remains alive, with per-callback panic isolation.
// no wire: holds owned Rust callbacks queued for the current sweep's flush; not a serializable payload.
// exhaustive: the collector's state is private to the crate; a part only calls its methods.
pub struct InsideEffects {
    owner: Option<&'static str>,
    last_step: i32,
    in_step: [bool; 5],
    scratch: Option<Box<SweepScratch>>,
}

impl Default for InsideEffects {
    fn default() -> Self {
        Self {
            owner: None,
            last_step: -1,
            in_step: [false; 5],
            scratch: None,
        }
    }
}

impl Drop for InsideEffects {
    fn drop(&mut self) {
        if let Some(mut scratch) = self.scratch.take() {
            scratch.discard();
            let _ = SPARE_SWEEP.try_with(|slot| slot.replace(Some(scratch)));
        }
    }
}

impl std::fmt::Debug for InsideEffects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsideEffects")
            .field("last_step", &self.last_step)
            .field("in_step", &self.in_step)
            .field(
                "planned_count",
                &self.scratch.as_ref().map_or(0, |s| s.planned.len()),
            )
            .finish()
    }
}

impl InsideEffects {
    fn scratch(&mut self) -> &mut SweepScratch {
        self.scratch
            .get_or_insert_with(|| SPARE_SWEEP.with(|slot| slot.take()).unwrap_or_default())
    }

    /// Checkpoint the current step. Only an answered listener keeps its effects and
    /// captures; a listener with no writes never checks out scratch just to stage them.
    // hidden: engine-internal per-listener collector checkpoint.
    #[doc(hidden)]
    pub fn stage(&mut self, owner: &'static str, listen: impl FnOnce(&mut Self) -> bool) {
        let in_step = self.in_step;
        let (before, after) = self.scratch.as_ref().map_or(([0; 5], [0; 5]), |s| {
            (
                std::array::from_fn(|i| s.before[i].len()),
                std::array::from_fn(|i| s.after[i].len()),
            )
        });
        let old_owner = self.owner.replace(owner);
        let accepted = listen(self);
        self.owner = old_owner;
        if accepted {
            return;
        }
        self.in_step = in_step;
        if let Some(scratch) = &mut self.scratch {
            for i in 0..5 {
                for action in scratch.before[i]
                    .drain(before[i]..)
                    .chain(scratch.after[i].drain(after[i]..))
                {
                    action.dispose(&mut scratch.captures);
                }
            }
        }
    }

    /// `advanceStep`: flush the current step when the walk reaches a new one. The walk
    /// calls this before a block's effects, as vanilla does.
    pub fn advance_step(&mut self, step: i32) {
        if self.last_step != step {
            self.last_step = step;
            self.flush_step();
        }
    }

    /// `apply`: queue an effect for the current step. A type queued twice in one step
    /// still applies once.
    pub fn apply(&mut self, effect: InsideEffect) {
        self.in_step[effect as usize] = true;
    }

    /// `runBefore`: queue a callback that runs just before `effect` in the flush order.
    /// It runs even when nothing queued `effect` itself that step, as vanilla's do.
    pub fn run_before(
        &mut self,
        effect: InsideEffect,
        callback: impl FnMut(&mut InsideEntityView<'_>) + 'static,
    ) {
        let owner = self.owner;
        let scratch = self.scratch();
        let action = scratch.queue(owner, callback);
        scratch.before[effect as usize].push(action);
    }

    /// `runAfter`: queue a callback that runs just after `effect` in the flush order.
    /// It runs even when nothing queued `effect` itself that step, as vanilla's do.
    pub fn run_after(
        &mut self,
        effect: InsideEffect,
        callback: impl FnMut(&mut InsideEntityView<'_>) + 'static,
    ) {
        let owner = self.owner;
        let scratch = self.scratch();
        let action = scratch.queue(owner, callback);
        scratch.after[effect as usize].push(action);
    }

    /// Queue a vanilla built-in action just before `effect` in the flush order.
    // hidden: engine-internal built-in action queuing for vanilla blocks.
    #[doc(hidden)]
    pub fn run_before_builtin(&mut self, effect: InsideEffect, action: BuiltinInsideAction) {
        self.scratch().before[effect as usize].push(Action::Builtin(action));
    }

    /// Queue a vanilla built-in action just after `effect` in the flush order.
    // hidden: engine-internal built-in action queuing for vanilla blocks.
    #[doc(hidden)]
    pub fn run_after_builtin(&mut self, effect: InsideEffect, action: BuiltinInsideAction) {
        self.scratch().after[effect as usize].push(Action::Builtin(action));
    }

    /// Whether nothing is queued, neither a built-in nor a mod callback.
    pub fn is_empty(&self) -> bool {
        self.in_step == [false; 5]
            && self.scratch.as_ref().is_none_or(|s| {
                s.planned.is_empty() && s.before.iter().chain(&s.after).all(Vec::is_empty)
            })
    }

    /// Whether flushing this collector needs the owning `Game` (mod callbacks or
    /// built-in actions that mutate blocks or dispatch live hurt hooks). Plain
    /// built-in effects (`Freeze`, `ClearFreeze`, `FireIgnite`, `LavaIgnite`,
    /// `Extinguish`) only mutate the mob's own fields and run inline without
    /// pausing the mob tick pass.
    // hidden: engine-internal check for borrowed mob tick fast path.
    #[doc(hidden)]
    pub fn needs_live_owner(&self) -> bool {
        self.scratch.as_ref().is_some_and(|s| {
            let needs = |action: &Action| !matches!(action, Action::Effect(_));
            s.planned.iter().any(needs)
                || s.before
                    .iter()
                    .chain(&s.after)
                    .any(|list| list.iter().any(needs))
        })
    }

    /// `applyAndClear`: flush the last step and run every queued effect and callback in
    /// order while the swept entity remains alive, isolating panics per callback owner.
    // hidden: engine sweep flush matching vanilla StepBasedCollector.applyAndClear.
    #[doc(hidden)]
    pub fn apply_and_clear(&mut self, access: &mut dyn InsideEntityAccess) {
        self.flush_step();
        self.last_step = -1;
        let Some(scratch) = &mut self.scratch else {
            return;
        };
        // Leave unconsumed actions in the plan if engine work unwinds. Drop
        // still needs their ownership records to fence capture destructors.
        scratch.planned.reverse();
        while let Some(action) = scratch.planned.pop() {
            if !access.is_alive() {
                action.dispose(&mut scratch.captures);
                continue;
            }
            match action {
                Action::Effect(effect) => access.apply_effect(effect),
                Action::Builtin(builtin) => access.apply_builtin(builtin),
                Action::Callback(queued) => {
                    if !queued
                        .owner
                        .is_some_and(|id| crate::runtime::poisoned(crate::HookId::EntityInside, id))
                    {
                        // The capture stays in its slot across a body unwind. Its separate
                        // destruction cannot cause a second panic during that unwind.
                        queued.invoke(|| {
                            scratch.captures[queued.storage]
                                .call(queued.slot, &mut InsideEntityView::new(access))
                        });
                    }
                    queued.dispose(&mut scratch.captures);
                }
            }
        }
        for buffer in &mut scratch.captures {
            buffer.clear();
        }
    }

    /// Flush the step in effect order: before the effect, the effect, then after.
    fn flush_step(&mut self) {
        let in_step = std::mem::take(&mut self.in_step);
        if self.scratch.is_none() && in_step == [false; 5] {
            return;
        }
        let scratch = self.scratch();
        for (index, effect) in EFFECT_ORDER.into_iter().enumerate() {
            scratch.planned.append(&mut scratch.before[index]);
            if in_step[index] {
                scratch.planned.push(Action::Effect(effect));
            }
            scratch.planned.append(&mut scratch.after[index]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingAccess {
        alive: bool,
        log: Vec<String>,
    }

    impl RecordingAccess {
        fn new() -> Self {
            Self {
                alive: true,
                log: Vec::new(),
            }
        }
    }

    impl InsideEntityAccess for RecordingAccess {
        fn id(&self) -> u32 {
            7
        }
        fn kind(&self) -> &str {
            "minecraft:zombie"
        }
        fn is_alive(&self) -> bool {
            self.alive
        }
        fn position(&self) -> [f64; 3] {
            [0.0, 64.0, 0.0]
        }
        fn velocity(&self) -> [f64; 3] {
            [0.0, 0.0, 0.0]
        }
        fn fire_ticks(&self) -> i32 {
            0
        }
        fn is_on_fire(&self) -> bool {
            false
        }
        fn fall_distance(&self) -> f64 {
            0.0
        }
        fn block_state(&self, _: [i32; 3]) -> Option<BlockState> {
            None
        }
        fn push(&mut self, delta: [f64; 3]) -> bool {
            self.log.push(format!("push:{}", delta[0]));
            true
        }
        fn set_velocity(&mut self, _: [f64; 3]) -> bool {
            true
        }
        fn make_stuck_in_block(&mut self, _: [f64; 3]) -> bool {
            self.log.push("stuck".to_owned());
            true
        }
        fn reset_fall_distance(&mut self) {
            self.log.push("reset_fall".to_owned());
        }
        fn set_remaining_fire_ticks(&mut self, ticks: i32) {
            self.log.push(format!("fire:{ticks}"));
        }
        fn add_effect(&mut self, _: EffectKey, duration: i32, amplifier: u8) -> bool {
            self.log.push(format!("effect:{duration}:{amplifier}"));
            true
        }
        fn hurt_with_source(
            &mut self,
            damage_type: &str,
            amount: f32,
            _: Option<EntityRef>,
            _: Option<EntityRef>,
        ) -> bool {
            self.log.push(format!("hurt:{damage_type}:{amount}"));
            if amount >= 100.0 {
                self.alive = false;
            }
            true
        }
        fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool {
            if state.ordinal() == u16::MAX {
                self.log.push("set_block:refused".to_owned());
                return false;
            }
            self.log.push(format!("set_block:ok:{}", pos[0]));
            true
        }
        fn destroy_block(&mut self, _: [i32; 3], _: bool) -> bool {
            true
        }
        fn apply_effect(&mut self, effect: InsideEffect) {
            self.log.push(format!("builtin:{effect:?}"));
        }
        fn apply_builtin(&mut self, action: BuiltinInsideAction) {
            self.log.push(format!("action:{action:?}"));
        }
    }

    /// `StepBasedCollector` flushes each step once, in `InsideBlockEffectType` order,
    /// with `run_before` callbacks before their effect and `run_after` callbacks after it;
    /// duplicate `apply` calls within a step deduplicate, `run_before`/`run_after` still
    /// run when `apply` was not called for that effect slot, and steps flush across
    /// `advance_step` boundaries in walk order.
    #[test]
    fn steps_flush_in_vanilla_order() {
        let mut effects = InsideEffects::default();
        effects.advance_step(0);
        // Queue out of EFFECT_ORDER to prove flush_step orders by EFFECT_ORDER.
        effects.run_after(InsideEffect::FireIgnite, |view| {
            view.hurt("minecraft:in_fire", 1.0);
        });
        effects.apply(InsideEffect::FireIgnite);
        effects.run_before(InsideEffect::FireIgnite, |view| {
            view.push([10.0, 0.0, 0.0]);
        });
        // ClearFreeze is never applied in step 0, yet both run_before and run_after still run.
        effects.run_after(InsideEffect::ClearFreeze, |view| {
            view.set_remaining_fire_ticks(20);
        });
        effects.run_before(InsideEffect::ClearFreeze, |view| {
            view.reset_fall_distance();
        });
        // Duplicate Freeze in step 0 deduplicates to one built-in Freeze between before and after.
        effects.apply(InsideEffect::Freeze);
        effects.apply(InsideEffect::Freeze);
        effects.run_after(InsideEffect::Freeze, |view| {
            view.make_stuck_in_block([0.25, 0.05, 0.25]);
        });
        effects.run_before(InsideEffect::Freeze, |view| {
            view.push([1.0, 0.0, 0.0]);
        });

        effects.advance_step(2);
        effects.apply(InsideEffect::Extinguish);
        effects.apply(InsideEffect::Extinguish);
        // LavaIgnite is not applied in step 2, yet its run_after still runs before Extinguish.
        effects.run_after(InsideEffect::LavaIgnite, |view| {
            view.push([20.0, 0.0, 0.0]);
        });
        // Same effect type in a later step runs again in that step.
        effects.apply(InsideEffect::Freeze);

        let mut access = RecordingAccess::new();
        effects.apply_and_clear(&mut access);
        assert_eq!(
            access.log,
            vec![
                "push:1",
                "builtin:Freeze",
                "stuck",
                "reset_fall",
                "fire:20",
                "push:10",
                "builtin:FireIgnite",
                "hurt:minecraft:in_fire:1",
                "builtin:Freeze",
                "push:20",
                "builtin:Extinguish",
            ]
        );
        assert!(effects.is_empty(), "applyAndClear empties the collector");
    }

    /// Once a callback makes `access.is_alive()` become `false`, all subsequent callbacks
    /// and effects in the same step and in later steps stop immediately.
    #[test]
    fn death_stops_remaining_callbacks_and_effects() {
        let mut effects = InsideEffects::default();
        effects.advance_step(0);
        effects.run_before(InsideEffect::Freeze, |view| {
            view.push([1.0, 0.0, 0.0]);
        });
        effects.run_before(InsideEffect::Freeze, |view| {
            view.hurt("minecraft:generic", 100.0);
        });
        effects.run_before(InsideEffect::Freeze, |view| {
            view.push([2.0, 0.0, 0.0]);
        });
        effects.apply(InsideEffect::Freeze);
        effects.run_after(InsideEffect::Freeze, |view| {
            view.push([3.0, 0.0, 0.0]);
        });
        effects.apply(InsideEffect::FireIgnite);

        effects.advance_step(1);
        effects.run_before(InsideEffect::Extinguish, |view| {
            view.push([4.0, 0.0, 0.0]);
        });
        effects.apply(InsideEffect::Extinguish);
        effects.run_after(InsideEffect::Extinguish, |view| {
            view.push([5.0, 0.0, 0.0]);
        });

        let mut access = RecordingAccess::new();
        effects.apply_and_clear(&mut access);
        assert_eq!(access.log, vec!["push:1", "hurt:minecraft:generic:100"]);
        assert!(effects.is_empty());
    }

    #[test]
    fn successive_builtin_only_sweeps_do_not_replay_old_actions() {
        let mut effects = InsideEffects::default();
        let mut access = RecordingAccess::new();
        for _ in 0..2 {
            effects.advance_step(0);
            effects.apply(InsideEffect::Freeze);
            effects.run_after_builtin(InsideEffect::FireIgnite, BuiltinInsideAction::FireHurt(1.0));
            effects.apply(InsideEffect::Extinguish);
            effects.apply_and_clear(&mut access);
        }
        assert_eq!(
            access.log,
            [
                "builtin:Freeze",
                "action:FireHurt(1.0)",
                "builtin:Extinguish",
                "builtin:Freeze",
                "action:FireHurt(1.0)",
                "builtin:Extinguish"
            ]
        );
    }

    /// A panicking callback disables only its owning mod for `entity_inside`, keeps earlier
    /// writes, lets other mods' callbacks and built-in effects run, queues a panic notice
    /// naming the panicking mod, and non-panic refusals (`set_block` returning `false`)
    /// report `false` every time without latching or disabling subsequent calls.
    #[test]
    fn panicking_callback_disables_only_owning_mod_and_keeps_prior_writes() {
        struct Quiet;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Quiet {}
        crate::testing::with_dispatch(
            vec![
                crate::ModEntry::new("mod-a", Box::new(Quiet)),
                crate::ModEntry::new("mod-b", Box::new(Quiet)),
            ]
            .into(),
            || {
                let valid = crate::__private::block_state_from_ordinal(crate::BlockKey(1), 0);
                let invalid =
                    crate::__private::block_state_from_ordinal(crate::BlockKey(1), u16::MAX);

                let mut effects = InsideEffects::default();
                effects.advance_step(0);
                // Explicit owner from staging works without a crash instrumentation backend.
                let depth = std::rc::Rc::new(std::cell::Cell::new(0));
                effects.stage("mod-a", |staged_a| {
                    staged_a.run_before(InsideEffect::Freeze, |view| {
                        view.push([1.0, 0.0, 0.0]);
                    });
                    let observed_depth = depth.clone();
                    staged_a.run_after(InsideEffect::Freeze, move |view| {
                        observed_depth.set(crate::__private::hook_dispatch_depth());
                        view.push([4.0, 0.0, 0.0]);
                        panic!("mod-a callback boom");
                    });
                    staged_a.run_after(InsideEffect::FireIgnite, |view| {
                        view.push([2.0, 0.0, 0.0]);
                    });
                    true
                });

                effects.stage("mod-b", |staged_b| {
                    staged_b.run_after(InsideEffect::FireIgnite, move |view| {
                        assert!(!view.set_block([0, 64, 0], invalid));
                        assert!(
                            !view.set_block([0, 64, 0], invalid),
                            "non-panic refusal returns false every time without latching"
                        );
                        assert!(
                            view.set_block([0, 64, 0], valid),
                            "subsequent valid call succeeds after refusals"
                        );
                        view.push([3.0, 0.0, 0.0]);
                    });
                    true
                });
                effects.apply(InsideEffect::Freeze);
                effects.apply(InsideEffect::FireIgnite);

                let mut access = RecordingAccess::new();
                effects.apply_and_clear(&mut access);
                assert_eq!(
                    access.log,
                    vec![
                        "push:1",
                        "builtin:Freeze",
                        "push:4",
                        "builtin:FireIgnite",
                        "set_block:refused",
                        "set_block:refused",
                        "set_block:ok:0",
                        "push:3",
                    ],
                    "mod-a's completed writes stay, its third callback is skipped, and the built-in and mod-b continue"
                );
                assert_eq!(depth.get(), 1, "flush callback uses the shared boundary");
                assert!(crate::runtime::poisoned(
                    crate::HookId::EntityInside,
                    "mod-a"
                ));
                assert!(!crate::runtime::poisoned(
                    crate::HookId::EntityInside,
                    "mod-b"
                ));
                assert_eq!(
                    crate::__private::take_hook_panics(),
                    [crate::__private::HookPanicReport {
                        mod_id: "mod-a",
                        hook: "entity_inside",
                    }]
                );
            },
        );
    }

    /// Dropping unexecuted callbacks with panicking capture destructors (on death-discard,
    /// poisoned-owner skip, or rejected staging) is contained and attributed to the explicit owner.
    #[test]
    fn unexecuted_callback_with_panicking_drop_is_contained() {
        struct PanicOnDrop(&'static str);
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("{}", self.0);
            }
        }
        struct Quiet;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Quiet {}
        crate::testing::with_dispatch(
            vec![
                crate::ModEntry::new("drop-death", Box::new(Quiet)),
                crate::ModEntry::new("drop-staged", Box::new(Quiet)),
            ]
            .into(),
            || {
                // Case 1: death-discard drops remaining queued callback whose capture panics on Drop.
                let mut effects = InsideEffects::default();
                effects.advance_step(0);
                effects.stage("drop-death", |staged| {
                    staged.run_before(InsideEffect::Freeze, |view| {
                        view.hurt("minecraft:generic", 100.0);
                    });
                    let bomb = PanicOnDrop("death-drop boom");
                    staged.run_after(InsideEffect::Freeze, move |view| {
                        let _keep = &bomb;
                        view.push([1.0, 0.0, 0.0]);
                    });
                    true
                });
                effects.run_after_builtin(
                    InsideEffect::FireIgnite,
                    BuiltinInsideAction::FireHurt(1.0),
                );

                let mut access = RecordingAccess::new();
                effects.apply_and_clear(&mut access);
                assert_eq!(access.log, vec!["hurt:minecraft:generic:100"]);
                assert!(crate::runtime::poisoned(
                    crate::HookId::EntityInside,
                    "drop-death"
                ));

                // Case 2: rolling back a rejected listener contains panicking Drop.
                effects.stage("drop-staged", |rejected| {
                    let bomb = PanicOnDrop("rejected-stage boom");
                    rejected.run_before(InsideEffect::Extinguish, move |_view| {
                        let _keep = &bomb;
                    });
                    false
                });
                assert!(crate::runtime::poisoned(
                    crate::HookId::EntityInside,
                    "drop-staged"
                ));

                assert_eq!(
                    crate::__private::take_hook_panics(),
                    [
                        crate::__private::HookPanicReport {
                            mod_id: "drop-death",
                            hook: "entity_inside",
                        },
                        crate::__private::HookPanicReport {
                            mod_id: "drop-staged",
                            hook: "entity_inside",
                        },
                    ]
                );
            },
        );
    }

    /// A callback whose body panics AND whose captured value panics on Drop must not abort
    /// the process: the body panic unwinds without dropping captures, then capture disposal
    /// runs through the shared containment boundary while other owners continue.
    #[test]
    fn body_panic_and_capture_drop_panic_do_not_abort_or_stop_other_owners() {
        struct PanicOnDrop(&'static str);
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("{}", self.0);
            }
        }
        struct Quiet;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Quiet {}
        crate::testing::with_dispatch(
            vec![
                crate::ModEntry::new("double-boom", Box::new(Quiet)),
                crate::ModEntry::new("survivor", Box::new(Quiet)),
            ]
            .into(),
            || {
                let mut effects = InsideEffects::default();
                effects.advance_step(0);
                effects.stage("double-boom", |bad| {
                    let bomb = PanicOnDrop("capture-drop boom");
                    bad.run_before(InsideEffect::Freeze, move |_view| {
                        let _keep = &bomb;
                        panic!("body boom");
                    });
                    true
                });
                effects.stage("survivor", |good| {
                    good.run_after(InsideEffect::Freeze, |view| {
                        view.push([2.0, 0.0, 0.0]);
                    });
                    true
                });

                let mut access = RecordingAccess::new();
                effects.apply_and_clear(&mut access);
                assert_eq!(access.log, vec!["push:2"]);
                assert!(crate::runtime::poisoned(
                    crate::HookId::EntityInside,
                    "double-boom"
                ));
                assert!(!crate::runtime::poisoned(
                    crate::HookId::EntityInside,
                    "survivor"
                ));
            },
        );
    }

    #[test]
    fn needs_live_owner_is_false_for_plain_effects_and_true_for_builtins_or_callbacks() {
        let mut water_only = InsideEffects::default();
        water_only.advance_step(0);
        water_only.apply(InsideEffect::Extinguish);
        water_only.advance_step(1);
        water_only.apply(InsideEffect::ClearFreeze);
        assert!(!water_only.is_empty());
        assert!(!water_only.needs_live_owner());

        let mut with_builtin = InsideEffects::default();
        with_builtin.advance_step(0);
        with_builtin.apply(InsideEffect::FireIgnite);
        with_builtin
            .run_after_builtin(InsideEffect::FireIgnite, BuiltinInsideAction::FireHurt(1.0));
        assert!(with_builtin.needs_live_owner());

        let mut with_callback = InsideEffects::default();
        with_callback.advance_step(0);
        with_callback.run_after(InsideEffect::Extinguish, |_| {});
        assert!(with_callback.needs_live_owner());
    }
}
