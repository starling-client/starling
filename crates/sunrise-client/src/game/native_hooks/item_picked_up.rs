use super::*;
use crate::game::commands::Target;
use sunrise_client::entities::ItemEntity;
use sunrise_client::join_seq::{JoinList, Joined};
use sunrise_core::random::RandomSource;

/// Where a touching item entity lives during `ItemEntity.playerTouch`.
///
/// The host player's pickup pass walks a lifted `JoinList` (`items`), while
/// guest pickups walk `self.dropped` directly inside `as_guest` turns.
enum PickupSlot<'a> {
    Lifted {
        items: &'a mut JoinList<ItemEntity>,
        index: &'a mut usize,
        feet: [f64; 3],
    },
    Resident {
        conn: u32,
        item_id: u32,
        taken: &'a mut Vec<(u32, u32)>,
    },
}

impl Game<'_> {
    /// `player` touches the ground item entity `item`
    /// (`ItemEntity.playerTouch`), before its pickup delay and target are
    /// checked: it is asked on every tick the two touch, even while the item
    /// can't be taken. `Default` requires a live delay of exactly zero and
    /// no target or this player as the target. Set `True` to take it despite
    /// its delay or another player's target
    /// (a full inventory still leaves it), or `False` to veto this pickup
    /// attempt without changing the item's pickup delay (matching NeoForge
    /// `ItemEntityPickupEvent.Pre`). The last mod to set it decides. The
    /// handle reads and mutates the live ground item entity on both the host
    /// player and guest paths. Pickup inserts from the stack captured before
    /// Pre, without overwriting a replacement installed by the listener. Server-side.
    pub(in crate::game) fn native_item_picked_up(
        &mut self,
        player: u128,
        item: ItemEntityRef,
        can_pickup: &mut TriState,
    ) {
        self.fire_item_picked_up(player, item, can_pickup);
    }

    /// `player` picked up all or part of the stack of the ground item entity
    /// `item` (`ItemEntity.playerTouch`, right after `Inventory.add` took some
    /// of it), the entity `item_picked_up` named. `original` is a copy of the
    /// stack as it lay before the pickup: changing it does nothing.
    /// `remaining` is the part of the captured stack that did not fit, or `None`
    /// if all of it fit. An explicit Pre stack replacement stays on `item`
    /// unless Post edits `remaining`. The amount taken is `original`'s count
    /// less `remaining`'s. A mod that removes the
    /// item entity meanwhile keeps it removed: no take animation, the stat and
    /// criteria still run. Not called when `item_picked_up` refused the
    /// pickup or the inventory took nothing; a forced `True` from
    /// `item_picked_up` changes none of this. A creative player always takes
    /// the whole stack. Server-side.
    pub(in crate::game) fn native_item_picked_up_post(
        &mut self,
        player: u128,
        item: ItemEntityRef,
        original: &ItemStack,
        remaining: &mut Option<ItemStack>,
    ) {
        self.fire_item_picked_up_post(player, item, original, remaining);
    }

    /// `Inventory.add` of the ground `stack` the acting player picks up: what
    /// did not fit. An empty stack inserts nothing (`Inventory.add(EMPTY) == false`).
    /// A player with infinite materials (creative) takes all of a non-empty stack.
    pub(in crate::game) fn native_pickup_insert(&mut self, stack: &ItemStack) -> Option<ItemStack> {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return Some(stack.clone());
        }
        let rest = self.add_stack_to_inventory(stack.clone());
        rest.filter(|_| !self.creative)
    }

    /// `item_picked_up_post` for the acting player, who took all or part of
    /// `original` from the item entity `item`, leaving `remaining` (`None`
    /// when all of it went in): the remainder after listener edits.
    pub(in crate::game) fn native_pickup_post(
        &mut self,
        item: u32,
        original: &ItemStack,
        mut remaining: Option<ItemStack>,
    ) -> Option<ItemStack> {
        if mod_api::__private::listened(mod_api::HookId::ItemPickedUpPost)
            && let Some(item) = self.native_item_handle(item)
        {
            let player = self.native_acting_id();
            self.native_item_picked_up_post(player, item, original, &mut remaining);
        }
        remaining
    }

    /// `ItemEntity.playerTouch` for the host's player, at `feet`, and each of
    /// the lifted `items` whose box meets `reach` (its low and high corners):
    /// `item_picked_up`, `Inventory.add`, `item_picked_up_post`, then the
    /// stat, the criterion and the take animation. An item whose notice a mod
    /// hears is lent back to the level meanwhile, so the mod reaches and
    /// mutates it live, and one that removes it keeps it removed.
    pub(in crate::game) fn native_player_pickups(
        &mut self,
        items: &mut JoinList<ItemEntity>,
        (min, max): ([f64; 3], [f64; 3]),
        feet: [f64; 3],
        own: [i32; 4],
    ) {
        let mut i = 0;
        while i < items.len() {
            let b = items[i].bounds();
            let touching = b.min_position().x < max[0]
                && b.max_position().x > min[0]
                && b.min_position().y < max[1]
                && b.max_position().y > min[1]
                && b.min_position().z < max[2]
                && b.max_position().z > min[2];
            if !touching {
                i += 1;
                continue;
            }
            self.native_touch_item_entity(
                PickupSlot::Lifted {
                    items,
                    index: &mut i,
                    feet,
                },
                own,
            );
        }
    }

    /// `ItemEntity.playerTouch` for guest `conn` touching resident item `item_id`.
    pub(in crate::game) fn native_guest_pickup(
        &mut self,
        conn: u32,
        item_id: u32,
        player: [i32; 4],
        taken: &mut Vec<(u32, u32)>,
    ) {
        self.native_touch_item_entity(
            PickupSlot::Resident {
                conn,
                item_id,
                taken,
            },
            player,
        );
    }

    /// Unified `ItemEntity.playerTouch` sequence shared by host-player and guest
    /// pickups (matching NeoForge `ItemEntityPickupEvent.Pre` and `Post`):
    /// 1. Capture the pre-event stack before firing `Pre`.
    /// 2. Fire `Pre` with the item resident in `self.dropped` (lending lifted
    ///    items temporarily so listeners can read and mutate the live entity).
    /// 3. If the listener removed the item (`discard` or `/kill`), stop without
    ///    picking up or resurrecting it.
    /// 4. Evaluate `pickup_decision` against the live post-hook `pickup_delay`
    ///    and `target`: `False` returns immediately, preserving all setter edits.
    /// 5. Insert the captured pre-event stack (`Inventory.add`).
    /// 6. Fire `Post` on whole or partial insertion, preserving explicit
    ///    `set_stack` replacements unless `Post` overrides `remaining`, and
    ///    handling listener removal without take animation.
    fn native_touch_item_entity(&mut self, mut slot: PickupSlot<'_>, player: [i32; 4]) {
        let listens_pre = mod_api::__private::listened(mod_api::HookId::ItemPickedUp);
        let (id, captured, stack_revision) = match &mut slot {
            PickupSlot::Lifted { items, index, .. } => {
                let item = &items[**index];
                let id = item.id;
                if self.persist.removals.contains(&Target::Item(id))
                    || (!listens_pre
                        && !Self::pickup_decision(
                            item.pickup_delay,
                            item.target,
                            player,
                            TriState::Default,
                        ))
                {
                    **index += 1;
                    return;
                }
                (id, item.stack.clone(), item.stack_revision)
            }
            PickupSlot::Resident { item_id, .. } => {
                let id = *item_id;
                if self.persist.removals.contains(&Target::Item(id)) {
                    return;
                }
                let Some(at) = self.dropped.iter().position(|d| d.id == id) else {
                    return;
                };
                let item = &self.dropped[at];
                if !listens_pre
                    && !Self::pickup_decision(
                        item.pickup_delay,
                        item.target,
                        player,
                        TriState::Default,
                    )
                {
                    return;
                }
                (id, item.stack.clone(), item.stack_revision)
            }
        };

        let answer = if listens_pre {
            let (answer, present, removed) = match &mut slot {
                PickupSlot::Lifted { items, index, .. } => {
                    self.lend_lifted_item(items, **index, |game| game.native_pickup_answer(id))
                }
                PickupSlot::Resident { conn, .. } => {
                    let Some(answer) = self.as_guest(*conn, |game| game.native_pickup_answer(id))
                    else {
                        return;
                    };
                    let present = self.dropped.iter().any(|d| d.id == id);
                    let removed = !present || self.persist.removals.contains(&Target::Item(id));
                    (answer, present, removed)
                }
            };
            if removed {
                if let PickupSlot::Lifted { index, .. } = &mut slot
                    && present
                {
                    **index += 1;
                }
                return;
            }
            answer
        } else {
            TriState::Default
        };

        let live = match &slot {
            PickupSlot::Lifted { items, index, .. } => &items[**index],
            PickupSlot::Resident { .. } => {
                let Some(live) = self.dropped.iter().find(|item| item.id == id) else {
                    return;
                };
                live
            }
        };
        if !Self::pickup_decision(live.pickup_delay, live.target, player, answer) {
            if let PickupSlot::Lifted { index, .. } = &mut slot {
                **index += 1;
            }
            return;
        }

        let stack_replaced =
            listens_pre && (live.stack_revision != stack_revision || live.stack != captured);
        let thrower = live.thrower;
        let before = captured.count;
        let kind = captured.item.clone();
        let rest = match &slot {
            PickupSlot::Lifted { .. } => self.native_pickup_insert(&captured),
            PickupSlot::Resident { conn, .. } => {
                let Some(rest) = self.as_guest(*conn, |game| game.native_pickup_insert(&captured))
                else {
                    return;
                };
                rest
            }
        };
        if matches!(&rest, Some(r) if r.count >= before) {
            if let PickupSlot::Lifted { index, .. } = &mut slot {
                **index += 1;
            }
            return;
        }

        let listens_post = mod_api::__private::listened(mod_api::HookId::ItemPickedUpPost);
        let mut post_changed = false;
        let remaining = if listens_post {
            if !stack_replaced {
                let during = rest.clone().unwrap_or_else(|| {
                    let mut empty = captured.clone();
                    empty.count = 0;
                    empty
                });
                match &mut slot {
                    PickupSlot::Lifted { items, index, .. } => items[**index].stack = during,
                    PickupSlot::Resident { .. } => {
                        if let Some(at) = self.dropped.iter().position(|d| d.id == id) {
                            self.dropped[at].stack = during;
                        }
                    }
                }
            }
            let rest_before = rest.clone();
            let (told, present, removed) = match &mut slot {
                PickupSlot::Lifted { items, index, .. } => {
                    self.lend_lifted_item(items, **index, |game| {
                        game.native_pickup_post(id, &captured, rest)
                    })
                }
                PickupSlot::Resident { conn, .. } => {
                    let told = self
                        .as_guest(*conn, |game| game.native_pickup_post(id, &captured, rest))
                        .unwrap_or(None);
                    let present = self.dropped.iter().any(|d| d.id == id);
                    let removed = !present || self.persist.removals.contains(&Target::Item(id));
                    (told, present, removed)
                }
            };
            if removed {
                // The count the inventory took, from the captured original and what Post left
                // of it, before asking whether the entity is still alive (`ItemEntity.playerTouch`).
                let left = told.as_ref().filter(|s| s.count > 0);
                let taken = before.saturating_sub(left.map_or(0, |s| s.count));
                if taken > 0 {
                    self.record_item_pickup(
                        &slot,
                        &kind,
                        taken,
                        left.unwrap_or(&captured),
                        thrower,
                    );
                }
                if let PickupSlot::Lifted { index, .. } = &mut slot
                    && present
                {
                    **index += 1;
                }
                return;
            }
            post_changed = told != rest_before;
            told
        } else {
            rest
        };

        let remaining = remaining.filter(|s| s.count > 0 && &*s.item != "minecraft:air");
        let Some(rem) = remaining else {
            self.record_item_pickup(&slot, &kind, before, &captured, thrower);
            // `Inventory.add` emptied the captured stack, not a live replacement
            // installed by Pre. Only an explicit Post remainder edit overrides it.
            let keeps_replacement = |item: &ItemEntity| {
                stack_replaced
                    && !post_changed
                    && item.stack.count > 0
                    && &*item.stack.item != "minecraft:air"
            };
            match slot {
                PickupSlot::Lifted { items, index, feet } => {
                    let mut item = if keeps_replacement(&items[*index]) {
                        let taken = items[*index].clone();
                        *index += 1;
                        taken
                    } else {
                        items.remove(*index)
                    };
                    item.stack = captured;
                    self.native_take_item(item, feet);
                }
                PickupSlot::Resident { taken, .. } => {
                    // The take packet shrinks the entity's stack by its amount and removes it
                    // only when empty (`handleTakeItemEntity`): a replacement Pre installed
                    // survives it, so it is told as an animation of nothing taken from it.
                    let mut amount = before;
                    if let Some(at) = self.dropped.iter().position(|d| d.id == id) {
                        if keeps_replacement(&self.dropped[at]) {
                            amount = 0;
                        } else {
                            self.remove_dropped(at);
                        }
                    }
                    taken.push((id, amount));
                }
            }
            return;
        };

        if (!stack_replaced && !listens_post) || post_changed {
            match &mut slot {
                PickupSlot::Lifted { items, index, .. } => items[**index].stack = rem.clone(),
                PickupSlot::Resident { .. } => {
                    if let Some(at) = self.dropped.iter().position(|d| d.id == id) {
                        self.dropped[at].stack = rem.clone();
                    }
                }
            }
        }
        let taken_count = before.saturating_sub(rem.count);
        if taken_count > 0 {
            self.record_item_pickup(&slot, &kind, taken_count, &rem, thrower);
            match &mut slot {
                PickupSlot::Lifted { items, index, feet } => {
                    self.native_take_item(items[**index].clone(), *feet);
                }
                PickupSlot::Resident { taken, .. } => {
                    taken.push((id, taken_count));
                }
            }
        }
        if let PickupSlot::Lifted { index, .. } = &mut slot {
            **index += 1;
        }
    }

    fn record_item_pickup(
        &mut self,
        slot: &PickupSlot<'_>,
        kind: &str,
        count: u32,
        picked: &ItemStack,
        thrower: Option<[i32; 4]>,
    ) {
        match slot {
            PickupSlot::Lifted { .. } => {
                self.award_stat("picked_up", kind, count as i32);
                if let Some(thrower) = thrower {
                    self.trigger_thrown_picked_up_by_player(picked, thrower);
                }
            }
            PickupSlot::Resident { conn, .. } => {
                let _ = self.as_guest(*conn, |game| {
                    game.award_stat("picked_up", kind, count as i32);
                    if let Some(thrower) = thrower {
                        game.trigger_thrown_picked_up_by_player(picked, thrower);
                    }
                });
            }
        }
    }

    /// The take animation of `item` toward the host's player, and its sound at `feet`.
    fn native_take_item(&mut self, item: ItemEntity, feet: [f64; 3]) {
        self.add_pickup(
            crate::game::pickups::Taken::Item(Box::new(item)),
            crate::game::pickups::Collector::Player,
        );
        // handleTakeItemEntity: the random pitch of a full pickup.
        // `ClientPacketListener.random`, the listener's own stream, never the level's.
        let mut random = sunrise_client::random_source::create();
        let pitch = ((random.next_float() - random.next_float()) * 0.7 + 1.0) * 2.0;
        self.play_audio("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
    }

    /// Runs `notice` with the lifted `items[index]` lent back into `self.dropped`
    /// so mods can read and mutate the live entity handle. Preserves every
    /// in-place mutation on return, and never resurrects an item removed from
    /// `self.dropped`. Returns `(result, still_present_in_items, removed)`.
    fn lend_lifted_item<R>(
        &mut self,
        items: &mut JoinList<ItemEntity>,
        index: usize,
        notice: impl FnOnce(&mut Self) -> R,
    ) -> (R, bool, bool) {
        let (id, join) = (items[index].id, items[index].join_seq());
        let lent = items.extract(join).expect("in its pass");
        self.dropped.put_back(self.dropped.len(), lent);
        let result = notice(self);
        let removed = self.persist.removals.contains(&Target::Item(id));
        if let Some(back) = self.dropped.extract(join) {
            items.put_back(index, back);
            (result, true, removed)
        } else {
            (result, false, true)
        }
    }

    /// `item_picked_up` for the acting player touching the item entity
    /// `item`: the mods' answer, `Default` when no mod listens.
    pub(in crate::game) fn native_pickup_answer(&mut self, item: u32) -> TriState {
        let mut can_pickup = TriState::Default;
        if mod_api::__private::listened(mod_api::HookId::ItemPickedUp)
            && let Some(item) = self.native_item_handle(item)
        {
            let player = self.native_acting_id();
            self.native_item_picked_up(player, item, &mut can_pickup);
        }
        can_pickup
    }

    /// The item entity `id` as the hooks name it. It need not be in the
    /// level: a canceled toss hands out one the handle reads cannot find.
    pub(in crate::game) fn native_item_handle(&self, id: u32) -> Option<ItemEntityRef> {
        let epoch = self.native_lifecycle.epoch()?;
        Some(ItemEntityRef::new(EntityRef::new(epoch, id)))
    }

    /// `ItemEntity.playerTouch`'s rule for `player`: exactly zero pickup delay
    /// and no target or `player` as the target. NeoForge's `Pre` can veto this
    /// touch or override both conditions, without changing the delay.
    pub(in crate::game) fn pickup_decision(
        pickup_delay: i32,
        target: Option<[i32; 4]>,
        player: [i32; 4],
        answer: TriState,
    ) -> bool {
        answer.or(pickup_delay == 0 && target.is_none_or(|t| t == player))
    }
}
