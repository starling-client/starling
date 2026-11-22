//! `LivingEntity.startUsingItem` and the use over time of items vanilla's
//! `Item` subclasses don't know: what `Item.getUseDuration` answers, a
//! native mod's own item counted down to its `finishUsingItem`, and the hand
//! pose of a `getUseAnimation` (`native_use_duration`, `native_use_animation`,
//! `native_living_entity_use_item_start`, `native_living_entity_use_item_finish`),
//! and each tick and the release of a use (`native_living_entity_use_item_tick`,
//! `native_on_use_tick`, `native_living_entity_use_item_stop`, `native_release_using`).
use super::{Game, shield::OFFHAND_SLOT};
use sunrise_client::{
    first_person::Hand,
    items::{ItemInfo, ItemStack},
    mobs::spear_use,
};
use sunrise_core::use_animation::UseAnimation;

#[derive(Clone, Copy)]
pub(super) enum ItemUseState {
    Spear(sunrise_client::spear::UseTiming),
    Other(UseAnimation),
}

impl ItemUseState {
    pub(super) fn animation(self) -> UseAnimation {
        match self {
            Self::Spear(_) => UseAnimation::Spear,
            Self::Other(animation) => animation,
        }
    }
}

/// Resolve USE_EFFECTS through the stack's component patch.
pub(super) fn interact_vibrations(stack: &ItemStack, default: Option<bool>) -> bool {
    if stack.count == 0 {
        return false;
    }
    let Some(data) = stack.data() else {
        return default.unwrap_or(false);
    };
    if data.other("!minecraft:use_effects").is_some() {
        return false;
    }
    data.other("minecraft:use_effects").map_or_else(
        || default.unwrap_or(false),
        |tag| sunrise_client::save::number(tag.get("interact_vibrations")).unwrap_or(1) != 0,
    )
}

impl Game<'_> {
    /// Resolve an item's animation once for the caller's rendering decision.
    pub(super) fn item_use_animation(&self, stack: &ItemStack) -> UseAnimation {
        self.item_use_state(stack).animation()
    }

    /// Select the animation and its already decoded spear timing together,
    /// before firing the scoped override. No stack view is built per frame.
    pub(super) fn item_use_state(&self, stack: &ItemStack) -> ItemUseState {
        let info = self.items().get(&stack.item);
        let kinetic =
            sunrise_client::spear::UseTiming::of_stack(stack, info.and_then(|i| i.kinetic_timing));
        let fixed = match &*stack.item {
            "minecraft:bow" => Some(UseAnimation::Bow),
            "minecraft:crossbow" => Some(UseAnimation::Crossbow),
            "minecraft:trident" => Some(UseAnimation::Trident),
            "minecraft:brush" => Some(UseAnimation::Brush),
            "minecraft:spyglass" => Some(UseAnimation::Spyglass),
            "minecraft:goat_horn" => Some(UseAnimation::TootHorn),
            _ if info.is_some_and(|i| i.bundle) => Some(UseAnimation::Bundle),
            _ => None,
        };
        let data = stack.data();
        let mut animation = fixed.unwrap_or_else(|| {
            if let Some(consumable) = self.consumable(stack) {
                consumable.animation
            } else if self.stack_can_perform(
                stack,
                super::item_abilities::ability::SHIELD_BLOCK,
                data.is_some_and(|d| d.other("minecraft:blocks_attacks").is_some())
                    || (data.is_none_or(|d| d.other("!minecraft:blocks_attacks").is_none())
                        && info.is_some_and(|i| i.blocks_attacks.is_some())),
            ) {
                UseAnimation::Block
            } else if kinetic.is_some() {
                UseAnimation::Spear
            } else {
                UseAnimation::None
            }
        });
        self.native_use_animation(stack, &mut animation);
        // A spear pose needs the effective stack's kinetic data, even when a
        // scoped use_animation override requested it.
        if animation == UseAnimation::Spear {
            return kinetic.map_or(ItemUseState::Other(UseAnimation::None), ItemUseState::Spear);
        }
        ItemUseState::Other(animation)
    }

    /// `LivingEntity.startUsingItem` for the acting player's hand: nothing
    /// when a use runs already or the hand is empty. The use lasts
    /// `getUseDuration` ticks, which mods may change or refuse
    /// (`LivingEntityUseItemEvent.Start`), on both sides as in vanilla. True
    /// when the use started.
    pub(super) fn start_using_item(&mut self) -> bool {
        let Some(stack) = self.held().cloned().filter(|_| self.using.is_none()) else {
            return false;
        };
        let own = self.use_duration(&stack);
        let (mut duration, mut canceled) = (own, false);
        let hand = self.hand;
        self.native_living_entity_use_item_start(&stack, None, hand, &mut duration, &mut canceled);
        let Some(ticks_set) = start_of(own, duration, canceled) else {
            return false;
        };
        self.using = Some((self.held_slot(), 0));
        self.use_duration = duration;
        self.use_start = super::using_item::UseStart {
            ticks_set,
            item: Some(stack.clone()),
        };
        // `LivingEntity.startUsingItem`: `causeUseVibration(ITEM_INTERACT_START)`
        // when the use effects of the stack ask for interaction vibrations.
        self.use_vibration(&stack, "minecraft:item_interact_start");
        // A new charge hears the crossbow's loading sounds again
        // (`game/items/crossbow.rs`).
        self.crossbow_sounds = [false; 2];
        true
    }
    /// `ItemStack.getUseDuration`: a consumable's time, 72000 for what is
    /// held until let go, the fixed times of brushes, bundles, spyglasses and
    /// goat horns, 0 for the rest, then a mod's answer. A `consumable` or
    /// `blocks_attacks` the server's patch adds counts as the item's own
    /// (`Item.getUseDuration` reads the stack's components).
    pub(super) fn use_duration(&self, stack: &ItemStack) -> i32 {
        let info = self.items().get(&stack.item);
        let (patched_ticks, patched_blocks) = sunrise_client::item_sync::patched_use(stack);
        let mut duration = match &*stack.item {
            "minecraft:bow" | "minecraft:crossbow" | "minecraft:trident" => 72_000,
            "minecraft:spyglass" => 1200,
            "minecraft:brush" => 200,
            _ if info.is_some_and(|i| i.bundle) => 200,
            "minecraft:ender_eye" => 0,
            "minecraft:goat_horn" => self.native_item_instrument_duration(stack),
            _ => {
                if let Some(ticks) = patched_ticks {
                    ticks
                } else if let Some(consumable) = self.consumable(stack) {
                    consumable.ticks()
                } else if patched_blocks
                    || self.stack_carries_blocks_attacks(stack, info)
                    || stack.carries_component(info, "minecraft:kinetic_weapon")
                {
                    72_000
                } else {
                    0
                }
            }
        };
        self.native_use_duration(stack, &mut duration);
        duration
    }
    /// Whether `stack` is a mod's item that only a mod uses over time: no
    /// `consumable`, `blocks_attacks` or kinetic weapon of vanilla's own, and
    /// a `native_use_duration` above 0. Its use is its own `Item.use`.
    pub(super) fn mod_use(&self, stack: &ItemStack) -> bool {
        let info = self.items().get(&stack.item);
        !stack.item.starts_with("minecraft:")
            && info.is_some()
            && self.consumable(stack).is_none()
            && !self.stack_carries_blocks_attacks(stack, info)
            && !stack.carries_component(info, "minecraft:kinetic_weapon")
            && self.use_duration(stack) > 0
    }
    /// A right click with a `mod_use` item: `startUsingItem` (CONSUME).
    pub(super) fn start_mod_use(&mut self, stack: &ItemStack) -> bool {
        if !self.mod_use(stack) {
            return false;
        }
        self.start_using_item();
        true
    }
    /// `LivingEntity.updateUsingItem` for a `mod_use` item used `ticks` so far:
    /// `completeUsingItem` once its duration ran out.
    pub(super) fn tick_mod_use(&mut self, slot: usize, ticks: u32, stack: &ItemStack) {
        if !self.item_on_use_tick(stack, ticks) {
            return;
        }
        if let Some(ticks) = count_down(ticks, self.use_duration) {
            self.using = Some((slot, ticks));
            return;
        }
        let Some(info) = self.items().get(&stack.item).cloned() else {
            self.using = None;
            return;
        };
        self.complete_using_item(slot, stack, &info);
    }
    /// `ServerPlayer.completeUsingItem` of `stack` in `slot`: the client is
    /// told, `ItemStack.finishUsingItem` (a consumable is eaten, a potion
    /// drunk, a spyglass lowered, anything else given back) with its use
    /// cooldown, then `LivingEntityUseItemEvent.Finish`.
    pub(super) fn complete_using_item(&mut self, slot: usize, stack: &ItemStack, info: &ItemInfo) {
        // a potion is drunk (no last tick of its own: a mod's tick ran it out).
        if self.finish_drinking(slot, stack) {
            self.use_item_finished(stack);
            self.use_vibration(stack, "minecraft:item_interact_finish");
            return;
        }
        self.using = None;
        self.send_use_item_complete();
        self.apply_use_cooldown(stack);
        if let Some(consumable) = self.consumable(stack) {
            self.finish_consuming(info, &consumable, stack);
        } else if &*stack.item == "minecraft:spyglass" {
            self.stop_spyglass();
        }
        self.use_item_finished(stack);
        // `LivingEntity.completeUsingItem` ends with `stopUsingItem`.
        self.use_vibration(stack, "minecraft:item_interact_finish");
    }
    /// `LivingEntityUseItemEvent.Finish` after `finishUsingItem` of `stack`
    /// (no ticks left): the hand holds what a mod made the result.
    pub(super) fn use_item_finished(&mut self, stack: &ItemStack) {
        let mut result = self.held().cloned();
        let before = result.clone();
        let hand = self.hand;
        self.native_living_entity_use_item_finish(stack, None, hand, 0, &mut result);
        if result != before {
            *self.held_mut() = result.filter(|s| s.count > 0);
        }
    }
    /// `LivingEntityUseItemEvent.Finish` on a server's client, whose
    /// `completeUsingItem` (`handleEntityEvent(9)`) ran for `stack` in `slot`.
    /// `ItemStack.finishUsingItem` runs first, as on the client: the use
    /// cooldown, then the hand holds `finish_result`. A mod's result is put in
    /// the slot; the server's inventory update replaces either. Finishing a use
    /// sends no creative packet in vanilla (its inventory-slot edits go out
    /// through the open creative screen's `CreativeInventoryListener`), so the
    /// slot counts as synced for this client's creative sync.
    pub(super) fn remote_use_item_finished(
        &mut self,
        slot: usize,
        stack: &ItemStack,
        info: &ItemInfo,
    ) {
        self.apply_use_cooldown(stack);
        let mut result = self.finish_result(stack, info);
        let hand = hand_of(Some((slot, 0))).unwrap_or(self.hand);
        self.native_living_entity_use_item_finish(stack, None, hand, 0, &mut result);
        let before = self.creative_stacks();
        if !self.creative && stack.count == 1 && self.consumable(stack).is_some() {
            if self.stack_in(slot) != Some(stack) {
                return;
            }
            *self.stack_in_mut(slot) = None;
            self.player_item_destroyed(stack, Some(hand));
        }
        *self.stack_in_mut(slot) = result.filter(|s| s.count > 0);
        self.remote_synced_since(&before);
    }
    /// What `Item.finishUsingItem` leaves in the hand on a server's client: a
    /// stack that is not a consumable stays as it is (a spyglass); a
    /// consumable loses one (`ItemStack.consume`), and once that empties the
    /// hand, its `use_remainder` takes the hand
    /// (`UseRemainder.convertIntoRemainder`). Neither in creative mode. A
    /// remainder of a stack that is not emptied goes to the inventory on the
    /// server and arrives with its inventory update.
    pub(super) fn finish_result(&self, stack: &ItemStack, info: &ItemInfo) -> Option<ItemStack> {
        if self.creative || self.consumable(stack).is_none() {
            return Some(stack.clone());
        }
        let mut left = stack.clone();
        left.count = left.count.saturating_sub(1);
        if left.count > 0 {
            return Some(left);
        }
        sunrise_client::item_sync::use_remainder(stack, Some(info))
    }
    /// `LivingEntityUseItemEvent.Tick` for `stack` in use `ticks` so far, as
    /// `updatingUsingItem` starts: a mod may change the ticks left, which
    /// moves the end. False when the remaining counter is nonpositive:
    /// effects are skipped, but the countdown still decrements.
    pub(super) fn use_item_ticked(&mut self, stack: &ItemStack, ticks: u32) -> bool {
        let before = ticks_left(self.use_duration, ticks);
        let (mut remaining, mut canceled) = (before, false);
        let hand = self.used_hand().unwrap_or(self.hand);
        self.native_living_entity_use_item_tick(stack, None, hand, &mut remaining, &mut canceled);
        let (left, set) = tick_left(before, remaining, canceled, self.use_start.ticks_set);
        self.use_start.ticks_set = set;
        self.use_duration = duration_leaving(ticks, left);
        left > 0
    }
    /// A mod's `Item.onUseTick` of `stack` used `ticks`, while ticks are left.
    /// False when that mod released the use, which then has nothing left to run.
    pub(super) fn item_on_use_tick(&mut self, stack: &ItemStack, ticks: u32) -> bool {
        let using = self.using;
        let remaining = ticks_left(self.use_duration, ticks);
        if remaining > 0 {
            self.native_on_use_tick(stack, None, remaining);
        }
        self.using == using
    }
    /// How long `stack`, used `ticks`, counts as held: `getUseDuration` less
    /// the ticks left (`BowItem.releaseUsing`, `CrossbowItem.onUseTick`,
    /// `TridentItem.releaseUsing`), `ticks` unless a mod's Start or Tick
    /// moved the ticks left.
    pub(super) fn ticks_held(&self, stack: &ItemStack, ticks: u32) -> u32 {
        held(
            self.use_duration(stack),
            ticks_left(self.use_duration, ticks),
        )
    }
    /// `LivingEntityUseItemEvent.Stop` as `releaseUsingItem` lets go of
    /// `stack` used `ticks`: false when a mod skips its `releaseUsing`.
    pub(super) fn use_item_stopped(&mut self, stack: &ItemStack, ticks: u32) -> bool {
        let mut canceled = false;
        let remaining = ticks_left(self.use_duration, ticks);
        let hand = self.hand;
        self.native_living_entity_use_item_stop(stack, None, hand, remaining, &mut canceled);
        !canceled
    }
    /// A mod's `Item.releaseUsing` of `stack` used `ticks`: when it says it
    /// was used, `ItemStack.releaseUsing` applies the use cooldown.
    pub(super) fn mod_release_using(&mut self, stack: &ItemStack, ticks: u32) {
        let mut used = false;
        let remaining = ticks_left(self.use_duration, ticks);
        self.native_release_using(stack, None, remaining, &mut used);
        if used {
            self.apply_use_cooldown(stack);
        }
    }
    /// Whether a use in `slot` lost its item: switching hotbar slots only ends
    /// a main-hand use.
    pub(super) fn use_hand_changed(&self, slot: usize) -> bool {
        slot != self.selected && slot != OFFHAND_SLOT
    }
    /// `LivingEntity.releaseUsingItem` of the acting player, the hand of the
    /// item in use acting: the use is over before any hook runs, so a mod
    /// that releases again from one finds nothing. False when none runs.
    pub(super) fn release_using_item(&mut self) -> bool {
        let Some(hand) = self.used_hand() else {
            return false;
        };
        self.with_hand(hand, |game| {
            let Some((slot, ticks)) = game.using.take() else {
                return false;
            };
            let Some(stack) = game.stack_in(slot).cloned() else {
                game.use_start.item = None;
                return true;
            };
            // `releaseUsing` only for the same item; `stopUsingItem` always,
            // from the stack the use began with.
            let (stack, hand_changed) = match game.use_start.item.take() {
                Some(item) if item.item != stack.item => (item, true),
                _ => (stack, game.use_hand_changed(slot)),
            };
            let player = (!mod_api::__private::mods().is_empty()).then(|| {
                let name = game.acting_player_name().to_owned();
                (game.native_acting_id(), name)
            });
            if let Some((uuid, name)) = &player
                && !hand_changed
                && &*stack.item == "minecraft:bow"
                && let Some(arrow) = game
                    .projectile_slot("minecraft:bow")
                    .and_then(|slot| game.stack_in(slot).map(|s| s.item.clone()))
            {
                game.native_player_ready_arrow(*uuid, name, &arrow);
            }
            game.release_used_item(&stack, ticks, hand_changed);
            if let Some((uuid, name)) = &player {
                game.native_player_stop_using_item(*uuid, name);
            }
            true
        })
    }
    /// The stack in the hand of mob `id`.
    fn mob_hand_stack(&self, id: u32, off_hand: bool) -> Option<ItemStack> {
        use sunrise_items::equipment::{MAINHAND, OFFHAND};
        let slot = if off_hand { OFFHAND } else { MAINHAND };
        let mob = self.mobs.iter().find(|m| m.id == id)?;
        mob.extra.slot_stack(slot).cloned()
    }
    /// `ItemStack.causeUseVibration` for `stack` used by mob `id`.
    fn mob_use_vibration(&mut self, id: u32, stack: &ItemStack, event: &'static str) {
        if !self.use_vibrates(stack) {
            return;
        }
        let Some(p) = self.mobs.iter().find(|m| m.id == id).map(|m| m.position) else {
            return;
        };
        let ctx = self.mob_context(id, None);
        self.post_game_event(event, [p.x, p.y, p.z], ctx);
    }
    /// `LivingEntity.startUsingItem` of mob `id`, `native_living_entity_use_item_start`
    /// as for a player. True when the use started. A mob's use is its
    /// `using_item` counter: its goals run that, and nothing counts a use down
    /// to `finishUsingItem` for it.
    pub(super) fn start_mob_using_item(&mut self, id: u32, hand: Hand) -> bool {
        let off_hand = hand == Hand::Off;
        let free = |m: &sunrise_client::mobs::Mob| {
            !spear_use::is_using_item(m) && !spear_use::counts_own_timer(m)
        };
        if !self.mobs.iter().any(|m| m.id == id && free(m)) {
            return false;
        }
        let Some(stack) = self.mob_hand_stack(id, off_hand) else {
            return false;
        };
        let own = self.use_duration(&stack);
        let (mut duration, mut canceled) = (own, false);
        let entity = Some(id as i32);
        self.native_living_entity_use_item_start(
            &stack,
            entity,
            hand,
            &mut duration,
            &mut canceled,
        );
        if start_of(own, duration, canceled).is_none() {
            return false;
        }
        // The hook may have changed the mob: the same checks again.
        let started = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id)
            .is_some_and(|m| free(m) && spear_use::start_using_item(m, off_hand));
        if started {
            self.mob_use_vibration(id, &stack, "minecraft:item_interact_start");
        }
        started
    }
    /// `LivingEntity.getUseItemRemainingTicks` of mob `id`: the use's duration
    /// less the ticks its counter has run, 0 when it uses nothing (an admiring
    /// piglin and a drinking witch use none). None for a mob that is gone.
    pub(super) fn mob_item_use_remaining(&self, id: u32) -> Option<i32> {
        let mob = self.mobs.iter().find(|m| m.id == id)?;
        if !spear_use::is_using_item(mob) {
            return Some(0);
        }
        let stack = self.mob_hand_stack(id, mob.extra.using_off_hand);
        Some(stack.map_or(0, |stack| {
            ticks_left(
                self.use_duration(&stack),
                mob.extra.using_item.unsigned_abs() - 1,
            )
        }))
    }
    /// `LivingEntity.useItemRemaining` of mob `id` set to `remaining`: its use
    /// counter moves to where that many ticks are left. A mob's use never
    /// runs out, so nothing else changes. False when it uses nothing.
    pub(super) fn mob_set_item_use_remaining(&mut self, id: u32, remaining: i32) -> bool {
        let Some(mob) = self.mobs.iter().find(|m| m.id == id) else {
            return false;
        };
        if !spear_use::is_using_item(mob) {
            return false;
        }
        let Some(stack) = self.mob_hand_stack(id, mob.extra.using_off_hand) else {
            return false;
        };
        let counter = use_counter_leaving(self.use_duration(&stack), remaining);
        if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) {
            mob.extra.using_item = counter;
        }
        true
    }
    /// `LivingEntity.releaseUsingItem` of mob `id`: a mod's `Stop` and
    /// `Item.releaseUsing`, the use over before them as for a player. A bow's
    /// and a trident's own release ask for a player, and a use cooldown too
    /// (`UseCooldown.apply`). False when no use runs.
    pub(super) fn release_mob_using_item(&mut self, id: u32) -> bool {
        let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        if !spear_use::is_using_item(mob) {
            return false;
        }
        let off_hand = mob.extra.using_off_hand;
        let ticks = mob.extra.using_item.unsigned_abs() - 1;
        spear_use::stop_using_item(mob);
        let Some(stack) = self.mob_hand_stack(id, off_hand) else {
            return true;
        };
        let remaining = ticks_left(self.use_duration(&stack), ticks);
        let hand = if off_hand { Hand::Off } else { Hand::Main };
        let entity = Some(id as i32);
        let mut canceled = false;
        self.native_living_entity_use_item_stop(&stack, entity, hand, remaining, &mut canceled);
        if !canceled {
            self.native_release_using(&stack, entity, remaining, &mut false);
        }
        self.mob_use_vibration(id, &stack, "minecraft:item_interact_finish");
        true
    }
    /// `Player.canEat(canAlwaysEat)` for `stack`, food that is eaten only
    /// when hungry, then a mod's answer.
    pub(super) fn can_eat(&self, stack: Option<&ItemStack>, can_always_eat: bool) -> bool {
        let vanilla = can_always_eat
            || self.creative
            || self
                .player
                .as_ref()
                .is_some_and(|p| p.health.food.food < 20);
        if !mod_api::__private::enabled_listened(mod_api::HookId::CanEat) {
            return vanilla;
        }
        // The shared resolver's guards: no fake actor, a placed online guest,
        // the integrated player only on the serving side.
        let player = self.local_player().or_else(|| {
            let uuid = self.lan_acting_uuid.or_else(|| {
                self.level
                    .player_uuid
                    .map(sunrise_client::lan::playerdata::ints_uuid)
            })?;
            self.player(uuid)
        });
        let Some(player) = player else { return vanilla };
        let mut answer = None;
        self.native_can_eat(player, stack, vanilla, &mut answer);
        answer.unwrap_or(vanilla)
    }
}

/// The event's replacement counter (a cancel leaves -1), and whether a
/// mod has changed this use's countdown. This flag preserves vanilla's
/// initially zero-duration items; a mod-run-out use skips effects, but
/// still decrements on both sides.
fn tick_left(before: i32, remaining: i32, canceled: bool, set: bool) -> (i32, bool) {
    let left = if canceled { -1 } else { remaining };
    (left, set || canceled || remaining != before)
}

/// What `releaseUsingItem` runs of a release: the item's own and a mod's
/// `releaseUsing` (not when a mod's `Stop` canceled, nor when the hand
/// changed item, which stops the use without one), and the spyglass's
/// sound (not when canceled).
pub(super) fn release_parts(canceled: bool, hand_changed: bool) -> (bool, bool) {
    (!canceled && !hand_changed, !canceled)
}

pub(super) fn bow_release_power(has_ammo: bool, charge: i32, canceled: bool) -> Option<f32> {
    if canceled || charge < 0 || !has_ammo {
        return None;
    }
    let f = charge as f32 / 20.0;
    let power = ((f * f + f * 2.0) / 3.0).min(1.0);
    (power >= 0.1).then_some(power)
}

/// `useItemRemaining` of a use of `duration` ticks, `ticks` old.
pub(super) fn ticks_left(duration: i32, ticks: u32) -> i32 {
    duration.saturating_sub_unsigned(ticks)
}

/// `LivingEntity.getUsedItemHand` of the use `using` (its slot and ticks so
/// far), none when nothing is in use.
pub(super) fn hand_of(using: Option<(usize, u32)>) -> Option<Hand> {
    using.map(|(slot, _)| {
        if slot == OFFHAND_SLOT {
            Hand::Off
        } else {
            Hand::Main
        }
    })
}

/// `LivingEntity.useItemRemaining` of the use `using` (its slot and ticks so
/// far) of `duration` ticks, 0 when nothing is in use.
pub(super) fn remaining_of(using: Option<(usize, u32)>, duration: i32) -> i32 {
    using.map_or(0, |(_, ticks)| ticks_left(duration, ticks))
}

/// Sets `useItemRemaining` of the use `using` by moving its `duration`, as a
/// mod's tick does; false when nothing is in use.
pub(super) fn set_remaining(
    using: Option<(usize, u32)>,
    duration: &mut i32,
    remaining: i32,
) -> bool {
    let Some((_, ticks)) = using else {
        return false;
    };
    *duration = duration_leaving(ticks, remaining);
    true
}

/// `getUseDuration(stack) - remainingTime` of a use of `own` ticks with
/// `remaining` left, none below 0.
fn held(own: i32, remaining: i32) -> u32 {
    u32::try_from(i64::from(own) - i64::from(remaining)).unwrap_or(0)
}

/// The duration that leaves `remaining` ticks of a use `ticks` old, as a
/// mod's `Tick` set them.
pub(super) fn duration_leaving(ticks: u32, remaining: i32) -> i32 {
    remaining.saturating_add_unsigned(ticks)
}

/// The use counter of a mob (1 on the tick its use starts) that leaves
/// `remaining` ticks of a use of `own`, none past the use's whole duration.
fn use_counter_leaving(own: i32, remaining: i32) -> i32 {
    i32::try_from(held(own, remaining).saturating_add(1)).unwrap_or(i32::MAX)
}

/// What a `Start` left of a use of `own` ticks: None refused (a cancel, or
/// a time below 0), else whether a
/// mod set the ticks left to none. Such a use runs out on its first tick
/// (`tick_left`), while
/// an item's own 0 (an eye of ender) is held until let go, as vanilla's.
fn start_of(own: i32, duration: i32, canceled: bool) -> Option<bool> {
    (!canceled && duration >= 0).then_some(duration == 0 && own != 0)
}

/// `Consumable.startConsuming`: a time of 0 or less is no use, the item is
/// consumed at once (`onConsume`).
pub(super) fn consumes_at_once(consume_ticks: i32) -> bool {
    consume_ticks <= 0
}

/// `updateUsingItem`'s `--useItemRemaining == 0` for a use `ticks` old of
/// `duration` ticks: the ticks used after this one, none on the tick it runs
/// out. A time of 0 or less, or one a mod cut below what was used, never
/// does: the use lasts until let go.
pub(super) fn count_down(ticks: u32, duration: i32) -> Option<u32> {
    let ticks = ticks + 1;
    (i64::from(duration) - i64::from(ticks) != 0).then_some(ticks)
}

/// The first person pose of a hand using an item of `animation`
/// (`ItemInHandRenderer.renderArmWithItem`'s switch), for an item of no
/// pose of its own.
pub(super) fn use_pose(
    animation: UseAnimation,
    remaining: f32,
    duration: f32,
    partial: f32,
) -> sunrise_client::first_person::ItemPose {
    use sunrise_client::first_person::ItemPose;
    let ticks_used = duration - (remaining - partial + 1.0);
    match animation {
        UseAnimation::Eat | UseAnimation::Drink => ItemPose::Eat {
            remaining,
            duration,
            partial,
        },
        UseAnimation::Bow => ItemPose::Bow { ticks_used },
        UseAnimation::Trident => ItemPose::Trident { ticks_used },
        UseAnimation::Brush => ItemPose::Brush { remaining, partial },
        _ => ItemPose::Using,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bow_release_power, consumes_at_once, count_down, duration_leaving, held, release_parts,
        start_of, tick_left, ticks_left, use_counter_leaving, use_pose,
    };
    use sunrise_client::first_person::ItemPose;
    use sunrise_core::use_animation::UseAnimation;

    /// A mob's use counter is 1 on the tick it starts and a bow's use lasts
    /// 72000 ticks: 71995 left is 5 ticks in, counter 6 (what releasing
    /// reports back as 71995), and more than the whole use is no ticks in.
    #[test]
    fn a_mobs_use_counter_leaves_the_remaining_ticks() {
        for (own, remaining, counter) in [
            (72_000, 71_995, 6),
            (72_000, 72_000, 1),
            (72_000, 1, 72_000),
            (72_000, 0, 72_001),
            (32, 40, 1),
            (32, -3, 36),
            (i32::MAX, i32::MIN, i32::MAX),
            // The extremes a mod may set on a 72000 tick use, which
            // `spear_use`'s tick test runs on.
            (72_000, i32::MIN, i32::MAX),
            (72_000, i32::MAX, 1),
            (72_000, -5, 72_006),
            (i32::MIN, i32::MIN, 1),
            (i32::MIN, i32::MAX, 1),
        ] {
            let got = use_counter_leaving(own, remaining);
            assert_eq!(got, counter, "{own} {remaining}");
            if remaining <= own && got < i32::MAX {
                // Read back the way a use reads its ticks left.
                assert_eq!(ticks_left(own, got.unsigned_abs() - 1), remaining);
            }
            // Reading the ticks left back never overflows either.
            ticks_left(own, got.unsigned_abs() - 1);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn patched_bow_keeps_vanilla_animation_without_listener() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            use UseAnimation::*;
            for (id, expected) in [
                ("minecraft:bread", Eat),
                ("minecraft:milk_bucket", Drink),
                ("minecraft:shield", Block),
                ("minecraft:bow", Bow),
                ("minecraft:crossbow", Crossbow),
                ("minecraft:trident", Trident),
                ("minecraft:spyglass", Spyglass),
                ("minecraft:goat_horn", TootHorn),
                ("minecraft:brush", Brush),
                ("minecraft:bundle", Bundle),
                ("minecraft:iron_spear", Spear),
                ("minecraft:stick", None),
            ] {
                assert_eq!(
                    game.item_use_animation(&sunrise_client::items::ItemStack::new(id, 1)),
                    expected,
                    "{id}"
                );
            }
            let mut bow = sunrise_client::items::ItemStack::new("minecraft:bow", 1);
            bow.data_mut().set_other(
                "minecraft:consumable",
                Some(sunrise_core::nbt::Tag::compound([(
                    "animation",
                    sunrise_core::nbt::Tag::String("drink".into()),
                )])),
            );
            assert_eq!(game.item_use_animation(&bow), UseAnimation::Bow);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_animation_dispatches_vanilla_parts_and_refuses_invalid_spears() {
        use mod_api::{ClientHost, Hooks, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Clone)]
        struct Part(Arc<AtomicUsize>);
        #[mod_api::sunrise_part]
        impl ItemPart for Part {
            fn use_animation(&self, _: &dyn ClientHost, event: &mut UseAnimationEvent<'_>) {
                self.0.fetch_add(1, Ordering::Relaxed);
                let expected = match mod_api::__private::item_key_id(event.stack.item()).unwrap() {
                    "minecraft:bow" => UseAnimation::Bow,
                    "minecraft:crossbow" => UseAnimation::Crossbow,
                    "minecraft:trident" => UseAnimation::Trident,
                    "minecraft:brush" => UseAnimation::Brush,
                    "minecraft:wooden_spear"
                        if event.stack.components().is_none_or(|d| {
                            d.as_nbt()
                                .get(&"!minecraft:kinetic_weapon".into())
                                .is_none()
                        }) =>
                    {
                        UseAnimation::Spear
                    }
                    "minecraft:stick"
                        if event.stack.components().is_some_and(|d| {
                            d.as_nbt().get(&"minecraft:kinetic_weapon".into()).is_some()
                        }) =>
                    {
                        UseAnimation::Spear
                    }
                    _ => UseAnimation::None,
                };
                assert_eq!(*event.animation, expected);
                *event.animation = if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                    == "minecraft:stick"
                    || event.stack.components().is_some_and(|d| {
                        d.as_nbt()
                            .get(&"!minecraft:kinetic_weapon".into())
                            .is_some()
                    }) {
                    UseAnimation::Spear
                } else {
                    UseAnimation::Drink
                };
            }
        }
        struct Mod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                for id in [
                    "minecraft:bow",
                    "minecraft:crossbow",
                    "minecraft:trident",
                    "minecraft:brush",
                    "minecraft:wooden_spear",
                    "minecraft:stick",
                ] {
                    let item = r.item_key(id);
                    r.item_part(item, Part(self.0.clone()));
                }
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mut overridden_geometry = None;
        let mods: Mods = vec![ModEntry::new("animation", Box::new(Mod(calls.clone())))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                for id in [
                    "minecraft:bow",
                    "minecraft:crossbow",
                    "minecraft:trident",
                    "minecraft:brush",
                    "minecraft:wooden_spear",
                ] {
                    assert_eq!(
                        game.item_use_animation(&sunrise_client::items::ItemStack::new(id, 1)),
                        UseAnimation::Drink
                    );
                }
                assert_eq!(
                    game.item_use_animation(&sunrise_client::items::ItemStack::new(
                        "minecraft:stick",
                        1
                    )),
                    UseAnimation::None
                );
                let mut patched = sunrise_client::items::ItemStack::new("minecraft:bow", 1);
                patched.data_mut().set_other(
                    "minecraft:consumable",
                    Some(sunrise_core::nbt::Tag::compound([(
                        "animation",
                        sunrise_core::nbt::Tag::String("drink".into()),
                    )])),
                );
                assert_eq!(game.item_use_animation(&patched), UseAnimation::Drink);
                use sunrise_core::nbt::Tag;
                let mut kinetic_stick = sunrise_client::items::ItemStack::new("minecraft:stick", 1);
                kinetic_stick.data_mut().set_other(
                    "minecraft:kinetic_weapon",
                    Some(Tag::compound([("delay_ticks", Tag::Int(30))])),
                );
                assert!(matches!(
                    game.item_use_state(&kinetic_stick),
                    super::ItemUseState::Spear(_)
                ));
                let mut removed =
                    sunrise_client::items::ItemStack::new("minecraft:wooden_spear", 1);
                removed.data_mut().set_other(
                    "!minecraft:kinetic_weapon",
                    Some(Tag::Compound(Default::default())),
                );
                assert_eq!(game.item_use_animation(&removed), UseAnimation::None);
                assert_eq!(calls.load(Ordering::Relaxed), 9);
                game.player = Some(
                    sunrise_client::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::ZERO,
                    )
                    .unwrap(),
                );
                game.inventory[game.selected] =
                    Some(sunrise_client::items::ItemStack::new("minecraft:bow", 1));
                game.using = Some((game.selected, 0));
                game.tick_player_model();
                assert_eq!(
                    game.player_model.as_ref().unwrap().extra.use_animation,
                    UseAnimation::Drink
                );
                assert_eq!(calls.load(Ordering::Relaxed), 10);
                game.visible_hands[0].1 = game.inventory[game.selected].clone();
                game.use_duration = 72_000;
                let (overridden, _) = game.hand_geometry(0.0);
                assert_eq!(calls.load(Ordering::Relaxed), 11);
                // A scoped spear override on a component-patched ordinary item
                // must reach hand geometry without the old whitelist panic.
                game.inventory[game.selected] = Some(kinetic_stick);
                game.visible_hands[0].1 = game.inventory[game.selected].clone();
                let (kinetic_geometry, _) = game.hand_geometry(0.0);
                assert!(!kinetic_geometry.is_empty());
                overridden_geometry = Some(overridden);
            });
        });
        // The unmodified comparison owns a new resource graph and Parts snapshot.
        crate::game::api_values::install_parts(&[]).unwrap();
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.inventory[game.selected] =
                Some(sunrise_client::items::ItemStack::new("minecraft:bow", 1));
            game.using = Some((game.selected, 0));
            game.tick_player_model();
            game.visible_hands[0].1 = game.inventory[game.selected].clone();
            game.use_duration = 72_000;
            let (vanilla, _) = game.hand_geometry(0.0);
            let overridden = overridden_geometry.unwrap();
            assert!(!overridden.is_empty());
            assert_eq!(overridden.len(), vanilla.len());
            assert!(overridden.iter().zip(&vanilla).any(|(a, b)| a.pos != b.pos));
        });
    }

    /// `render_hand` draws in a hand that holds an item, where `render_arm`
    /// never fires. What the mod drew is in the hand's vertices, in a run of
    /// its own ahead of vanilla's, and it stays when the mod leaves vanilla's
    /// hand out. The pose the mod sets moves vanilla's hand, not what it
    /// drew. The empty off hand is asked too.
    #[test]
    #[ignore = "daily"]
    fn a_mod_draws_in_a_hand_that_holds_an_item() {
        use mod_api::{ClientHost, Flow, Hand, Hooks, ModEntry, Mods, RenderHandEvent};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        // One quad a block wide, a block in front of the eyes.
        const MARK: [[f32; 3]; 4] = [
            [0.0, 1.0, -1.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 1.0, -1.0],
        ];
        struct Mod {
            cancel: Arc<AtomicBool>,
            shift: Arc<AtomicBool>,
            asked: Arc<Mutex<Vec<(Hand, bool)>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
                let holds_an_item = event.render.stack.is_some();
                self.asked
                    .lock()
                    .unwrap()
                    .push((event.render.hand, holds_an_item));
                if self.shift.load(Ordering::Relaxed) {
                    event.render.translation[0] += 1.0;
                }
                // Draw in the hand that holds the sword only.
                if holds_an_item {
                    let uv = [0.0, 0.0, 16.0, 16.0];
                    assert!(event.draw.quad("block/stone", MARK, uv, 0xFF_FF_FF));
                }
                if self.cancel.load(Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        fn hold_a_sword(game: &mut crate::game::Game<'_>) {
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.inventory[game.selected] = Some(sunrise_client::items::ItemStack::new(
                "minecraft:iron_sword",
                1,
            ));
            game.tick_player_model();
            game.visible_hands[0].1 = game.inventory[game.selected].clone();
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let shift = Arc::new(AtomicBool::new(false));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let hooks = Mod {
            cancel: cancel.clone(),
            shift: shift.clone(),
            asked: asked.clone(),
        };
        let mods: Mods = vec![ModEntry::new("hand", Box::new(hooks))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        let mut drawn = None;
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                hold_a_sword(game);
                let kept = game.hand_geometry(0.0);
                shift.store(true, Ordering::Relaxed);
                let posed = game.hand_geometry(0.0).0;
                shift.store(false, Ordering::Relaxed);
                cancel.store(true, Ordering::Relaxed);
                drawn = Some((kept, posed, game.hand_geometry(0.0)));
            });
        });
        let ((kept, kept_runs), posed, (alone, alone_runs)) = drawn.unwrap();
        // Each frame both hands were asked, the empty off hand included.
        let frame = [(Hand::Main, true), (Hand::Off, false)];
        assert_eq!(*asked.lock().unwrap(), [frame, frame, frame].concat());
        // Cancelled: only the mod's quad, in a run with no rendered item id.
        assert!(!alone.is_empty());
        assert_eq!(alone_runs, [(0, 0)]);
        // The pose the mod set is not under its draw: the quad stays, and
        // vanilla's hand moves by the block asked along the view's x.
        assert_eq!(posed.len(), kept.len());
        let (quad, hand) = posed.split_at(alone.len());
        assert!(quad.iter().zip(&alone).all(|(a, b)| a.pos == b.pos));
        for (moved, still) in hand.iter().zip(&kept[alone.len()..]) {
            let by = (0..3)
                .map(|i| (moved.pos[i] - still.pos[i]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!((by - 1.0).abs() < 1e-4, "moved by {by}");
        }

        // The unmodified comparison owns a new resource graph and Parts snapshot.
        crate::game::api_values::install_parts(&[]).unwrap();
        super::super::native_entity_refs::tests::with_client_game(|game| {
            hold_a_sword(game);
            let (vanilla, vanilla_runs) = game.hand_geometry(0.0);
            assert!(!vanilla.is_empty());
            // Kept: the same quad first, then vanilla's hand exactly as without a mod.
            assert_eq!(kept.len(), alone.len() + vanilla.len());
            assert!(kept.iter().zip(&alone).all(|(a, b)| a.pos == b.pos));
            let hand = &kept[alone.len()..];
            assert!(hand.iter().zip(&vanilla).all(|(a, b)| a.pos == b.pos));
            let shifted = vanilla_runs
                .iter()
                .map(|&(start, id)| (start + alone.len() as u32, id));
            let runs = std::iter::once((0, 0)).chain(shifted).collect::<Vec<_>>();
            assert_eq!(kept_runs, runs);
        });
    }

    fn stand_in_first_person(game: &mut crate::game::Game<'_>) {
        game.player = Some(
            sunrise_client::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::ZERO,
            )
            .unwrap(),
        );
        game.tick_player_model();
    }

    #[test]
    #[ignore = "daily"]
    fn render_hand_submits_both_skinned_arms_in_draw_order() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
        use glam::{Mat4, Quat, Vec3};
        use mod_api::{
            ClientHost, Flow, Hand, Hooks, MainHand, ModEntry, RenderArmEvent, RenderHandEvent,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicU8, Ordering::Relaxed},
        };
        use sunrise_client::{hud::Rect, items::ItemStack};

        if run_fresh_cases(
            "game::item_use::tests::render_hand_submits_both_skinned_arms_in_draw_order",
            "SUNRISE_HAND_ARM_CASE",
            &["wide", "slim"],
        ) {
            return;
        }
        let slim = std::env::var("SUNRISE_HAND_ARM_CASE").unwrap() == "slim";
        const MOVES: [[f32; 3]; 2] = [[0.75, -0.5, -1.0], [-0.25, 0.5, -2.0]];
        const LIGHT: [u32; 2] = [0x30a0, 0x60b0];
        const SCALE: [f32; 3] = [5e-7, 1e-8, 1e-8];
        #[derive(Default)]
        struct State {
            mode: AtomicU8,
            default_arms: AtomicU8,
        }
        struct Mod(Arc<State>);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn render_arm(&self, _: &dyn ClientHost, _: &mut RenderArmEvent<'_>) -> Flow {
                self.0.default_arms.fetch_add(1, Relaxed);
                Flow::Continue
            }
            fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
                let mode = self.0.mode.load(Relaxed);
                if event.render.hand != Hand::Main || mode == 0 {
                    return Flow::Continue;
                }
                let original_light = event.draw.light();
                for (i, arm) in [MainHand::Left, MainHand::Right].into_iter().enumerate() {
                    event.draw.push();
                    event.draw.translate(MOVES[i]);
                    if mode == 3 && i == 0 {
                        event.draw.scale(SCALE);
                        event.draw.rotate_about([0.0, 0.0, 1.0], 90.0, [0.0; 3]);
                    }
                    event.draw.set_light(LIGHT[i]);
                    event.draw.render_arm(arm);
                    event.draw.pop();
                }
                event.draw.set_light(original_light);
                if mode == 2 {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let state = Arc::new(State::default());
        mod_platform::testing::with_mods(
            vec![ModEntry::new("arms", Box::new(Mod(state.clone())))].into(),
            || {
                with_client_game(|game| {
                    stand_in_first_person(game);
                    for (i, (slot, item)) in [
                        (game.selected, "minecraft:iron_sword"),
                        (super::OFFHAND_SLOT, "minecraft:stick"),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let stack = ItemStack::new(item, 1);
                        *game.stack_in_mut(slot) = Some(stack.clone());
                        game.visible_hands[i] = (slot, Some(stack));
                    }
                    let atlas = Rect {
                        u0: 0.25,
                        v0: 0.125,
                        u1: 0.75,
                        v1: 0.375,
                        w: 64,
                        h: 64,
                    };
                    game.gui
                        .atlas
                        .sprites
                        .insert("arms:fixture_skin".into(), atlas);
                    let avatar = game.player_model.as_mut().unwrap();
                    avatar.skin = Some("arms:fixture_skin".into());
                    avatar.slim = slim;
                    let (vanilla, runs) = game.hand_geometry(0.0);
                    let [_, (offhand_at, offhand_id)] = runs[..] else {
                        panic!("expected both item runs: {runs:?}")
                    };
                    assert!(offhand_at > 0 && (offhand_at as usize) < vanilla.len());
                    state.mode.store(1, Relaxed);
                    let (added, added_runs) = game.hand_geometry(0.0);
                    state.mode.store(2, Relaxed);
                    let (replaced, replaced_runs) = game.hand_geometry(0.0);
                    assert_eq!(state.default_arms.load(Relaxed), 0);

                    // Six quads for the arm and six for the sleeve. Check each layer's first
                    // corner from PlayerModel's cube origins/texture offsets, including 3px arms.
                    let width = if slim { 3.0 } else { 4.0 };
                    let quads = 6 * 4;
                    let arm_vertices = quads * 2;
                    let submitted = arm_vertices * 2;
                    let view = game.sway.to_world(0.0, &game.view);
                    for (i, (shoulder, roll, x, base_u, sleeve_u, v)) in [
                        (
                            5.0,
                            -0.1,
                            width - 1.0,
                            32.0 + 4.0 + width,
                            48.0 + 4.0 + width,
                            48.0,
                        ),
                        (-5.0, 0.1, 1.0, 40.0 + 4.0 + width, 40.0 + 4.0 + width, 16.0),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let world = view
                            * Mat4::from_translation(Vec3::from_array(MOVES[i]))
                            * Mat4::from_rotation_translation(
                                Quat::from_rotation_z(roll),
                                Vec3::new(shoulder / 16.0, 2.0 / 16.0, 0.0),
                            );
                        let arm = &added[i * arm_vertices..(i + 1) * arm_vertices];
                        for (vertex, inflate, u, v) in [
                            (arm[0], 0.0, base_u, v),
                            (
                                arm[quads],
                                0.25,
                                sleeve_u,
                                if i == 0 { v } else { v + 16.0 },
                            ),
                        ] {
                            let p = Vec3::new(x + inflate, -2.0 - inflate, 2.0 + inflate) / 16.0;
                            assert_eq!(
                                vertex.pos.map(f32::to_bits),
                                world.transform_point3(p).to_array().map(f32::to_bits)
                            );
                            assert_eq!(vertex.uv, [0.25 + u / 128.0, 0.125 + v / 256.0]);
                        }
                        assert!(
                            arm.iter()
                                .all(|vertex| vertex.light == LIGHT[i] && vertex.layer == 0)
                        );
                    }
                    assert_eq!(
                        &added_runs[..],
                        &[
                            (0, 0),
                            (submitted as u32, 0),
                            (submitted as u32 + offhand_at, offhand_id)
                        ]
                    );
                    assert_eq!(
                        &replaced_runs[..],
                        &[(0, 0), (submitted as u32, offhand_id)]
                    );
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&added[submitted..]),
                        bytemuck::cast_slice::<_, u8>(&vanilla)
                    );
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&replaced[..submitted]),
                        bytemuck::cast_slice::<_, u8>(&added[..submitted])
                    );
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&replaced[submitted..]),
                        bytemuck::cast_slice::<_, u8>(&vanilla[offhand_at as usize..])
                    );

                    state.mode.store(3, Relaxed);
                    let (scaled, _) = game.hand_geometry(0.0);
                    // ModelPart.Cube's first polygon faces DOWN. The left arm's
                    // -0.1 roll followed by the listener's quarter turn sends -Y
                    // toward +X and -Y. Scale normals by the reciprocal axes.
                    let normal = view
                        .transform_vector3(Vec3::new(
                            sunrise_core::jmath::cos(0.1) as f32 / SCALE[0],
                            -(sunrise_core::jmath::sin(0.1) as f32) / SCALE[1],
                            0.0,
                        ))
                        .normalize();
                    let packed = normal.to_array().map(|n| (n * 127.0) as i8);
                    assert!(scaled[..4].iter().all(|v| v.normal[..3] == packed));
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&scaled[arm_vertices..submitted]),
                        bytemuck::cast_slice::<_, u8>(&added[arm_vertices..submitted]),
                        "pop restores the unscaled normal pose for the second arm"
                    );

                    game.gui.atlas.sprites.remove("arms:fixture_skin");
                    for missing_avatar in [false, true] {
                        if missing_avatar {
                            game.player_model = None;
                        }
                        state.mode.store(0, Relaxed);
                        let (items, item_runs) = game.hand_geometry(0.0);
                        state.mode.store(1, Relaxed);
                        let (drawn, drawn_runs) = game.hand_geometry(0.0);
                        assert_eq!(drawn_runs, item_runs);
                        assert_eq!(
                            bytemuck::cast_slice::<_, u8>(&drawn),
                            bytemuck::cast_slice::<_, u8>(&items),
                            "no arm without the selected sprite or the avatar state"
                        );
                    }
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn default_first_person_arm_obeys_its_own_sleeve_option() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
        use sunrise_client::options_file::V;

        if run_fresh_cases(
            "game::item_use::tests::default_first_person_arm_obeys_its_own_sleeve_option",
            "SUNRISE_HAND_SLEEVE_CASE",
            &["left-hidden"],
        ) {
            return;
        }
        with_client_game(|game| {
            stand_in_first_person(game);
            game.settings.main_hand_left = true;
            let left = game.hand_geometry(0.0).0;
            game.settings.main_hand_left = false;
            let right = game.hand_geometry(0.0).0;

            game.settings
                .file
                .set("modelPart_left_sleeve", V::Bool(false));
            game.tick_player_model();
            let right_hidden_left = game.hand_geometry(0.0).0;
            game.settings.main_hand_left = true;
            let left_hidden_left = game.hand_geometry(0.0).0;

            assert_eq!(
                (left.len(), right.len(), left_hidden_left.len()),
                (48, 48, 24)
            );
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&left[..24]),
                bytemuck::cast_slice::<_, u8>(&left_hidden_left)
            );
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&right),
                bytemuck::cast_slice::<_, u8>(&right_hidden_left)
            );
        });
    }

    #[test]
    fn a_use_runs_out_on_its_last_tick() {
        // a 32 tick food: ticks 0 to 30 go on, the 32nd tick finishes it.
        assert_eq!(count_down(0, 32), Some(1));
        assert_eq!(count_down(30, 32), Some(31));
        assert_eq!(count_down(31, 32), None);
        // a one tick use finishes on its first tick.
        assert_eq!(count_down(0, 1), None);
        // no time, or one below what was used, waits for the release.
        assert_eq!(count_down(0, 0), Some(1));
        assert_eq!(count_down(10, 4), Some(11));
        assert_eq!(count_down(5, 72_000), Some(6));
    }

    #[test]
    fn a_started_use_finishes_after_its_duration() {
        // start at 0 ticks, count down each tick, finish when it runs out.
        for duration in [1, 16, 32, 40] {
            let (mut ticks, mut ticked) = (0, 0);
            while let Some(next) = count_down(ticks, duration) {
                ticks = next;
                ticked += 1;
            }
            assert_eq!(ticked + 1, duration);
        }
    }

    #[test]
    fn a_start_below_0_refuses_the_use_and_a_mods_0_runs_out() {
        // a cancel or a time below 0 refuses it.
        assert_eq!(start_of(32, 32, true), None);
        assert_eq!(start_of(32, -1, false), None);
        assert_eq!(start_of(0, -1, false), None);
        // a mod's 0 starts it, and its first tick runs it out.
        assert_eq!(start_of(32, 0, false), Some(true));
        let (left, set) = tick_left(0, 0, false, true);
        assert!(!(left > 0 || !set));
        // an item's own 0 (an eye of ender) goes on, as a changed time does.
        assert_eq!(start_of(0, 0, false), Some(false));
        let (left, set) = tick_left(0, 0, false, false);
        assert!(left > 0 || !set);
        assert_eq!(start_of(32, 64, false), Some(false));
    }

    #[test]
    fn a_tick_moves_the_end_of_a_use() {
        // a 32 tick use, 10 ticks in: 22 left.
        assert_eq!(ticks_left(32, 10), 22);
        // a mod leaves 1 tick: the next tick finishes it.
        let duration = duration_leaving(10, 1);
        assert_eq!(count_down(10, duration), None);
        // or 30 more: it goes on to tick 40.
        let duration = duration_leaving(10, 30);
        assert_eq!(duration, 40);
        assert_eq!(count_down(38, duration), Some(39));
        assert_eq!(count_down(39, duration), None);
        // left as it was, nothing moves.
        assert_eq!(duration_leaving(10, ticks_left(32, 10)), 32);
    }

    #[test]
    fn a_tick_of_no_time_left_runs_the_use_out() {
        let goes_on = |(left, set): (i32, bool)| left > 0 || !set;
        // untouched, a use goes on, even one of no time (an eye of ender).
        assert_eq!(tick_left(20, 20, false, false), (20, false));
        assert!(goes_on(tick_left(-3, -3, false, false)));
        // a mod's ticks left go on while above 0.
        assert_eq!(tick_left(20, 5, false, false), (5, true));
        assert!(goes_on(tick_left(20, 5, false, false)));
        // a cancel is -1 left, and it or 0 or less set runs the use out.
        assert_eq!(tick_left(20, 20, true, false), (-1, true));
        assert!(!goes_on(tick_left(20, 0, false, false)));
        assert!(!goes_on(tick_left(20, -1, false, false)));
        // A run-out use keeps skipping effects while its counter goes negative.
        assert!(!goes_on(tick_left(-1, -1, false, true)));
        // and goes on again if a mod gives it ticks back.
        assert!(goes_on(tick_left(-1, 10, false, true)));
    }

    #[test]
    fn a_release_runs_what_vanilla_runs() {
        // let go: the item's release and the spyglass's sound.
        assert_eq!(release_parts(false, false), (true, true));
        // a mod's Stop canceled: neither.
        assert_eq!(release_parts(true, false), (false, false));
        // another item in the hand: no release (Sunrise still lowers a spyglass).
        assert_eq!(release_parts(false, true), (false, true));
    }

    #[test]
    fn bow_release_respects_charge_cancel_and_ammo() {
        assert_eq!(bow_release_power(true, 20, false), Some(1.0));
        assert_eq!(bow_release_power(true, 10, false), Some(5.0 / 12.0));
        assert_eq!(bow_release_power(true, 0, false), None);
        assert_eq!(bow_release_power(true, -1, false), None);
        assert_eq!(bow_release_power(true, 20, true), None);
        assert_eq!(bow_release_power(false, 20, false), None);
    }

    #[test]
    fn a_moved_end_moves_the_charge() {
        // a bow held 10 ticks of 72000: 10, as vanilla's.
        assert_eq!(held(72_000, ticks_left(72_000, 10)), 10);
        // a mod's Tick leaves 71000 at tick 10: it counts as held 1000.
        assert_eq!(
            held(72_000, ticks_left(duration_leaving(10, 71_000), 10)),
            1000
        );
        // more ticks left than the item's own time: not held at all.
        assert_eq!(held(72_000, 80_000), 0);
    }

    #[test]
    fn a_release_tells_how_long_the_item_was_held() {
        // a bow-like item held 25 ticks of 72000: the time left gives the charge back.
        let left = ticks_left(72_000, 25);
        assert_eq!(left, 71_975);
        assert_eq!(72_000 - left, 25);
        // released at once.
        assert_eq!(ticks_left(72_000, 0), 72_000);
    }

    #[test]
    fn a_food_of_no_time_is_eaten_at_once() {
        assert!(consumes_at_once(0));
        assert!(consumes_at_once(-3));
        assert!(!consumes_at_once(1));
        assert!(!consumes_at_once(32));
    }

    #[test]
    fn a_use_animation_picks_the_hand_pose() {
        let pose = |animation| use_pose(animation, 12.0, 32.0, 0.5);
        assert!(matches!(
            pose(UseAnimation::Drink),
            ItemPose::Eat { remaining, duration, partial }
                if (remaining, duration, partial) == (12.0, 32.0, 0.5)
        ));
        // ticks used: 32 - (12 - 0.5 + 1).
        assert!(
            matches!(pose(UseAnimation::Bow), ItemPose::Bow { ticks_used } if ticks_used == 19.5)
        );
        assert!(matches!(
            pose(UseAnimation::Trident),
            ItemPose::Trident { ticks_used } if ticks_used == 19.5
        ));
        assert!(matches!(
            pose(UseAnimation::Brush),
            ItemPose::Brush { remaining, partial } if (remaining, partial) == (12.0, 0.5)
        ));
        for animation in [
            UseAnimation::None,
            UseAnimation::Block,
            UseAnimation::Spyglass,
        ] {
            assert!(matches!(pose(animation), ItemPose::Using));
        }
    }
}

#[cfg(test)]
pub(super) mod interact_vibration_tests {
    use super::*;

    type RecordedEvent = (&'static str, [f64; 3], Option<sunrise_client::mobs::Who>);
    thread_local! {
        pub(in crate::game) static EVENTS: std::cell::RefCell<Vec<RecordedEvent>> = const { std::cell::RefCell::new(Vec::new()) };
        static RECORDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub(in crate::game) fn record_event(
        event: &'static str,
        pos: [f64; 3],
        source: Option<sunrise_client::mobs::Who>,
    ) {
        if RECORDING.get() {
            EVENTS.with(|events| events.borrow_mut().push((event, pos, source)));
        }
    }

    /// Runs `f` on a windowless game built from the prepared resources with
    /// `held` in the main hand and returns the game events it posted;
    /// `None` without the resources.
    pub(in crate::game) fn with_game(
        held: &str,
        f: impl FnOnce(&mut Game<'_>),
    ) -> Option<Vec<String>> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES")?);
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.player = Some(
            sunrise_client::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(10.5, 64.0, -3.25),
            )
            .unwrap(),
        );
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        super::super::native_entity_refs::tests::bind_local_player(&mut game, 41, 123);
        *game.held_mut() = Some(ItemStack::new(held, 1));
        crate::game::dispense_fixture::start_recording();
        RECORDING.set(true);
        f(&mut game);
        RECORDING.set(false);
        Some(crate::game::dispense_fixture::stop_recording())
    }

    /// `LivingEntity.startUsingItem` ends with
    /// `this.useItem.causeUseVibration(this, GameEvent.ITEM_INTERACT_START)`,
    /// and `ItemStack.causeUseVibration` posts only when the stack's use effects
    /// ask for interaction vibrations (the default). `stopUsingItem` posts the
    /// finish the same way, which Sunrise already did, so a use is a pair of
    /// vibrations and an item that turns them off is neither.
    #[test]
    #[ignore = "daily"]
    fn a_use_start_is_an_item_interact_start_vibration() {
        let bow = || ItemStack::new("minecraft:bow", 1);
        let Some(lines) = with_game("minecraft:bow", |game| {
            assert!(game.start_using_item(), "the bow's use starts");
            game.stop_using_item(&bow());
        }) else {
            return;
        };
        assert_eq!(
            lines,
            vec![
                "G 10,64,-4 minecraft:item_interact_start".to_owned(),
                "G 10,64,-4 minecraft:item_interact_finish".to_owned(),
            ]
        );
        // A spear's use effects set `interact_vibrations` false: no vibration.
        let spear = || ItemStack::new("minecraft:iron_spear", 1);
        let Some(lines) = with_game("minecraft:iron_spear", |game| {
            assert!(game.start_using_item(), "the spear's use starts");
            game.stop_using_item(&spear());
        }) else {
            return;
        };
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    #[ignore = "daily"]
    fn cake_context_uses_can_eat_instead_of_raw_hunger() {
        use mod_api::{CanEatEvent, Hooks};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Reverse(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Reverse {
            fn can_eat(
                &self,
                host: &dyn mod_api::Host,
                event: &mut CanEatEvent<'_>,
            ) -> Option<bool> {
                assert_eq!(event.player.entity().id(), 41);
                assert_eq!(
                    event.player.food(host).unwrap().level,
                    if self.0.load(Ordering::Relaxed) < 4 {
                        20
                    } else {
                        10
                    }
                );
                let call = self.0.fetch_add(1, Ordering::Relaxed);
                if event
                    .stack
                    .is_some_and(|stack| stack.item().id(host) == Some("minecraft:rotten_flesh"))
                {
                    return Some(false);
                }
                assert!(event.stack.is_none(), "cake has no food stack");
                // First exercise player-based overrides, then the stack-only refusal.
                (call < 4).then_some(!event.vanilla_can_eat)
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cake-refusal",
            Box::new(Reverse(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game("minecraft:rotten_flesh", |game| {
                let use_block = |game: &mut Game<'_>, name: &str| {
                    let player = game.use_context();
                    game.with_block_use_level(|b, level, ticks| {
                        let state = b.table.default_state(b.table.block_by_name(name).unwrap());
                        b.use_without_item(
                            level,
                            ticks,
                            state,
                            sunrise_core::geometry::BlockHit {
                                block: sunrise_core::position::BlockPos::new(0, 64, 0),
                                direction: sunrise_core::geometry::Direction::Up,
                                location: sunrise_core::geometry::Vec3::new(0.5, 65.0, 0.5),
                                inside: false,
                            },
                            player,
                        )
                    })
                };
                game.creative = true;
                use_block(game, "minecraft:stone");
                assert_eq!(calls.load(Ordering::Relaxed), 0);
                assert_eq!(
                    use_block(game, "minecraft:cake"),
                    sunrise_world::level::InteractionResult::Pass
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(
                    use_block(game, "minecraft:candle_cake"),
                    sunrise_world::level::InteractionResult::Pass
                );
                assert_eq!(calls.load(Ordering::Relaxed), 2);
                game.creative = false;
                game.player.as_mut().unwrap().health.food.food = 20;
                assert_eq!(
                    use_block(game, "minecraft:cake"),
                    sunrise_world::level::InteractionResult::Success
                );
                assert_eq!(calls.load(Ordering::Relaxed), 3);
                assert_eq!(
                    use_block(game, "minecraft:candle_cake"),
                    sunrise_world::level::InteractionResult::Success,
                    "a stack-based food refusal does not refuse a candle-cake bite"
                );
                assert_eq!(calls.load(Ordering::Relaxed), 4);
                game.player.as_mut().unwrap().health.food.food = 10;
                assert_eq!(
                    use_block(game, "minecraft:cake"),
                    sunrise_world::level::InteractionResult::Success,
                    "holding refused food does not refuse a hungry player's cake bite"
                );
                assert_eq!(
                    use_block(game, "minecraft:candle_cake"),
                    sunrise_world::level::InteractionResult::Success
                );
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                game.with_level(|b, level, _| {
                    let cake = b
                        .table
                        .default_state(b.table.block_by_name("minecraft:cake").unwrap());
                    assert!(
                        sunrise_world::level::LevelAccess::set_raw(
                            level,
                            sunrise_core::position::BlockPos::new(0, 64, 0),
                            cake,
                        )
                        .is_some()
                    );
                });
                let hit = super::super::Hit {
                    pos: [0, 64, 0],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 0.5],
                    inside: false,
                };
                let held = game.held().cloned();
                let (result, (changes, _)) =
                    game.remote_block_template(&hit, held.as_ref(), true, true);
                assert_eq!(result, sunrise_world::level::InteractionResult::Success);
                assert_eq!(changes.len(), 1, "the client predicts a bitten cake");
                let bitten = game.with_level(|b, _, _| {
                    let cake = b
                        .table
                        .default_state(b.table.block_by_name("minecraft:cake").unwrap());
                    b.table.set_int(cake, b.props.bites, 1)
                });
                assert_eq!(changes[0].1, bitten);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    7,
                    "client dry-level path asks the hook"
                );
            })
            .expect("SUNRISE_TEST_RESOURCES must name prepared resources");
        });
        can_eat_player_on_client_and_without_eater();
    }

    /// The listener sees the logged-in remote player on a client, and a food
    /// stack with no resolvable eater keeps vanilla's answer without asking it.
    fn can_eat_player_on_client_and_without_eater() {
        use mod_api::{CanEatEvent, Hooks};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;

        const REMOTE: u128 = 0xC0FF_EE00_0000_0000_0000_0000_0000_002A;
        struct Allow(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Allow {
            fn can_eat(
                &self,
                host: &dyn mod_api::Host,
                event: &mut CanEatEvent<'_>,
            ) -> Option<bool> {
                assert_eq!(event.player.entity().id(), 55);
                assert_eq!(event.player.uuid(), REMOTE);
                let food = event.player.food(host).expect("the client's own food data");
                assert_eq!((food.level, food.saturation), (20, 2.5));
                assert!(!event.vanilla_can_eat, "a full survival player cannot eat");
                self.0.fetch_add(1, Ordering::Relaxed);
                Some(true)
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "allow-eating",
            Box::new(Allow(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game("minecraft:rotten_flesh", |game| {
                let food = ItemStack::new("minecraft:rotten_flesh", 1);
                game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "Eater").unwrap();
                // Replace the server fixture's identity and the body removed on join.
                game.lan_acting_uuid = None;
                game.level.player_uuid = None;
                let remote = game.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.profile = REMOTE;
                remote.entity_id = 55;
                let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap();
                player.entity_id = 55;
                player.health.food.food = 20;
                player.health.food.saturation = 2.5;
                game.player = Some(player);
                assert!(
                    game.can_eat(Some(&food), false),
                    "the listener lets a full player eat"
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                // Keep the body and food state, but no logged-in identity resolves.
                game.remote.as_mut().unwrap().logged_in = false;
                assert!(!game.can_eat(Some(&food), false));
                assert_eq!(calls.load(Ordering::Relaxed), 1, "no eater, no question");
            })
            .expect("SUNRISE_TEST_RESOURCES must name prepared resources");
        });
    }

    #[test]
    fn stack_use_effects_patch_controls_vibrations() {
        let mut rod = ItemStack::new("minecraft:fishing_rod", 1);
        assert!(interact_vibrations(&rod, Some(true)));
        rod.data_mut().set_loot_component(
            "minecraft:use_effects",
            &Some(
                sunrise_core::json_nbt::json_to_tag(
                    &serde_json::json!({"interact_vibrations": false}),
                )
                .unwrap(),
            ),
        );
        assert!(!interact_vibrations(&rod, Some(true)));
        rod.data_mut().set_loot_component(
            "minecraft:use_effects",
            &Some(sunrise_core::nbt::Tag::Compound(Default::default())),
        );
        assert!(interact_vibrations(&rod, Some(false)));
        rod.data_mut().set_other("minecraft:use_effects", None);
        rod.data_mut().set_other(
            "!minecraft:use_effects",
            Some(sunrise_core::nbt::Tag::Compound(Default::default())),
        );
        assert!(!interact_vibrations(&rod, Some(true)));
    }

    /// `FishingRodItem.use` and the cast model's `FishingRodCast` take a
    /// content mod's item copied from the rod (`ItemInfo::is`) as a rod: it
    /// casts, is drawn cast while its hook is out, and reels in. Another item
    /// is not a rod.
    #[test]
    #[ignore = "daily"]
    fn a_content_mods_rod_casts_and_reels_in() {
        let Some(_) = with_game("minecraft:stick", |game| {
            // A world with a data pack has tables of its own, which take the mod's item.
            let dir =
                std::env::temp_dir().join(format!("sunrise-content-rod-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let tags = dir.join("datapacks/rods/data/ruby/tags/item");
            std::fs::create_dir_all(&tags).unwrap();
            std::fs::write(
                dir.join("datapacks/rods/pack.mcmeta"),
                r#"{"pack":{"description":"fixture"}}"#,
            )
            .unwrap();
            std::fs::write(
                tags.join("rods.json"),
                r#"{"values":["minecraft:fishing_rod"]}"#,
            )
            .unwrap();
            std::fs::write(
                dir.join("datapacks.json"),
                r#"{"enabled":["file/rods"],"disabled":[]}"#,
            )
            .unwrap();
            game.world_root = dir.clone();
            let stack = game.reload_datapacks(false, &[]).unwrap();
            let root = game.options.resources.clone();
            game.world_tables = game
                .stage_world_tables(&stack, false, &root)
                .unwrap()
                .tables;
            let items =
                &mut std::sync::Arc::make_mut(&mut game.world_tables.as_mut().unwrap().items).items;
            let mut info = items["minecraft:fishing_rod"].clone();
            info.id = std::sync::Arc::from("ruby:rod");
            info.template = Some(std::sync::Arc::from("minecraft:fishing_rod"));
            assert!(
                super::super::remote::item_has_use(&info),
                "remote use accepts content rods"
            );
            items.insert(info.id.clone(), info);

            let items =
                &mut std::sync::Arc::make_mut(&mut game.world_tables.as_mut().unwrap().items).items;
            let mut stick_info = items["minecraft:stick"].clone();
            stick_info.id = std::sync::Arc::from("ruby:stick");
            stick_info.template = Some(std::sync::Arc::from("minecraft:stick"));
            items.insert(stick_info.id.clone(), stick_info);
            // Publish the edited tables as world open or reload would.
            game.load_server_loot();
            let stick = ItemStack::new("ruby:stick", 1);
            assert!(!game.use_fishing_rod(&stick), "a content stick is no rod");
            assert!(game.hook.is_none());
            let rod = ItemStack::new("ruby:rod", 1);
            *game.held_mut() = Some(rod.clone());
            assert!(game.main_hand_rod());
            let used_before = game.stats.get("used", "ruby:rod");
            let vanilla_before = game.stats.get("used", "minecraft:fishing_rod");
            assert!(game.use_fishing_rod(&rod), "the mod's rod casts");
            assert_eq!(game.stats.get("used", "ruby:rod"), used_before + 1);
            assert_eq!(
                game.stats.get("used", "minecraft:fishing_rod"),
                vanilla_before
            );
            assert!(game.hook.is_some());
            assert_eq!(game.cast_rod(), Some(&rod), "drawn cast");
            game.tick_fishing();
            assert!(
                game.hook.is_some(),
                "the content rod keeps its hook when ticked"
            );
            assert!(game.use_fishing_rod(&rod), "and reels in");
            assert_eq!(game.stats.get("used", "ruby:rod"), used_before + 1);
            assert_eq!(
                game.stats.get("used", "minecraft:fishing_rod"),
                vanilla_before
            );
            assert!(game.hook.is_none());
            assert_eq!(game.cast_rod(), None);
            std::fs::remove_dir_all(&dir).unwrap();
        }) else {
            panic!("prepared resources failed to load")
        };
    }

    #[test]
    #[ignore = "daily"]
    fn fishing_cast_and_reel_post_player_vibrations() {
        use sunrise_client::mobs::Who;
        let Some(_) = with_game("minecraft:fishing_rod", |game| {
            let rod = ItemStack::new("minecraft:fishing_rod", 1);
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.use_fishing_rod(&rod));
            assert!(game.use_fishing_rod(&rod));
            EVENTS.with(|events| {
                assert_eq!(
                    *events.borrow(),
                    vec![
                        (
                            "minecraft:item_interact_start",
                            [10.5, 64.0, -3.25],
                            Some(Who::Player)
                        ),
                        (
                            "minecraft:item_interact_finish",
                            [10.5, 64.0, -3.25],
                            Some(Who::Player)
                        ),
                    ]
                );
            });
            let mut silent = rod.clone();
            silent.data_mut().set_loot_component(
                "minecraft:use_effects",
                &Some(
                    sunrise_core::json_nbt::json_to_tag(
                        &serde_json::json!({"interact_vibrations": false}),
                    )
                    .unwrap(),
                ),
            );
            *game.held_mut() = Some(silent.clone());
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.use_fishing_rod(&silent));
            assert!(game.use_fishing_rod(&silent));
            EVENTS.with(|events| assert!(events.borrow().is_empty()));
            *game.held_mut() = Some(rod.clone());
            assert!(game.use_fishing_rod(&rod));
            game.hook
                .as_mut()
                .unwrap()
                .move_to([10.5, 64.0, -3.25], true);
            let max = game
                .items()
                .get("minecraft:fishing_rod")
                .unwrap()
                .max_damage;
            game.held_mut().as_mut().unwrap().damage = max - 1;
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.use_fishing_rod(&rod));
            EVENTS.with(|events| assert!(events.borrow().is_empty(), "{:?}", *events.borrow()));
        }) else {
            panic!("prepared resources failed to load")
        };
    }

    #[test]
    #[ignore = "daily"]
    fn thrown_eye_posts_projectile_shoot_at_eye_from_player() {
        use sunrise_client::mobs::Who;
        let Some(_) = with_game("minecraft:ender_eye", |game| {
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let terrain = sunrise_client::vanillagen::VanillaTerrain::new(
                sunrise_client::vanillagen::TerrainResources {
                    dir: &root.join("worldgen"),
                    data: Some(&game.pack.data_changes),
                    registry: game.pack.assets.registry,
                    biomes: &game.res.core.biomes,
                    item_data: &game.res.core.item_data,
                },
                1234,
            )
            .unwrap();
            game.end.set_locator(Some(std::sync::Arc::new(terrain)));
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(
                game.use_ender_eye(&ItemStack::new("minecraft:ender_eye", 1), false)
                    .unwrap()
            );
            EVENTS.with(|events| {
                let events = events.borrow();
                assert!(
                    events.contains(&(
                        "minecraft:projectile_shoot",
                        [10.5, 64.0 + f64::from(1.8_f32) * 0.5, -3.25],
                        Some(Who::Player),
                    )),
                    "{events:?}"
                );
            });
        }) else {
            panic!("prepared resources failed to load")
        };
    }
}
