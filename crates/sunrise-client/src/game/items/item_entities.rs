//! Dropped items and experience orbs: their tick (`tick_items`, `tick_dropped_items`,
//! `tick_experience`), the stacks a destroyed container lets fall, and spawning and merging orbs.
use super::*;
use crate::game::collision_source::Mover;
use sunrise_client::{entities::ItemCells, join_seq::JoinList};

/// A dropped item's `bobOffs` (its bob and spin phase) from its random's draw `drawn`, or under
/// `SUNRISE_STILL` from a random seeded by its position alone: a client seeds an item's random
/// with the server's entity id, which follows the order chunks load in and differs between
/// launches. The draw is made either way, so the item's random stays where vanilla's would be.
pub(super) fn bob_offset(position: Vec3, drawn: f32, still: bool) -> f32 {
    let drawn = if still {
        sunrise_client::mobs::entity_random(0, position).next_float()
    } else {
        drawn
    };
    drawn * std::f32::consts::PI * 2.0
}

/// The lifted list's removal mask, pending departures and merge grid, indexed by its current
/// row order. `cells` is refilled lazily (reusing map and bucket capacities) only when a merge
/// pass actually queries it, so `give_items_back` compacts `items` before any callback runs
/// without rebuilding `ItemCells` per expiry.
struct ItemPass {
    removed: Vec<bool>,
    cells: ItemCells,
    cells_dirty: bool,
    any_removed: bool,
    pending_left: Vec<u32>,
}

impl ItemPass {
    fn mark_removed(&mut self, index: usize, id: u32, at: Vec3, record_leave: bool) {
        if self.removed[index] {
            return;
        }
        self.removed[index] = true;
        self.any_removed = true;
        if !self.cells_dirty {
            self.cells.remove(index, at);
        }
        if record_leave {
            self.pending_left.push(id);
        }
    }
}

/// The items `give_items_back` takes again, where the item the loop was at now is (none once it
/// is gone), and the index the loop goes on from.
struct GivenBack {
    items: JoinList<ItemEntity>,
    at: Option<usize>,
    next: usize,
}

impl Game<'_> {
    pub(super) fn tick_items(&mut self) {
        let mut round = self.item_update_round();
        self.tick_dropped_items(&mut round);
        sunrise_threads::work::clear_work();
        self.drain_entity_inside();
    }
    /// An item that joined during this tick's entity pass: it first ticks on
    /// the next one.
    pub(super) fn joined_during_entity_pass(&self, id: u32) -> bool {
        id > self.ticked_entities_through || self.contact_drops.contains(&id)
    }
    pub(super) fn tick_dropped_items(&mut self, round: &mut native_ticks::TickRound) {
        // Item update hooks can discard a drop before the pass starts.
        self.native_item_removals_flush(|_| {});
        let mut items = self.dropped.lift();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut pass = ItemPass {
            removed: vec![false; items.len()],
            cells: ItemCells::default(),
            cells_dirty: true,
            any_removed: false,
            pending_left: Vec::new(),
        };
        // A LAN or server guest only shows the host's items.
        let merging = self.remote.is_none();
        // `Entity.checkBelowWorld`: below `Level.getMinY() - 64`.
        let void = f64::from(self.level_min_y() - 64);
        let mut obstacles =
            sunrise_client::entities::ObstacleGrid::new(self.entity_collisions(Mover::Passive));
        let mut nearby = Vec::new();
        let mut bubble_tops = Vec::new();
        let mut destroyed = Vec::new();
        let mut moves = ahead::Moves::work_out(self, &items, &obstacles, round, |item| {
            item.stack.count > 0
                && &*item.stack.item != "minecraft:air"
                && !self.joined_during_entity_pass(item.id)
                && ticking(
                    item.position.x.floor() as i32,
                    item.position.z.floor() as i32,
                )
        });
        let item_type = sunrise_threads::work::entity_id(native_ticks::ITEM);
        // When entity tick hooks are subscribed, each item hears its Pre right before its own
        // tick (followed by `on_entity_item_update`) and its Post right after it, with the list
        // given back for the call.
        let told = round.told_listening();
        let tell_pre = round.pre();
        let tell_post = round.post();
        let update_hook =
            merging && mod_api::__private::has_part_hook(mod_api::HookId::OnEntityItemUpdate);
        let leave_hook = merging && mod_api::__private::listened(mod_api::HookId::EntityLeaveLevel);
        let mut next = 0;
        'items: while next < items.len() {
            let mut i = next;
            next += 1;
            sunrise_threads::work::set_entity(item_type);
            if pass.removed[i] {
                continue;
            }
            // `addFreshEntity` during the entity pass: the item ticks from the
            // next tick on (`EntityTickList`).
            if merging && self.joined_during_entity_pass(items[i].id) {
                continue;
            }
            if !ticking(
                items[i].position.x.floor() as i32,
                items[i].position.z.floor() as i32,
            ) {
                continue;
            }
            if tell_pre || (told && update_hook) {
                let (id, join) = (items[i].id, items[i].join);
                let given = self.give_items_back(items, &mut pass, i, round, |game, round| {
                    if game
                        .dropped
                        .locate(join)
                        .is_some_and(|k| game.dropped[k].id == id)
                        && (!tell_pre || game.native_tick_pre_one(round, id, native_ticks::ITEM))
                    {
                        if tell_pre {
                            game.native_item_removals_flush(|_| {});
                        }
                        if update_hook
                            && let Some(k) = game
                                .dropped
                                .locate(join)
                                .filter(|&k| game.dropped[k].id == id)
                        {
                            if game.item_update_halts(k) {
                                round.halt(id);
                            }
                            if tell_post {
                                game.native_item_removals_flush(|_| {});
                                if !game
                                    .dropped
                                    .locate(join)
                                    .is_some_and(|k| game.dropped[k].id == id)
                                {
                                    game.native_tick_post_one(round, id, native_ticks::ITEM);
                                }
                            }
                        }
                    }
                });
                (items, next) = (given.items, given.next);
                let Some(at) = given.at else {
                    continue;
                };
                i = at;
            }
            if round.canceled(items[i].id) {
                continue;
            }
            let mut expiry_discarded = false;
            let mut env_destroyed = None;
            'tick: {
                if round.halted(items[i].id) {
                    break 'tick;
                }
                // `ItemEntity.tick`: after `onEntityItemUpdate`, an empty stack discards itself.
                if items[i].stack.count == 0 || &*items[i].stack.item == "minecraft:air" {
                    let (id, at) = (items[i].id, items[i].position);
                    pass.mark_removed(i, id, at, merging && !self.persist.live.is_empty());
                    break 'tick;
                }
                self.tick_saved_invulnerable_time(items[i].id);
                let (no_physics, alive, discarded, entered) = items
                    .with_one_and_rest(i, |item, before, after| {
                        let sources = |id| {
                            before
                                .iter()
                                .chain(after)
                                .zip(pass.removed[..i].iter().chain(&pass.removed[i + 1..]))
                                .find(|(p, removed)| p.id == id && !**removed && p.health > 0)
                                .map(|(p, _)| {
                                    (
                                        crate::game::commands::Target::Item(id),
                                        p.position.components(),
                                    )
                                })
                        };
                        let revision = self.world.revision;
                        let mut at_turn = |item: &mut ItemEntity| {
                            // `getEntityCollisions(entity, bounds.expandTowards(movement))`: the boxes around
                            // where the item can reach this tick.
                            obstacles.near(ahead::reach(item), &mut nearby);
                            // ItemEntity.tick's noPhysics (servers only): out of a block, a solid entity
                            // or the border, placed or closed on it, towards the closest open side (#1252).
                            let no_physics = merging
                                && item.inside_block(
                                    &self.world,
                                    self.collision,
                                    &nearby,
                                    Some(self.border.geometry()),
                                );
                            // `moveTowardsClosestSpace` draws the speed from the item's own random.
                            let escape_speed =
                                no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
                            let mut entered = Vec::new();
                            let (alive, discarded) = self.tick_item_with_block_contacts(
                                item,
                                escape_speed,
                                &nearby,
                                false,
                                Some(&sources),
                                &mut |splash| entered.push(splash),
                            );
                            (no_physics, alive, discarded, entered)
                        };
                        match moves.take(i, item, revision, &mut at_turn) {
                            Some(moved) => moved,
                            None => at_turn(item),
                        }
                    })
                    .expect("the item tick has a current row");
                let item = &mut items[i];
                // Entity.doWaterSplashEffect: the server plays the sound (broadcast
                // to clients). Its addParticle is empty.
                for splash in entered {
                    self.apply_item_splash(splash);
                }
                if discarded {
                    if merging
                        && item.health <= 0
                        && mod_api::__private::item_listens(
                            mod_api::HookId::ItemDestroyed,
                            &item.stack.item,
                        )
                    {
                        let damage_type = self
                            .item_destroyed_damage
                            .take_if(|(destroyed_id, _)| *destroyed_id == item.id)
                            .map(|(_, ty)| ty)
                            .unwrap_or_else(|| "minecraft:generic".to_owned());
                        if !pass.cells_dirty {
                            pass.cells.moved(i, item.previous, item.position);
                        }
                        env_destroyed = Some((item.id, item.position, damage_type));
                    } else {
                        self.item_destroyed_damage
                            .take_if(|(destroyed_id, _)| *destroyed_id == item.id);
                        let (id, at) = (item.id, item.previous);
                        pass.mark_removed(i, id, at, merging && !self.persist.live.is_empty());
                    }
                    break 'tick;
                }
                if merging && !no_physics {
                    self.item_landing_event(
                        item.id,
                        item.previous,
                        item.position,
                        item.ground.on_ground,
                    );
                    self.entity_powder_snow_particles(
                        item.previous,
                        item.position,
                        item.bounds(),
                        item.velocity,
                        false,
                    );
                }
                if !pass.cells_dirty {
                    pass.cells.moved(i, item.previous, item.position);
                }
                // Entity.handleOnAboveBubbleColumn: the server sends the particles of
                // the surface of a bubble column the item is at the top of.
                if merging {
                    bubble_tops.extend(sunrise_client::motion::bubble_column_surfaces(
                        &self.world,
                        self.pack.assets.registry,
                        self.res.facts(),
                        self.collision,
                        item.bounds(),
                    ));
                }
                // ItemEntity.tick: merge every 2 ticks while moving between
                // blocks, every 40 otherwise, before the lifetime check.
                let block = |v: Vec3| (v.x.floor(), v.y.floor(), v.z.floor());
                let rate = if block(item.previous) == block(item.position) {
                    40
                } else {
                    2
                };
                if merging
                    && (alive || item.age <= ItemEntity::LIFETIME && item.position.y >= void)
                    && item.ticks.is_multiple_of(rate)
                {
                    if pass.cells_dirty {
                        pass.cells.rebuild_live(&items, &pass.removed);
                        pass.cells_dirty = false;
                    }
                    let max_stack = |stack: &ItemStack| self.stack_max_size(stack).max(1);
                    let record_leave = merging && !self.persist.live.is_empty();
                    // A merge may take this item or a neighbour out of the pass.
                    sunrise_client::entities::merge_with_neighbours_recording(
                        &mut items,
                        &mut pass.removed,
                        &mut pass.cells,
                        i,
                        true,
                        &max_stack,
                        &mut |emptied_id| {
                            pass.any_removed = true;
                            if record_leave {
                                pass.pending_left.push(emptied_id);
                            }
                        },
                    );
                }
                let mut alive =
                    alive || (items[i].age < items[i].lifespan && items[i].position.y >= void);
                if merging
                    && !pass.removed[i]
                    && items[i].age >= items[i].lifespan
                    && items[i].position.y >= void
                {
                    // NeoForge `ItemEntity.tick` evaluates the left-hand `this.lifespan`
                    // before calling `EventHooks.onItemExpire(this)`.
                    let lifespan = items[i].lifespan;
                    let mut extra_life = 0;
                    if mod_api::__private::listened(mod_api::HookId::ItemExpire) {
                        let (id, join) = (items[i].id, items[i].join);
                        let given =
                            self.give_items_back(items, &mut pass, i, round, |game, round| {
                                if let Some(epoch) = game.native_lifecycle.epoch()
                                    && game
                                        .dropped
                                        .locate(join)
                                        .is_some_and(|k| game.dropped[k].id == id)
                                {
                                    let item = mod_api::ItemEntityRef::new(
                                        mod_api::EntityRef::new(epoch, id),
                                    );
                                    game.native_item_expire(item, &mut extra_life);
                                }
                                if tell_post {
                                    game.native_item_removals_flush(|_| {});
                                    if !game
                                        .dropped
                                        .locate(join)
                                        .is_some_and(|k| game.dropped[k].id == id)
                                    {
                                        game.native_tick_post_one(round, id, native_ticks::ITEM);
                                    }
                                }
                            });
                        (items, next) = (given.items, given.next);
                        let Some(at) = given.at else {
                            continue 'items;
                        };
                        i = at;
                    }
                    let item = &mut items[i];
                    // NeoForge `ItemEntity.tick`: Java int addition, then clamp to
                    // `0..=Short.MAX_VALUE - 1`, without rewinding `age`.
                    item.lifespan = lifespan
                        .wrapping_add(extra_life)
                        .clamp(0, ItemEntity::MAX_LIFESPAN);
                    alive =
                        item.age < item.lifespan && item.position.y >= void && item.stack.count > 0;
                    expiry_discarded = !alive;
                }
                if !alive || pass.removed[i] {
                    let (id, at) = (items[i].id, items[i].position);
                    pass.mark_removed(i, id, at, merging && !self.persist.live.is_empty());
                    break 'tick;
                }
                let item = &mut items[i];
                let from = item.previous;
                let to = item.position;
                if round.sections() {
                    let live = self.entity_section_step_visible(
                        item.id,
                        [from.x, from.y, from.z],
                        [to.x, to.y, to.z],
                    );
                    if !live {
                        let id = item.id;
                        pass.mark_removed(i, id, to, merging && !self.persist.live.is_empty());
                        break 'tick;
                    }
                } else {
                    self.entity_section_entered(
                        item.id,
                        [from.x, from.y, from.z],
                        [to.x, to.y, to.z],
                    );
                }
            }
            if let Some((id, at, damage_type)) = env_destroyed {
                let mut drops = Vec::new();
                let given = self.give_items_back(items, &mut pass, i, round, |game, round| {
                    drops = if let Some(k) =
                        native_ticks::place(&game.dropped, i, |item| item.id == id)
                    {
                        game.destroy_item_entity_drops(k, &damage_type)
                    } else {
                        Vec::new()
                    };
                    if tell_post && !game.dropped.iter().any(|item| item.id == id) {
                        game.native_tick_post_one(round, id, native_ticks::ITEM);
                    }
                });
                (items, next) = (given.items, given.next);
                for stack in drops {
                    destroyed.push((stack, at));
                }
                if let Some(at_idx) = given.at {
                    i = at_idx;
                    let (id, at) = (items[i].id, items[i].position);
                    pass.mark_removed(i, id, at, merging && !self.persist.live.is_empty());
                } else {
                    continue;
                }
            }
            // `Entity.discard` calls `ServerLevel.EntityCallbacks.onTrackingEnd` within
            // this turn. Restore surviving drops for the leave hook even without a Post.
            if tell_post || (expiry_discarded && leave_hook && !self.persist.live.is_empty()) {
                let id = items[i].id;
                let given = self.give_items_back(items, &mut pass, i, round, |game, round| {
                    if tell_post {
                        game.native_tick_post_one(round, id, native_ticks::ITEM);
                    }
                });
                (items, next) = (given.items, given.next);
            }
        }
        for pos in bubble_tops {
            self.bubble_column_particles(pos);
        }
        if pass.any_removed {
            let mut k = 0;
            items.retain(|_| {
                let keep = !pass.removed[k];
                k += 1;
                keep
            });
        }
        if !pass.pending_left.is_empty() {
            self.put_back_dropped(items);
            for id in pass.pending_left {
                self.native_entity_left(id);
            }
            self.native_item_removals_flush(|_| {});
            items = self.dropped.lift();
        }
        let Some(p) = self.player.as_ref() else {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [
                p.position.x - w - 1.0,
                p.position.y - 0.5,
                p.position.z - w - 1.0,
            ],
            [
                p.position.x + w + 1.0,
                p.position.y + f64::from(p.dimensions.height) + 0.5,
                p.position.z + w + 1.0,
            ],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let own = self.level.ensure_player_uuid();
        self.native_player_pickups(&mut items, (min, max), feet, own);
        self.put_back_dropped(items);
        self.spawn_destroyed_contents(destroyed);
    }
    /// Restores the surviving items for a lifecycle callback, compacting out removed rows
    /// first so `self.dropped` never exposes discarded drops to any host query, command
    /// selector or world pass while callbacks run. Announces pending departures in order,
    /// runs `call`, and retires any drops the callbacks discarded. `ItemCells` stays marked
    /// dirty on compaction or callback mutation and refills lazily (reusing its buffers) only
    /// when a later merge pass queries it.
    fn give_items_back(
        &mut self,
        mut items: JoinList<ItemEntity>,
        pass: &mut ItemPass,
        i: usize,
        round: &mut native_ticks::TickRound,
        call: impl FnOnce(&mut Self, &mut native_ticks::TickRound),
    ) -> GivenBack {
        let (id, join) = (items[i].id, items[i].join);
        let same_rows = self.dropped.is_empty();
        let compacted = pass.any_removed;
        if compacted {
            let mut k = 0;
            items.retain(|_| {
                let keep = !pass.removed[k];
                k += 1;
                keep
            });
            pass.any_removed = false;
            pass.cells_dirty = true;
        }
        let mut left = std::mem::take(&mut pass.pending_left);
        self.put_back_dropped(items);
        let restored = self.dropped.stamp();
        if !left.is_empty() {
            for &left_id in &left {
                self.native_entity_left(left_id);
            }
            self.native_item_removals_flush(|_| {});
            left.clear();
        }
        pass.pending_left = left;
        call(self, round);
        self.native_item_removals_flush(|_| {});
        let unchanged = same_rows && !compacted && self.dropped.stamp() == restored;
        let items = self.dropped.lift();
        if unchanged {
            return GivenBack {
                at: Some(i),
                next: i + 1,
                items,
            };
        }
        pass.removed.clear();
        pass.removed.resize(items.len(), false);
        pass.cells_dirty = true;
        let at = items.locate(join).filter(|&k| items[k].id == id);
        let next = match at {
            Some(at) => at + 1,
            None => items.partition_point(|item| item.join <= join),
        };
        GivenBack { items, at, next }
    }
    /// `ItemUtils.onContainerDestroyed`: spilled stacks use their own constructor
    /// motion (0.2 up and ±0.1 sideways), leaving the level random untouched.
    pub(super) fn spawn_destroyed_contents(&mut self, destroyed: Vec<(ItemStack, Vec3)>) {
        for (stack, at) in destroyed {
            self.spawn_item_own(stack, [at.x, at.y, at.z], 0);
        }
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes; each draws the id
    /// `tryMergeToExisting` compares from the level random, then moves with
    /// its own random.
    pub(super) fn spawn_orbs(&mut self, at: [f64; 3], value: u32) {
        self.spawn_orbs_toward(at, [0.0; 3], value);
    }
    /// `ExperienceOrb.awardWithDirection`: `award` whose fresh orbs start
    /// `roughly` that way (`new ExperienceOrb(level, pos, roughly, value)`).
    pub(super) fn spawn_orbs_toward(&mut self, at: [f64; 3], roughly: [f64; 3], mut value: u32) {
        while value > 0 {
            let v = ExperienceOrb::split(value);
            value -= v;
            let merge_id = self.world.random.next_int_bound(40).unwrap_or(0);
            if !self.merge_new_orb(at, v, merge_id) {
                self.add_orb_toward(at, roughly, v);
            }
        }
    }
    /// `ExperienceOrb.tryMergeToExisting`: an orb of the same value within a
    /// 1 block box of `at`, whose id is a multiple of 40 from `merge_id`, takes
    /// the new one as one more in its count (and restarts its age); otherwise
    /// a fresh orb spawns.
    pub(super) fn award_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) {
        if !self.merge_new_orb(at, value, merge_id) {
            self.add_orb(at, value);
        }
    }
    /// `tryMergeToExisting`: true when an orb took the new one.
    pub(super) fn merge_new_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) -> bool {
        let centre = Vec3::new(at[0], at[1], at[2]);
        let half = Vec3::new(0.5, 0.5, 0.5);
        let area = Aabb::new(centre.subtract(half), centre.add_vector(half));
        // A server's orbs are its own; a LAN or server guest only shows them.
        if self.remote.is_none() {
            for i in 0..self.orbs.len() {
                let o = &self.orbs[i];
                if !o.bounds().intersects(area) {
                    continue;
                }
                let vanilla = o.can_merge(merge_id as u32, value as i32);
                let (join, id, count, age) = (o.join, o.id, o.count, o.age);
                let orb_value = o.value;
                self.journal_orb_merge(join, (orb_value, count, age));
                if !self.orb_merges(id, orb_value, count, 0, value as i32, 1, vanilla) {
                    self.restore_orb(join, orb_value, count, age);
                    continue;
                }
                let Some(i) = self.orbs.locate(join) else {
                    continue;
                };
                let o = &mut self.orbs[i];
                // A mod's write in the merge hook is the merged orb; else one more in its count.
                if (o.value, o.count) == (orb_value, count) {
                    if count >= crate::orb_limits::MAX_COUNT {
                        continue;
                    }
                    o.count += 1;
                }
                o.age = 0;
                return true;
            }
        }
        false
    }
    /// Puts back what a merge hook may have written to the orb `join`: a refused merge
    /// leaves both orbs as they were.
    fn restore_orb(&mut self, join: crate::join_seq::JoinSeq, value: i32, count: u32, age: i32) {
        if let Some(i) = self.orbs.locate(join) {
            let orb = &mut self.orbs[i];
            (orb.value, orb.count, orb.age) = (value, count, age);
        }
    }
    /// Whether orb `into` takes orb `from` (0: a spawning orb of `from_value`),
    /// given vanilla's `canMerge` answer: the mods may flip it, then cancel the merge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn orb_merges(
        &mut self,
        into: u32,
        into_value: i32,
        into_count: u32,
        from: u32,
        from_value: i32,
        from_count: u32,
        vanilla: bool,
    ) -> bool {
        let dimension = self.dim.id();
        let mut can_merge = vanilla;
        self.native_experience_orb_can_merge(
            dimension,
            into,
            from,
            into_value,
            from_value,
            &mut can_merge,
        );
        if !can_merge {
            return false;
        }
        let mut canceled = false;
        self.native_experience_orb_merge(mod_api::__private::ExperienceOrbMergeCall {
            dimension,
            into,
            from,
            into_value,
            into_count,
            from_value,
            from_count,
            canceled: &mut canceled,
        });
        !canceled
    }
    /// `ExperienceOrb.tick`'s scan before movement, once every twenty ticks.
    /// Hooks can change the resident list, so retain join identities, not indices.
    pub(super) fn merge_orb(&mut self, i: usize) {
        if !self.orbs[i].ticks.is_multiple_of(20) {
            return;
        }
        let into_join = self.orbs[i].join;
        let area = self.orbs[i].bounds().inflate(Vec3::new(0.5, 0.5, 0.5));
        let candidates: Vec<_> = self
            .orbs
            .iter()
            .filter(|o| o.join != into_join && o.bounds().intersects(area))
            .map(|o| o.join)
            .collect();
        for from_join in candidates {
            let Some(i) = self.orbs.locate(into_join) else {
                return;
            };
            let Some(j) = self.orbs.locate(from_join) else {
                continue;
            };
            let (a, b) = (&self.orbs[i], &self.orbs[j]);
            let (into, into_value, into_count, into_age) = (a.id, a.value, a.count, a.age);
            let (from, from_value, from_count) = (b.id, b.value, b.count);
            let vanilla = b.can_merge(a.id, a.value);
            if !self.orb_merges(
                into, into_value, into_count, from, from_value, from_count, vanilla,
            ) {
                self.restore_orb(into_join, into_value, into_count, into_age);
                continue;
            }
            let Some(i) = self.orbs.locate(into_join) else {
                return;
            };
            let written = (self.orbs[i].value, self.orbs[i].count) != (into_value, into_count);
            let fresh = self.orbs.locate(from_join).filter(|&j| {
                let (a, b) = (&self.orbs[i], &self.orbs[j]);
                (b.value, b.count) == (from_value, from_count)
                    && b.bounds()
                        .intersects(a.bounds().inflate(Vec3::new(0.5, 0.5, 0.5)))
            });
            // A mod's write in the merge hook is the merged orb; else `into` adds the
            // source's count. A stale source, or a count past what a save holds, refuses
            // the merge and puts a written `into` back.
            let count = if written {
                Some(self.orbs[i].count)
            } else {
                into_count
                    .checked_add(from_count)
                    .filter(|&c| c <= crate::orb_limits::MAX_COUNT)
            };
            let (Some(j), Some(count)) = (fresh, count) else {
                self.restore_orb(into_join, into_value, into_count, into_age);
                continue;
            };
            let age = self.orbs[j].age;
            self.orbs[i].count = count;
            self.orbs[i].age = self.orbs[i].age.min(age);
            self.orbs.remove(j);
        }
    }
    /// `Entity.unstuckIfPossible`, which `ExperienceOrb`'s constructor runs on an
    /// orb whose box is in a block: the free spot nearest its box's centre. An
    /// orb in open air keeps its position.
    pub(super) fn unstuck_orb(&self, at: [f64; 3]) -> [f64; 3] {
        let b = self.res.behaviours();
        let border = self.border_bounds();
        ExperienceOrb::unstuck(Vec3::new(at[0], at[1], at[2]), &border, |p| {
            b.collision_shape(self.world.block(p.x, p.y, p.z))
                .map(|s| s.moved(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))))
        })
        .map_or(at, |p| [p.x, p.y, p.z])
    }
    /// `new ExperienceOrb(level, pos, Vec3.ZERO, value)`: yaw and motion from
    /// the orb's own random.
    pub(super) fn add_orb(&mut self, at: [f64; 3], value: u32) {
        self.add_orb_toward(at, [0.0; 3], value);
    }
    /// `new ExperienceOrb(level, pos, roughly, value)`: a random motion that
    /// points against `roughly` is turned round, and the orb starts half its
    /// size (0.25) along `roughly`, unstuck if it collides there.
    pub(super) fn add_orb_toward(&mut self, at: [f64; 3], roughly: [f64; 3], value: u32) {
        let len_sqr = roughly.iter().map(|v| v * v).sum::<f64>();
        // Vec3.normalize: a vector shorter than 1e-5 has no direction.
        let len = len_sqr.sqrt();
        let unit = if len < 1.0e-5 {
            [0.0; 3]
        } else {
            roughly.map(|v| v / len)
        };
        let at = self.unstuck_orb([0, 1, 2].map(|a| at[a] + unit[a] * 0.25));
        let mut r = random_source::entity();
        let _yaw = r.next_float();
        let p = Vec3::new(at[0], at[1], at[2]);
        let mut velocity = Vec3::new(
            (r.next_double() * 0.2 - 0.1) * 2.0,
            r.next_double() * 0.2 * 2.0,
            (r.next_double() * 0.2 - 0.1) * 2.0,
        );
        let dot = roughly[0] * velocity.x + roughly[1] * velocity.y + roughly[2] * velocity.z;
        if len_sqr > 0.0 && dot < 0.0 {
            velocity = Vec3::new(-velocity.x, -velocity.y, -velocity.z);
        }
        self.next_entity += 1;
        self.orbs.push(ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: self.next_entity,
            position: p,
            previous: p,
            velocity,
            // `getExperienceValue` sizes: 1 to 2477.
            value: value as i32,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            following: None,
            fire_ticks: 0,
        });
        // ServerLevel.addFreshEntity.
        self.native_joins_flush(false, 0);
    }
    /// The orbs' tick, but for the movement: that is `run_ordered_ticks`'s,
    /// in join order with the other kinds.
    pub(in crate::game) fn tick_experience(&mut self) {
        if self.xp_delay > 0 {
            self.xp_delay -= 1;
        }
        let spectator = self.spectator();
        let player = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator);
        // Player.aiStep: standing, the pickup area is the player's box
        // inflated by (1, 0.5, 1); riding, the union with the vehicle's box
        // inflated by (1, 0, 1).
        let bounds = player.map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            let own = Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ),
            );
            let area = match self.riding_box() {
                Some(vehicle) => own.union_bounds(vehicle).inflate(Vec3::new(1.0, 0.0, 1.0)),
                None => own.inflate(Vec3::new(1.0, 0.5, 1.0)),
            };
            (
                [
                    area.min_position().x,
                    area.min_position().y,
                    area.min_position().z,
                ],
                [
                    area.max_position().x,
                    area.max_position().y,
                    area.max_position().z,
                ],
            )
        });
        let mut orbs = self.orbs.lift();
        #[cfg(test)]
        crate::game::entity::taken_pass_tests::after_take(self);
        let mut gained = 0i32;
        let mut taken = Vec::new();
        if let Some((min, max)) = bounds {
            orbs.retain_mut(|o| {
                let touching = o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2];
                if touching && self.xp_delay == 0 {
                    // ExperienceOrb.playerTouch: one orb every 2 ticks; a
                    // merged orb gives its value once per count.
                    self.xp_delay = 2;
                    gained = gained.saturating_add(o.value);
                    taken.push(o.clone());
                    o.count -= 1;
                    return o.count > 0;
                }
                true
            });
        }
        self.orbs
            .restore(orbs, sunrise_client::join_seq::RestoreOrder::SurvivorsFirst);
        // ExperienceOrb.playerTouch repairs with the signed value, whatever it is.
        if !taken.is_empty() {
            gained = self.mending(gained);
        }
        for o in taken {
            // ClientPacketListener.handleTakeItemEntity: every orb taken sounds
            // at the orb, whether mending took its points or not.
            let mut r = random_source::create();
            let pitch = (r.next_float() - r.next_float()) * 0.35 + 0.9;
            let at = [o.position.x, o.position.y, o.position.z];
            self.play_audio(
                "minecraft:entity.experience_orb.pickup",
                Some(at),
                0.1,
                pitch,
            );
            self.add_pickup(
                crate::game::pickups::Taken::Orb(Box::new(o)),
                crate::game::pickups::Collector::Player,
            );
        }
        if gained > 0 {
            self.give_experience_points(gained);
        }
    }

    /// `Player.dropEquipment` for the player who died: unless keep_inventory
    /// is on, `destroyVanishingCursedItems`, then `Inventory.dropAll` from
    /// `at` (the eyes less 0.3): slots 0..36, armor feet to head (36..39),
    /// the off hand (40), each thrown in a random direction. The mods hear
    /// the drops (`native_living_drops`) before they join the level.
    pub(super) fn drop_inventory(&mut self, at: [f64; 3], dying: &creatures::DyingDrops<'_>) {
        let mut stacks: Vec<ItemStack> = Vec::new();
        if !self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::KEEP_INVENTORY)
        {
            stacks.extend(self.inventory.iter_mut().filter_map(Option::take));
            stacks.extend(self.armor.iter_mut().rev().filter_map(Option::take));
            stacks.extend(self.offhand.take());
            stacks.extend(self.equipment.take_all());
            stacks.retain(|s| s.enchantment_level("minecraft:vanishing_curse") == 0);
            self.apply_equipment();
        }
        let stacks = stacks.into_iter().map(|stack| (stack, at, 0)).collect();
        let mut random = random_source::entity();
        self.drop_death_items(dying, stacks, 40, |game| {
            Some(sunrise_client::entities::drop_motion(
                &mut random,
                game.yaw,
                game.pitch,
                true,
            ))
        });
    }

    /// Item entity `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_dropped(&mut self, i: usize) -> ItemEntity {
        let e = self.dropped.remove(i);
        self.native_entity_left(e.id);
        e
    }

    /// Experience orb `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_orb(&mut self, i: usize) -> ExperienceOrb {
        let o = self.orbs.remove(i);
        self.native_entity_left(o.id);
        o
    }

    /// The item entity `spawn_item` just added, unless a native mod kept it
    /// out of the level (`native_entity_join_level`).
    pub(super) fn just_spawned_item(&mut self) -> Option<&mut ItemEntity> {
        let id = self.next_entity;
        self.dropped.last_mut().filter(|d| d.id == id)
    }

    /// `new ItemEntity(level, x, y, z, stack, dx, dy, dz)`.
    pub(super) fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: i32,
    ) {
        self.add_item_entity(stack, position, Some(velocity), pickup_delay);
    }

    /// `new ItemEntity(level, x, y, z, stack)`: the item leaves with its own
    /// random's motion, which this returns.
    pub(in crate::game) fn spawn_item_own(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        pickup_delay: i32,
    ) -> [f64; 3] {
        self.add_item_entity(stack, position, None, pickup_delay)
    }

    /// The dispenser constructs default motion before replacing it with the throw.
    pub(super) fn spawn_item_thrown(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
    ) {
        let random = spill::prepare_item_random(None);
        self.add_item_entity_prepared(stack, position, Some(velocity), 0, Some(random));
    }

    pub(super) fn add_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: i32,
    ) -> [f64; 3] {
        self.add_item_entity_prepared(stack, position, velocity, pickup_delay, None)
    }

    /// Block.popResource
    pub(super) fn pop_resource(&mut self, pos: [i32; 3], stack: ItemStack) {
        let block_drops = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::BLOCK_DROPS);
        if let Some(at) = spill::admitted_pop_position(&mut self.world.random, pos, block_drops) {
            self.spawn_item_own(stack, at, 10);
        }
    }

    /// `Containers.dropItemStack`: a crafting remainder leaves from inside the
    /// block, split by the level random, and can be picked up at once. Unlike
    /// `Block.popResource` it has no `block_drops` test.
    pub(super) fn drop_item_stack_containers(&mut self, pos: [i32; 3], stack: ItemStack) {
        let count = stack.count;
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        for piece in
            sunrise_world::level::removal::drop_item_stack_at(&mut self.world.random, at, count)
        {
            self.spawn_item_thrown(
                ItemStack {
                    count: piece.count,
                    ..stack.clone()
                },
                piece.pos,
                piece.motion,
            );
        }
    }

    /// `Block.popResourceFromFace`: the loot leaves the clicked face with the
    /// face's own motion instead of the item's random one.
    pub(super) fn pop_resource_from_face(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        stack: ItemStack,
    ) {
        let (p, v) = spill::pop_position_from_face(&mut self.world.random, pos, face);
        // The private overload checks the gamerule after the offsets are drawn.
        if !self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        self.spawn_item(stack, p, v, 10);
    }

    /// Player.drop: thrown from the eyes along the view direction. True when
    /// the selected stack's `on_dropped_by_player` Part kept it in the slot
    /// (`ServerPlayer.drop(boolean)`, as NeoForge asks it).
    pub(in crate::game) fn drop_held_item(&mut self, all: bool) -> bool {
        if self.eye_position().is_none() {
            return false;
        }
        let spectator = self.spectator();
        if !spectator && self.native_dropped_by_player_event(all) {
            return true;
        }
        let Some(thrown) = sunrise_client::items::take_held_drop(
            &mut self.inventory[self.selected],
            all,
            spectator,
        ) else {
            return false;
        };
        self.player_drop(thrown);
        false
    }

    /// A guest's drop key. A drop the held item's Part refused sends the
    /// inventory back, undoing the removal the client predicted.
    pub(in crate::game) fn guest_drop_held(&mut self, gi: usize, conn: u32, all: bool) {
        let kept = self.as_guest(conn, |g| g.drop_held_item(all)) == Some(true);
        if let Some(host) = self.lan.as_mut() {
            if kept {
                host.hub.send_inventory(gi);
            } else {
                host.hub.guests.update(gi, |g| g.told_selected());
            }
        }
    }

    /// `Player.createItemStackToDrop`: an item entity thrown from the eyes
    /// along the view direction. `thrown_from_hand` (`drop(stack, true)`, the
    /// drop key) names the player as the thrower; `drop(stack, false)`, for
    /// what no longer fits in the inventory, does not.
    pub(super) fn throw_from_eyes(
        &mut self,
        thrown: ItemStack,
        eye: [f64; 3],
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        let v = self.player_drop_motion(randomly);
        let at = [eye[0], eye[1] - f64::from(0.3f32), eye[2]];
        let uuid = thrown_from_hand.then(|| self.acting_player_uuid());
        // Only `drop(.., false)` tosses ask `item_toss`: a death's scatter (`randomly`) does not.
        let id = if randomly {
            self.spawn_item(thrown, at, v, 40);
            self.next_entity
        } else {
            self.toss_item(thrown, at, v, uuid)?
        };
        let item = self.dropped.iter_mut().rev().find(|d| d.id == id)?;
        item.thrower = uuid;
        Some(item.id)
    }

    /// `LivingEntity.createItemStackToDrop` draws the spread from the player's own random.
    pub(super) fn player_drop_motion(&mut self, randomly: bool) -> [f64; 3] {
        match self.player.as_mut() {
            Some(player) => sunrise_client::entities::drop_motion(
                &mut player.random,
                self.yaw,
                self.pitch,
                randomly,
            ),
            None => sunrise_client::entities::drop_motion(
                &mut random_source::entity(),
                self.yaw,
                self.pitch,
                randomly,
            ),
        }
    }

    /// `FurnaceResultSlot.checkTakeAchievements`: the player took from the output of the
    /// furnace at `pos`, which pops the experience of what it made at them
    /// (`awardUsedRecipesAndPopExperience`) and forgets it. Returns the used recipes' ids, which
    /// the caller awards to the extracting player (`ServerPlayer.awardRecipes`) in the body that
    /// took the output.
    #[must_use = "the used recipes are awarded to the extracting player"]
    pub(super) fn award_furnace_recipes(
        &mut self,
        pos: (i32, i32, i32),
        at: [f64; 3],
    ) -> Vec<String> {
        let (used, recipes) = match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::Furnace(f)) => {
                let used = f.recipe_experience(world_recipes!(self));
                let mut recipes: Vec<String> = f.recipes_used.keys().cloned().collect();
                recipes.sort();
                f.recipes_used.clear();
                (used, recipes)
            }
            _ => return Vec::new(),
        };
        for (count, experience) in used {
            let xp = sunrise_client::containers::create_experience(
                count,
                experience,
                &mut self.world.random,
            );
            self.spawn_orbs(at, xp);
        }
        recipes
    }
}

#[cfg(test)]
mod destroyed_contents_tests {
    use super::*;
    use crate::game::game_tests::tests::server_game;
    use sunrise_client::items::ItemStack;
    use sunrise_core::random::RandomSource;

    /// The motion of each spilled stack, in the order the spill made them.
    fn spilled_motions(game: &Game<'_>) -> Vec<[f64; 3]> {
        game.dropped
            .iter()
            .map(|item| [item.velocity.x, item.velocity.y, item.velocity.z])
            .collect()
    }

    /// `ItemUtils.onContainerDestroyed` makes each stack with
    /// `new ItemEntity(level, x, y, z, stack)`, whose motion is 0.2 up and
    /// ±0.1 sideways from the *item's* random (ItemEntity.java:61-66), and
    /// draws nothing from the level's. Every destroyed container funnels here:
    /// lava, fire and cactus in the item's environment tick, an explosion, and
    /// the damage command.
    #[test]
    #[ignore = "daily"]
    fn spilled_stacks_rise_from_their_own_randoms() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let at = Vec3::new(8.5, 70.0, 8.5);
        let stacks = ["minecraft:stone", "minecraft:dirt", "minecraft:glass"]
            .into_iter()
            .map(|id| (ItemStack::new(id, 1), at))
            .collect();
        let mut level = game.world.random.clone();
        game.spawn_destroyed_contents(stacks);
        let motions = spilled_motions(&game);
        assert_eq!(motions.len(), 3);
        for motion in &motions {
            assert_eq!(motion[1], 0.2, "the four-argument constructor rises 0.2");
            assert!((-0.1..0.1).contains(&motion[0]), "{motion:?}");
            assert!((-0.1..0.1).contains(&motion[2]), "{motion:?}");
        }
        // Three stacks, three different sideways motions: each drew its own.
        assert_ne!(motions[0], motions[1]);
        assert_ne!(motions[1], motions[2]);
        assert_ne!(motions[0], motions[2]);
        // The spill leaves the level's random alone.
        assert_eq!(game.world.random.next_double(), level.next_double());
        assert_eq!(game.world.random.next_double(), level.next_double());
    }

    /// The damage command is one of those callers: killing a shulker box item
    /// spills its contents with the same upward motion.
    #[test]
    #[ignore = "daily"]
    fn a_damaged_shulker_box_spills_its_stacks_upward() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
        );
        let feet = Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(sunrise_client::motion::PlayerMotion::new(game.motion, feet).unwrap());
        game.update_chunk_tickets();
        let boxed = sunrise_client::item_data::parse_stack(
            r#"{id:"minecraft:shulker_box",count:1,components:{"minecraft:container":[
                {slot:0,item:{id:"minecraft:stone",count:2}},
                {slot:4,item:{id:"minecraft:dirt",count:1}},
                {slot:8,item:{id:"minecraft:glass",count:3}}]}}"#,
        )
        .unwrap()
        .unwrap();
        game.spawn_item(boxed, [8.5, 70.0, 8.5], [0.0; 3], 0);
        assert_eq!(game.dropped.len(), 1);
        assert!(game.run_command("damage @e[type=minecraft:item,limit=1] 100 minecraft:generic"));
        let mut spilled: Vec<_> = game
            .dropped
            .iter()
            .map(|item| (item.stack.item.to_string(), item.stack.count))
            .collect();
        spilled.sort();
        assert_eq!(
            spilled,
            [
                ("minecraft:dirt".to_owned(), 1),
                ("minecraft:glass".to_owned(), 3),
                ("minecraft:stone".to_owned(), 2)
            ]
        );
        for item in &game.dropped {
            assert_eq!(item.velocity.y, 0.2, "{item:?}");
        }
    }
}

#[cfg(test)]
mod escape_tests {
    use crate::game::game_tests::tests::server_game;
    use sunrise_client::items::ItemStack;
    use sunrise_core::random::RandomSource;

    /// `Entity.moveTowardsClosestSpace` uses the item's own random, never the
    /// level's random. Seeded Java values pin vanilla's float arithmetic (#13043).
    #[test]
    #[ignore = "daily"]
    fn an_item_escapes_a_block_at_its_own_randoms_speed() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let feet = sunrise_core::geometry::Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(sunrise_client::motion::PlayerMotion::new(game.motion, feet).unwrap());
        super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            &mut game,
            sunrise_core::position::ChunkPos::new(0, 0),
        );
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(8, 8));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(8, 64, 8, stone).unwrap();
        let stack = ItemStack::new("minecraft:stone", 2);
        game.spawn_item(stack, [8.5, 64.4, 8.5], [0.0, 0.0, 0.0], 10);
        let item = game.dropped.last_mut().unwrap();
        item.random = sunrise_core::random::LegacyRandom::new(12345);
        let mut level = game.world.random.clone();
        game.ticked_entities_through = u32::MAX;
        game.tick_items();
        let item = game.dropped.last().unwrap();
        // Up is the nearest open side. Independent Java Random(12345):
        // nextFloat()*0.2F+0.1F, widened before multiplying by widened 0.98F.
        assert_eq!(item.velocity.y, f64::from_bits(0x3fc5_9ef4_570f_5500));
        assert_eq!(item.random.clone().next_float().to_bits(), 0x3f03_61b3);
        assert_eq!(game.world.random.next_float(), level.next_float());
    }
}

#[cfg(test)]
mod expire_tests {
    use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
    use crate::game::{Dim, Game};
    use mod_api::{
        Compound, EntityLeaveLevelEvent, EntityTickPostEvent, EntityTickPreEvent, HostSide,
        HostTransaction, ItemExpireEvent, ModEntry, ServerHost, Tag,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicU32, Ordering},
    };
    use sunrise_client::chunk::Chunk;
    use sunrise_client::items::ItemStack;
    use sunrise_client::motion::PlayerMotion;
    use sunrise_core::{geometry::Vec3, position::ChunkPos};

    const AT: [f64; 3] = [2.5, 64.0, 2.5];

    fn expiry_level(game: &mut Game<'_>) {
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(game.entity_ticking_area()(2, 2));
    }

    fn drop_at_age(game: &mut Game<'_>, name: &str, age: i32) -> u32 {
        game.push_item_entity(ItemStack::new(name, 1), AT, Some([0.0; 3]), 0, None);
        let item = game.dropped.last_mut().unwrap();
        item.age = age;
        let id = item.id;
        game.native_joins_flush(false, 0);
        game.ticked_entities_through = id;
        id
    }

    #[derive(Default)]
    struct ExpireLog {
        seen: Vec<(u32, Option<[f64; 3]>, Option<i32>)>,
        leaves: Vec<(u32, String)>,
    }

    struct ExpireListener {
        extra: Arc<AtomicI32>,
        log: Arc<Mutex<ExpireLog>>,
        record_leave: bool,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ExpireListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let entity = event.item.entity();
            let pos = entity.position(host);
            let age = event.item.age(host);
            self.log.lock().unwrap().seen.push((entity.id(), pos, age));
            event.extra_life = event
                .extra_life
                .wrapping_add(self.extra.load(Ordering::Relaxed));
        }

        fn entity_leave_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            if self.record_leave {
                self.log
                    .lock()
                    .unwrap()
                    .leaves
                    .push((event.entity, event.kind.to_owned()));
            }
        }
    }

    /// NeoForge `EventHooks.onItemExpire` / `ItemEntity.tick`: listeners see the live
    /// resident item, add to `extra_life`, extend `lifespan` without rewinding `age`,
    /// clamp `lifespan` to `Short.MAX_VALUE - 1` (32766), and fire `entity_leave_level`
    /// once at the removal site when discarded.
    #[test]
    #[ignore = "daily"]
    fn item_expire_extends_lifespan_without_rewinding_age_and_caps_at_32766() {
        let log = Arc::new(Mutex::new(ExpireLog::default()));
        let extra_a = Arc::new(AtomicI32::new(60));
        let extra_b = Arc::new(AtomicI32::new(40));
        let mods: mod_api::Mods = vec![
            ModEntry::new(
                "item-expire-a",
                Box::new(ExpireListener {
                    extra: extra_a.clone(),
                    log: log.clone(),
                    record_leave: true,
                }),
            ),
            ModEntry::new(
                "item-expire-b",
                Box::new(ExpireListener {
                    extra: extra_b.clone(),
                    log: log.clone(),
                    record_leave: false,
                }),
            ),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let id = drop_at_age(game, "minecraft:diamond", 5999);

                // First expiry at age 6000: listeners add 60 + 40 -> lifespan 6100, age stays 6000.
                game.tick_items();
                let item = game
                    .dropped
                    .iter()
                    .find(|item| item.id == id)
                    .expect("extended item survives");
                assert_eq!(item.age, 6000);
                assert_eq!(item.lifespan, 6100);
                {
                    let guard = log.lock().unwrap();
                    assert_eq!(guard.seen.len(), 2);
                    for &(seen_id, pos, age) in &guard.seen {
                        assert_eq!(seen_id, id);
                        assert!(pos.is_some(), "item entity is resident during item_expire");
                        assert_eq!(age, Some(6000));
                    }
                    assert!(guard.leaves.is_empty());
                }

                // The sum 6100 + 30000 fits a Java int but exceeds Short.MAX_VALUE - 1.
                extra_a.store(30000, Ordering::Relaxed);
                extra_b.store(0, Ordering::Relaxed);
                game.dropped.last_mut().unwrap().age = 6099;
                game.tick_items();
                let item = game
                    .dropped
                    .iter()
                    .find(|item| item.id == id)
                    .expect("clamped item survives at age 6100 < 32766");
                assert_eq!(item.age, 6100);
                assert_eq!(item.lifespan, 32766);

                // Reaching 32766 discards even when listeners still request extra life;
                // entity_leave_level fires immediately at removal and does not duplicate on flush.
                game.dropped.last_mut().unwrap().age = 32765;
                game.tick_items();
                assert!(!game.dropped.iter().any(|item| item.id == id));
                assert_eq!(
                    log.lock().unwrap().leaves.as_slice(),
                    &[(id, "minecraft:item".to_owned())]
                );
                game.native_leaves_flush();
                assert_eq!(
                    log.lock().unwrap().leaves.len(),
                    1,
                    "native_leaves_flush does not duplicate entity_leave_level"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_expire_discards_after_insufficient_negative_or_wrapping_extensions() {
        let log = Arc::new(Mutex::new(ExpireLog::default()));
        let extra = Arc::new(AtomicI32::new(0));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "item-expire-discard",
            Box::new(ExpireListener {
                extra: extra.clone(),
                log: log.clone(),
                record_leave: true,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                for (age, lifespan, extension) in [
                    (6499, 6000, 100),      // Age 6500 still exceeds the extended lifespan of 6100.
                    (5999, 6000, -500),     // Age 6000 exceeds the shortened lifespan of 5500.
                    (6099, 6100, i32::MAX), // Java int sum -2147477549 clamps to zero.
                ] {
                    extra.store(extension, Ordering::Relaxed);
                    let id = drop_at_age(game, "minecraft:stone", age);
                    game.dropped.last_mut().unwrap().lifespan = lifespan;
                    game.tick_items();
                    assert!(game.dropped.is_empty(), "extension {extension}");
                    assert_eq!(
                        std::mem::take(&mut log.lock().unwrap().leaves),
                        [(id, "minecraft:item".to_owned())]
                    );
                    game.native_leaves_flush();
                    assert!(log.lock().unwrap().leaves.is_empty());
                }
            });
        });
    }

    #[derive(Default)]
    struct LeaveObservation {
        next_drop: AtomicU32,
        heard: Mutex<Vec<(u32, Option<i32>)>>,
    }

    struct LeaveObserver(Arc<LeaveObservation>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for LeaveObserver {
        fn entity_leave_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            let next_age = host
                .entity(self.0.next_drop.load(Ordering::Relaxed))
                .and_then(|entity| host.item_entity(entity))
                .and_then(|item| item.age(host));
            self.0.heard.lock().unwrap().push((event.entity, next_age));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn expiry_leave_runs_before_the_next_due_drop_ticks() {
        let observed = Arc::new(LeaveObservation::default());
        // With no item_expire or per-entity tick listener, only removal itself lends the list.
        let mods: mod_api::Mods = vec![ModEntry::new(
            "expiry-leave-order",
            Box::new(LeaveObserver(observed.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let first = drop_at_age(game, "minecraft:stone", 5999);
                let next = drop_at_age(game, "minecraft:diamond", 5999);
                observed.next_drop.store(next, Ordering::Relaxed);

                game.tick_items();
                assert_eq!(
                    *observed.heard.lock().unwrap(),
                    [(first, Some(5999)), (next, None)]
                );
                assert!(game.dropped.is_empty());
                game.native_leaves_flush();
                assert_eq!(observed.heard.lock().unwrap().len(), 2);
            });
        });
    }

    #[derive(Default)]
    struct DiscardLog {
        drops: [u32; 2],
        commit: Option<bool>,
        discarded: Option<[bool; 2]>,
        expires: Vec<u32>,
        leaves: Vec<u32>,
    }

    struct DiscardOnExpiry(Arc<Mutex<DiscardLog>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DiscardOnExpiry {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let id = event.item.entity().id();
            let (drops, commit) = {
                let mut log = self.0.lock().unwrap();
                log.expires.push(id);
                (log.drops, log.commit)
            };
            // Without the discard both items live long enough to reach pickup.
            event.extra_life = 20;
            if id != drops[0] {
                return;
            }
            let mut discarded = [false; 2];
            let mut discard = |host: &mut dyn ServerHost| {
                discarded =
                    drops.map(|id| host.entity(id).is_some_and(|entity| entity.discard(host)));
            };
            if let Some(commit) = commit {
                let _ = host.transaction(|host, _| {
                    discard(host);
                    if commit { Ok(()) } else { Err(()) }
                });
            } else {
                discard(host);
            }
            self.0.lock().unwrap().discarded = Some(discarded);
        }

        fn entity_leave_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            self.0.lock().unwrap().leaves.push(event.entity);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn expiry_listener_discards_are_swept_before_more_ticks_or_pickup() {
        let log = Arc::new(Mutex::new(DiscardLog::default()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "discard-on-expiry",
            Box::new(DiscardOnExpiry(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(AT[0], AT[1], AT[2])).unwrap());
                game.update_chunk_tickets();
                assert!(game.entity_ticking_area()(2, 2));

                for commit in [None, Some(true), Some(false)] {
                    game.inventory.fill(None);
                    let drops = [
                        drop_at_age(game, "minecraft:apple", 5999),
                        drop_at_age(game, "minecraft:diamond", 5999),
                    ];
                    *log.lock().unwrap() = DiscardLog {
                        drops,
                        commit,
                        ..DiscardLog::default()
                    };

                    game.tick_items();
                    assert!(game.dropped.is_empty(), "{commit:?}");
                    let picked_up: Vec<&str> = game
                        .inventory
                        .iter()
                        .flatten()
                        .map(|s| s.item.as_ref())
                        .collect();
                    {
                        let log = log.lock().unwrap();
                        assert_eq!(log.discarded, Some([true, true]), "{commit:?}");
                        if commit == Some(false) {
                            assert_eq!(log.expires, drops);
                            assert_eq!(picked_up, ["minecraft:apple", "minecraft:diamond"]);
                        } else {
                            assert_eq!(log.expires, [drops[0]], "{commit:?}");
                            assert_eq!(log.leaves, drops, "{commit:?}");
                            assert!(picked_up.is_empty(), "{commit:?}");
                        }
                    }
                    game.native_leaves_flush();
                    assert_eq!(log.lock().unwrap().leaves, drops, "{commit:?}");
                }
            });
        });
    }

    #[derive(Default)]
    struct AloneDiscardLog {
        drops: [u32; 2],
        commit: Option<bool>,
        events: Vec<(&'static str, u32)>,
    }

    struct AloneDiscardListener(Arc<Mutex<AloneDiscardLog>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AloneDiscardListener {
        fn entity_tick_pre(
            &self,
            _: HostSide<'_>,
            event: &mut EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            self.0.lock().unwrap().events.push(("pre", event.entity));
            mod_api::Flow::Continue
        }

        fn entity_tick_post(&self, _: HostSide<'_>, event: &mut EntityTickPostEvent<'_>) {
            self.0.lock().unwrap().events.push(("post", event.entity));
        }

        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let id = event.item.entity().id();
            let (drops, commit) = {
                let mut log = self.0.lock().unwrap();
                log.events.push(("expire", id));
                (log.drops, log.commit)
            };
            event.extra_life = 20;
            if id != drops[0] {
                return;
            }
            let discard = |host: &mut dyn ServerHost| {
                for id in drops {
                    assert!(host.entity(id).is_some_and(|entity| entity.discard(host)));
                }
            };
            if let Some(commit) = commit {
                let _ = host.transaction(|host, _| {
                    discard(host);
                    if commit { Ok(()) } else { Err(()) }
                });
            } else {
                discard(host);
            }
        }

        fn entity_leave_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            self.0.lock().unwrap().events.push(("leave", event.entity));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn expiry_listener_discards_while_told_alone_still_deliver_post_once() {
        let log = Arc::new(Mutex::new(AloneDiscardLog::default()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "alone-discard-on-expiry",
            Box::new(AloneDiscardListener(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                for commit in [None, Some(true)] {
                    let drops = [
                        drop_at_age(game, "minecraft:apple", 5999),
                        drop_at_age(game, "minecraft:diamond", 5999),
                    ];
                    *log.lock().unwrap() = AloneDiscardLog {
                        drops,
                        commit,
                        events: Vec::new(),
                    };

                    game.tick_items();
                    assert!(game.dropped.is_empty(), "{commit:?}");
                    assert_eq!(
                        log.lock().unwrap().events,
                        [
                            ("pre", drops[0]),
                            ("expire", drops[0]),
                            ("leave", drops[0]),
                            ("leave", drops[1]),
                            ("post", drops[0]),
                        ],
                        "{commit:?}"
                    );
                    game.native_leaves_flush();
                    assert_eq!(log.lock().unwrap().events.len(), 5, "{commit:?}");
                }
            });
        });
    }

    struct LifespanEditListener(Arc<Mutex<Option<bool>>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for LifespanEditListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let edited = event
                .item
                .entity()
                .merge_nbt(host, Compound::from([("Lifespan".into(), Tag::Int(12000))]));
            *self.0.lock().unwrap() = Some(edited);
            event.extra_life = 1;
        }
    }

    /// NeoForge `ItemEntity.tick` evaluates `this.lifespan` before calling
    /// `EventHooks.onItemExpire(this)`: editing `Lifespan` during dispatch does not
    /// shift the base of `lifespan + extra_life`.
    #[test]
    #[ignore = "daily"]
    fn item_expire_adds_extra_life_to_pre_dispatch_lifespan() {
        let edited = Arc::new(Mutex::new(None));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "lifespan-edit-on-expiry",
            Box::new(LifespanEditListener(edited.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let id = drop_at_age(game, "minecraft:stone", 5999);

                game.tick_items();
                assert_eq!(*edited.lock().unwrap(), Some(true));
                let item = game
                    .dropped
                    .iter()
                    .find(|item| item.id == id)
                    .expect("item survives at age 6000 < 6001");
                assert_eq!(item.age, 6000);
                assert_eq!(item.lifespan, 6001);
            });
        });
    }

    struct BurstListener {
        expired: Arc<Mutex<Vec<(u32, Option<u32>, Option<u32>)>>>,
        left: Arc<Mutex<Vec<(u32, bool)>>>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for BurstListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let mut count = None;
            let mut damage = None;
            event.item.with_stack(host, |stack| {
                count = Some(stack.count());
                damage = Some(stack.damage());
            });
            self.expired
                .lock()
                .unwrap()
                .push((event.item.entity().id(), count, damage));
        }

        fn entity_leave_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            let still_resident = host.entity(event.entity).is_some();
            self.left
                .lock()
                .unwrap()
                .push((event.entity, still_resident));
        }
    }

    /// Expiring K of N component-bearing drops in one tick lends each stack through
    /// `ItemEntityRef::with_stack`, preserves per-drop `entity_leave_level` departure
    /// order while hiding already-departed rows, and avoids O(KN) list compaction and
    /// `ItemCells` rebuilds.
    #[test]
    #[ignore = "daily"]
    fn expiry_burst_lends_stacks_preserves_order_and_reuses_cells() {
        let expired = Arc::new(Mutex::new(Vec::with_capacity(512)));
        let left = Arc::new(Mutex::new(Vec::with_capacity(512)));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "expiry-burst-listener",
            Box::new(BurstListener {
                expired: expired.clone(),
                left: left.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let mut expected_expired = Vec::with_capacity(500);
                let mut expected_survivors = Vec::with_capacity(500);
                for idx in 0..1000usize {
                    let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
                    stack.damage = 42;
                    let data = stack.data_mut();
                    data.custom_name = Some(Tag::String("Expiring Blade".into()));
                    data.enchantments.push(("minecraft:sharpness".into(), 5));
                    let x = 2.5 + (idx % 10) as f64 * 1.1;
                    let z = 2.5 + ((idx / 10) % 10) as f64 * 1.1;
                    game.push_item_entity(stack, [x, 64.0, z], Some([0.0; 3]), 0, None);
                    let item = game.dropped.last_mut().unwrap();
                    let id = item.id;
                    if idx % 2 == 0 {
                        item.age = 5999;
                        expected_expired.push(id);
                    } else {
                        item.age = 0;
                        expected_survivors.push(id);
                    }
                    game.ticked_entities_through = id;
                }
                game.native_joins_flush(false, 0);

                let start = std::time::Instant::now();
                let ((), allocs) = sunrise_threads::allocation_count::measure(|| {
                    game.tick_items();
                });
                let elapsed = start.elapsed();
                eprintln!(
                    "ROW5_AFTER total_ms={:.3} us_per_expiry={:.1} allocs={allocs}",
                    elapsed.as_secs_f64() * 1000.0,
                    elapsed.as_secs_f64() * 1e6 / 500.0,
                );

                let exp = expired.lock().unwrap();
                assert_eq!(exp.len(), 500);
                for (&(id, count, damage), &want) in exp.iter().zip(&expected_expired) {
                    assert_eq!(id, want);
                    assert_eq!(count, Some(1));
                    assert_eq!(damage, Some(42));
                }
                let lft = left.lock().unwrap();
                assert_eq!(lft.len(), 500);
                for (&(id, still_resident), &want) in lft.iter().zip(&expected_expired) {
                    assert_eq!(id, want);
                    assert!(
                        !still_resident,
                        "departed item {id} must not resolve during entity_leave_level"
                    );
                }
                let survivors: Vec<u32> = game.dropped.iter().map(|item| item.id).collect();
                assert_eq!(survivors, expected_survivors);
                // Baseline before Row 5 was 404,829 allocations (O(KN) ItemCells rebuilds + stack clones).
                assert!(
                    allocs < 10_000,
                    "expiry burst allocated {allocs} times, expected O(N) without per-expiry rebuilds",
                );
            });
        });
    }

    struct CountingExpireListener {
        counts: Arc<Mutex<Vec<(u32, usize)>>>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CountingExpireListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let level = host.level("minecraft:overworld").unwrap();
            let mut seen_drops = 0usize;
            let _ = level.visit_entities_in(
                host,
                [0.0, 60.0, 0.0],
                [20.0, 70.0, 20.0],
                &mut |entity| {
                    if entity.kind(host).and_then(|kind| kind.id(host)).as_deref()
                        == Some("minecraft:item")
                    {
                        seen_drops += 1;
                    }
                    std::ops::ControlFlow::Continue(())
                },
            );
            self.counts
                .lock()
                .unwrap()
                .push((event.item.entity().id(), seen_drops));
        }
    }

    /// An `item_expire` listener that queries drops in the level through the host
    /// does not see an earlier drop that already expired in the same pass.
    #[test]
    #[ignore = "daily"]
    fn item_expire_host_query_excludes_earlier_expired_drops() {
        let counts = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "expire-query-listener",
            Box::new(CountingExpireListener {
                counts: counts.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let mut ids = Vec::new();
                for (idx, age) in [5999, 5999, 0].into_iter().enumerate() {
                    let stack = ItemStack::new("minecraft:diamond_sword", 1);
                    let x = 2.5 + idx as f64 * 2.0;
                    game.push_item_entity(stack, [x, 64.0, 2.5], Some([0.0; 3]), 0, None);
                    let item = game.dropped.last_mut().unwrap();
                    item.age = age;
                    ids.push(item.id);
                    game.ticked_entities_through = item.id;
                }
                game.native_joins_flush(false, 0);

                game.tick_items();

                // First expiring drop sees all 3 live drops; second expiring drop sees only 2
                // (itself + the non-expiring survivor), never the first already-expired drop.
                assert_eq!(*counts.lock().unwrap(), vec![(ids[0], 3), (ids[1], 2)]);
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(game.dropped[0].id, ids[2]);
            });
        });
    }
}

#[cfg(test)]
#[path = "tests/thrown_items_tests.rs"]
pub(in crate::game) mod thrown_items_tests;

#[cfg(test)]
mod bob_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_core::random::RandomSource;

    /// The bob a client gives the item with entity id `id` at `at`.
    fn bob(id: u32, at: Vec3, still: bool) -> f32 {
        let mut random = sunrise_client::mobs::entity_random(id, at);
        super::bob_offset(at, random.next_float(), still)
    }

    /// Under `SUNRISE_STILL` the item's position decides its bob, not the id the server gave
    /// it; without it the bob is the id-seeded random's own draw.
    #[test]
    fn a_still_items_bob_comes_from_its_position() {
        let at = Vec3::new(0.5, 64.0, 2.5);
        assert_eq!(bob(17, at, true), bob(22, at, true));
        for id in [17, 22] {
            let mut random = sunrise_client::mobs::entity_random(id, at);
            let vanilla = random.next_float() * std::f32::consts::PI * 2.0;
            assert_eq!(bob(id, at, false), vanilla);
        }
        assert_ne!(bob(17, at, false), bob(22, at, false));
    }
}
