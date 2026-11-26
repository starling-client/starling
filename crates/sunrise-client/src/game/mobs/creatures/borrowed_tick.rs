//! Resumable borrowed mob-tick region; the caller owns final restoration.
use super::*;
use crate::mobs::living::section_writes::{self, PositionWrite};
use std::collections::hash_map::Entry;

#[cfg(test)]
thread_local! {
    static BORROWED_MOB_YIELDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// Route each player event to the player selected when it was emitted.
/// Untagged events retain the previous per-tick guest binding.
fn route_player_events(
    events: &mut Vec<MobEvent>,
    from: usize,
    seen_guest: Option<u32>,
    guest_ids: &[(u32, u32)],
    guest_events: &mut Vec<(u32, u32, MobEvent)>,
    mob: u32,
) {
    let mut k = from;
    while k < events.len() {
        let (conn, tagged) = match &events[k] {
            MobEvent::ForPlayer { conn, .. } => (*conn, true),
            _ => (seen_guest, false),
        };
        if tagged {
            let MobEvent::ForPlayer { event, .. } = events.remove(k) else {
                unreachable!();
            };
            events.insert(k, *event);
        }
        // A hoglin broadcast carries the source's player, not the receiving
        // hoglin's independently selected player.
        if let MobEvent::HoglinAttackTarget {
            target: Who::Player,
            player,
            ..
        } = &mut events[k]
        {
            *player = conn;
        }
        // ShulkerBullet stores a target for later projectile ticks. Preserve
        // the selected guest's entity ID instead of the host-player sentinel.
        if let Some(conn) = conn
            && let MobEvent::Projectile(p) = &mut events[k]
            && let ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
            && *target == Some(Who::Player)
        {
            *target = guest_ids
                .iter()
                .find(|g| g.0 == conn)
                .map(|g| Who::Mob(g.1));
        }
        if let Some(conn) = conn
            && super::super::lan_mobs::acts_on_player(&events[k])
        {
            guest_events.push((conn, mob, events.remove(k)));
        } else {
            k += 1;
        }
    }
}

/// Commit departures in the borrowed list, not when a tick merely proposes retention.
/// In particular an interrupted pass keeps the credit of every row it gives back.
fn retain_mob_rows(
    mobs: &mut sunrise_client::join_seq::JoinList<Mob>,
    player_credit: &mut std::collections::HashMap<u32, super::super::combat::PlayerCredit>,
    mut keep: impl FnMut(&Mob) -> bool,
) {
    mobs.retain(|mob| {
        let kept = keep(mob);
        if !kept {
            player_credit.remove(&mob.id);
        }
        kept
    });
}

pub(super) struct BorrowedMobTick<'a> {
    pub(super) mobs: &'a mut sunrise_client::join_seq::JoinList<Mob>,
    pub(super) snapshot: &'a mut Vec<Seen>,
    pub(super) passenger_objects: &'a [Seen],
    pub(super) blocked: &'a std::collections::HashSet<u32>,
    pub(super) events: &'a mut Vec<MobEvent>,
    pub(super) restocks: &'a mut Vec<sunrise_client::mobs::villager_brain::RestockNotice>,
    pub(super) pushes: &'a mut sunrise_client::mobs::PendingPushes,
    pub(super) dead: &'a mut Vec<Mob>,
    pub(super) died: &'a mut sunrise_client::fast_hash::FxHashSet<u32>,
    pub(super) ticked: &'a mut sunrise_client::fast_hash::FxHashSet<u32>,
    pub(super) guests: &'a mut super::super::lan_mobs::GuestViews,
    pub(super) spare: &'a mut Option<PlayerView>,
    pub(super) bindings: &'a mut sunrise_client::fast_hash::FxHashMap<u32, u32>,
    pub(super) guest_ids: &'a mut Vec<(u32, u32)>,
    pub(super) guest_events: &'a mut Vec<(u32, u32, MobEvent)>,
    pub(super) player_credit:
        &'a mut std::collections::HashMap<u32, super::super::combat::PlayerCredit>,
    pub(super) path_writes: &'a mut Vec<((i32, i32, i32), u32, u32)>,
    pub(super) gone: &'a mut Vec<(u32, Vec3, f64, f64)>,
    pub(super) lent: sunrise_client::health::LentLevel,
    pub(super) laps: &'a mut sunrise_client::spans::Laps,
    pub(super) uuid: Option<[i32; 4]>,
    pub(super) level_id: &'static str,
    pub(super) round: &'a mut super::super::native_ticks::TickRound,
    pub(super) stand_result: &'a mut Result<()>,
}
impl Game<'_> {
    pub(super) fn tick_borrowed_creatures(
        &mut self,
        state: BorrowedMobTick<'_>,
    ) -> (
        sunrise_client::health::LentLevel,
        Option<Box<dyn std::any::Any + Send>>,
    ) {
        let passenger_objects = state.passenger_objects;
        let mut blocked = std::borrow::Cow::Borrowed(state.blocked);
        let mut mobs = state.mobs.take_owned_pass();
        let mut snapshot = std::mem::take(state.snapshot);
        let mut events = std::mem::take(state.events);
        let mut dead = std::mem::take(state.dead);
        let mut died = std::mem::take(state.died);
        let mut ticked = std::mem::take(state.ticked);
        let mut guests = std::mem::take(state.guests);
        let guest_indices = |guests: &super::super::lan_mobs::GuestViews| {
            guests
                .iter()
                .enumerate()
                .map(|(slot, (conn, _))| (*conn, slot))
                .collect()
        };
        let mut guest_slots: sunrise_client::fast_hash::FxHashMap<u32, usize> =
            guest_indices(&guests);
        let mut spare = state.spare.take();
        let mut bindings = std::mem::take(state.bindings);
        let mut guest_ids = std::mem::take(state.guest_ids);
        let mut guest_events = std::mem::take(state.guest_events);
        let mut player_credit = std::mem::take(state.player_credit);
        let mut path_writes = std::mem::take(state.path_writes);
        let mut gone = std::mem::take(state.gone);
        let mut lent = state.lent;
        let laps = state.laps;
        let uuid = state.uuid;
        let level_id = state.level_id;
        let round = state.round;
        // Each mob's position updates in its tick, in order, filed and told before the next mob
        // is examined. The small record remains separate from `Mob` because teleports update
        // the same position field as ordinary movement.
        let section_scope = round
            .sections()
            .then(|| self.entity_section_scope())
            .flatten();
        let (section_table, live_section_observer) = if round.sections() {
            self.native_lifecycle
                .section_observer
                .begin(&mut self.native_lifecycle.entity_sections, section_scope)
        } else {
            (None, None)
        };
        // A server's mob crossings update residency whether or not a hook listens.
        // Flush the borrowed writes before an owning callback can record more, or at pass end.
        let entries = self.is_logical_server();
        let mut section_entered =
            std::mem::take(&mut self.native_lifecycle.entity_sections.entered);
        let mut section_moves = match section_table.as_ref() {
            Some(sections) => std::mem::take(&mut sections.borrow_mut().ticked),
            None => std::mem::take(&mut self.native_lifecycle.entity_sections.ticked),
        };
        let capture_sections = section_table.is_some() || entries;
        section_writes::begin(capture_sections);
        let mut dispatch_panic = None;
        let mut pushes = std::mem::take(state.pushes);
        let original_len = snapshot.len();
        // A restored list can put new joins first. Keep its storage and view
        // slots intact, but walk its turns in join order when interleaving stands.
        let turn_order = (!self.ordered.armor_stands.is_empty()
            && !mobs.is_sorted_by_key(|mob| mob.join))
        .then(|| {
            let mut order: Vec<_> = (0..original_len).collect();
            order.sort_unstable_by_key(|&slot| mobs[slot].join);
            order
        });
        let mut keep = vec![true; original_len];
        let mut grid =
            sunrise_client::mobs::seen_grid::SeenGrid::new(snapshot.iter().map(|s| s.bounds));
        // Snapshot slots stay stable when the owning list changes order. Use the
        // existing ID-to-snapshot index to refresh the slots' row positions only
        // when the pass or a callback changes the list, not on each section write.
        let mut positions: Vec<Option<usize>> = Vec::new();
        let mut index = sunrise_client::fast_hash::FxHashMap::<u32, u32>::default();
        let mut villager_slots = Vec::new();
        let mut solid_slots = Vec::new();
        for (i, s) in snapshot.iter().enumerate() {
            index.entry(s.id).or_insert(i as u32);
            if s.may_block_mobs() {
                solid_slots.push(i as u32);
            }
            if s.species == sunrise_client::mobs::Species::Villager {
                villager_slots.push(i as u32);
            }
        }
        let mut cursor = 0;
        // Sounds the tick's hits queued, appended to the level's queue once the level is dropped.
        let mut hit_sounds: Vec<super::NativeSound> = Vec::new();
        use sunrise_client::mobs::{
            ai::Pause,
            player_selection::{PlayerSelection, TickPlayer},
        };
        let mut resume = None;
        // Keep the suspended move out of the ordinary tick's result.
        let mut movement = None;
        let mut resume_entity = None;
        let mut resume_air_answer = None;
        let mut resume_selection: Option<PlayerSelection> = None;
        // An invalid removed vehicle is retried once at this rider's own
        // eligible entity tick, never from the later global positioning pass.
        let mut retried_rider = None;
        let mut resume_from = 0;
        let mut resume_sampled = None;
        let mut resume_guest = None;
        // The turn of the mob whose own Pre was just told (a type a mod named): it resumes
        // there with its checks done.
        let mut told_at: Option<usize> = None;
        let mut enchantment_tick_at = None;
        // Whether a mob still in the lifted list has left the pass (`keep` false) since the list
        // was last given back: a told-alone yield then takes it out before the call.
        let mut gone_rows = false;
        let mut resume_brain = None;
        // These belong to the finished mob, not the victim its event may hurt.
        // Keep them until the mob's event tail finishes, then file its section
        // writes and issue its Post while the rows are back in the owner.
        let mut tell_post: Option<(u32, std::sync::Arc<sunrise_client::mobs::MobType>)> = None;
        loop {
            let mut section_hint = None;
            let mut pause = None;
            let mut sting = None;
            let mut brain_from = 0;
            let mut air_answer = None;
            let mut retry_rider = None;
            let mut removing_vehicle = None;
            let mut pending_effects: Vec<MobEvent> = Vec::new();
            let mut removal = None;
            let mut stand_cut = None;
            let mut lid_push = None;
            let mut paused_hurt = None;
            // A mob told alone hears its Pre right before its tick and its Post right after:
            // stop there, restore the list, tell it while the live rows are readable, and resume.
            let mut tell_pre: Option<(u32, std::sync::Arc<sunrise_client::mobs::MobType>, usize)> =
                None;
            let mut tick_enchantments = None;
            let mut views_dirty = false;
            {
                let ticking = self.entity_ticking_area();
                let listed = self.entity_tick_listed_area();
                let players = self.level_players();
                let nearest = super::super::natural_spawning::NearestPlayers::new(&players);
                // All but the snapshot is the same for every mob of the tick, so
                // build it once: the player view and the teams scan every UUID.
                let mut leash_boats = std::mem::take(&mut self.leash_boats);
                leash_boats.clear();
                leash_boats.extend(self.boats.iter().map(super::super::boat_leads::boat_state));
                self.leash_boats = leash_boats;
                let mut base = self.mob_world_base(&[]);
                // Install the shared observer only on the per-mob view below; keeping the
                // reusable base empty lets its reference fields remain copyable between rows.
                base.section_writer = None;
                base.passenger_objects = passenger_objects;
                // The owning game completes live births at the goal/behaviour site even
                // without a listener. Hostless probes still emit an ordinary breed event.
                base.asks_baby_spawn = true;
                let mut level = sunrise_client::health::DamageLevel::lent(
                    &self.world,
                    self.pack.assets.registry,
                    &mut lent,
                    Some(world_items!(self)),
                    Some(&self.pack.block_registries),
                    &self.damage_hooks,
                )
                .with_variants(world_variants!(self))
                .with_mob_hurt_handoff(&mut paused_hurt)
                .with_mob_uuids(&self.persist.uuids)
                .with_player_resolver(
                    crate::game::entity::player_resolver!(self),
                    &mut player_credit,
                )
                .with_effects(crate::game::entity::hit_effects_of!(
                    self,
                    None,
                    in hit_sounds
                ))
                .with_reads(crate::game::entity::hit_reads_of!(self));
                let path_changes = sunrise_client::mobs::collision_changes(&base, &path_writes);
                laps.lap("creature_prepare");
                let mut cached_difficulty: Option<((i32, i32), spawning::LocalDifficulty)> = None;
                if let Some((from, hint, post)) = resume_brain.take() {
                    brain_from = from;
                    section_hint = hint;
                    tell_post = post;
                    let before_brain = mobs.stamp();
                    sting = sunrise_client::mobs::apply_brain_events_until_hurt(
                        &mut mobs,
                        &mut level,
                        &mut events,
                        from,
                        &base.teams,
                        true,
                    );
                    views_dirty |= mobs.stamp() != before_brain;
                }
                while sting.is_none()
                    && section_hint.is_none()
                    && tell_post.is_none()
                    && cursor < original_len
                {
                    let slot = turn_order.as_ref().map_or(cursor, |order| order[cursor]);
                    let i = if positions.is_empty() {
                        slot
                    } else if let Some(i) = positions[slot] {
                        i
                    } else {
                        keep[slot] = false;
                        snapshot[slot].alive = false;
                        // The row's mob left the list at a boundary: the
                        // tick view must answer a removed handle with nothing.
                        snapshot[slot].removed = true;
                        let equipment_finished = matches!(
                            resume.take(),
                            Some(Pause::Equipment { .. } | Pause::RegisterBehaviors { .. })
                        );
                        movement = None;
                        resume_selection = None;
                        resume_sampled = None;
                        cursor += 1;
                        // A discard in the equipment event ended a tick that had already started.
                        let mob = &snapshot[slot];
                        if equipment_finished && round.post() {
                            tell_post = Some((mob.id, std::sync::Arc::clone(&mob.kind)));
                            break;
                        }
                        continue;
                    };
                    let m = &mut mobs[i];
                    if self
                        .ordered
                        .armor_stands
                        .front()
                        .is_some_and(|&join| join < m.join)
                    {
                        stand_cut = Some(m.join);
                        break;
                    }
                    let retried = retried_rider.take() == Some(m.id);
                    // Its checks ran before its Pre or a pause in its tick: don't run them again.
                    let resuming = resume.is_some();
                    let checked = resuming || told_at == Some(cursor);
                    if blocked.contains(&m.id) || resuming && resume_entity != Some((m.id, m.join))
                    {
                        cursor += 1;
                        resume = None;
                        movement = None;
                        resume_selection = None;
                        resume_sampled = None;
                        continue;
                    }
                    if m.removed && !resuming {
                        keep[slot] = false;
                        gone_rows = true;
                        snapshot[slot].alive = false;
                        // `removed` is the view's authoritative gone: a dead body
                        // that is still in the level keeps resolving (#795).
                        snapshot[slot].removed = true;
                        cursor += 1;
                        continue;
                    }
                    let (bx, bz) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
                    let seen_guest = if resuming {
                        resume_guest
                    } else if guests.is_empty() {
                        None
                    } else {
                        let bound = bindings.get(&m.id).copied();
                        let conn = super::super::lan_mobs::player_for(
                            m,
                            base.player.as_ref(),
                            &guests,
                            bound,
                        );
                        match conn {
                            Some(conn) => {
                                bindings.insert(m.id, conn);
                            }
                            None => {
                                bindings.remove(&m.id);
                            }
                        }
                        conn
                    };
                    if !checked && !base.world.loaded(bx, bz) {
                        keep[slot] = !m.hostile() || m.persistent;
                        gone_rows |= !keep[slot];
                        cursor += 1;
                        let id = m.id;
                        if !keep[slot]
                            && (m.extra.mount.is_some()
                                || self.has_seated_passengers_with_lent_mobs(id, &mobs))
                        {
                            removing_vehicle = Some(id);
                            break;
                        }
                        continue;
                    }
                    // ServerLevel.tick walks the entity tick list, which holds only the
                    // entities of chunks whose own status is entity-ticking: a mob in the
                    // loaded ring outside it fires no `mob_despawn`, makes no despawn draws
                    // and doesn't tick.
                    if !checked && !listed(bx, bz) {
                        cursor += 1;
                        continue;
                    }
                    let nearest_player = nearest
                        .nearest(m.position.x, m.position.y, m.position.z)
                        .map(|p| p.position);
                    // `MobDespawnEvent`, before `checkDespawn`'s own draws; one branch
                    // with no listener. Cleared below: the tick may not reach the check.
                    // The wither overrides `checkDespawn` without `super`, so the
                    // event, patched into `Mob.checkDespawn`, never fires for it;
                    // a mannequin is no `Mob` and never checks despawning.
                    if !checked
                        && !retried
                        && mod_api::__private::listened(mod_api::HookId::MobDespawn)
                        && !matches!(
                            m.species(),
                            sunrise_client::mobs::Species::Wither
                                | sunrise_client::mobs::Species::Mannequin
                        )
                        && let Some(epoch) = self.native_lifecycle.epoch()
                    {
                        let (id, kind, at) = (
                            m.id,
                            std::sync::Arc::clone(&m.kind),
                            [m.position.x, m.position.y, m.position.z],
                        );
                        let mut result = mod_api::Despawn::default();
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                // The handle resolves through the tick's own view:
                                // the game has lent its mob list to this pass (#795).
                                let world = MobWorld {
                                    others: snapshot.as_slice(),
                                    others_grid: Some(&grid),
                                    others_index: Some(&index),
                                    others_villagers: Some(&villager_slots),
                                    others_solid: Some(&solid_slots),
                                    live_mobs: None,
                                    ..base.clone()
                                };
                                self.native_mob_despawn_in_tick(
                                    m,
                                    &world,
                                    self.dim.id(),
                                    mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id)),
                                    &kind.id,
                                    at,
                                    &mut result,
                                );
                            }));
                        m.despawn = result;
                        if let Err(payload) = outcome {
                            m.despawn = mod_api::Despawn::default();
                            dispatch_panic = Some(payload);
                            break;
                        }
                    }
                    // ServerLevel.tick: every listed entity checks despawning, but
                    // only those within the simulation distance as well tick. The
                    // despawn check reads the nearest player and the difficulty, not
                    // the player the mob sees.
                    if !checked && !ticking(bx, bz) {
                        let w = MobWorld {
                            others: &snapshot,
                            others_grid: Some(&grid),
                            others_index: Some(&index),
                            others_villagers: Some(&villager_slots),
                            others_solid: Some(&solid_slots),
                            nearest_player,
                            player: None,
                            teams: std::sync::Arc::clone(&base.teams),
                            section_writer: None,
                            ..base
                        };
                        let kept = sunrise_client::mobs::ai::despawn_check(m, &w);
                        m.despawn = mod_api::Despawn::default();
                        if !kept {
                            died.remove(&m.id);
                            snapshot[slot].alive = false;
                            snapshot[slot].removed = true;
                        }
                        keep[slot] = kept;
                        gone_rows |= !kept;
                        cursor += 1;
                        let id = m.id;
                        if !kept
                            && (m.extra.mount.is_some()
                                || self.has_seated_passengers_with_lent_mobs(id, &mobs))
                        {
                            removing_vehicle = Some(id);
                            break;
                        }
                        continue;
                    }
                    let controlling_player_movement = sunrise_client::mobs::equine::controlled(m)
                        .then(|| {
                            base.player
                                .as_ref()
                                .filter(|p| p.riding_vehicle == Some(m.id))
                                .map(|p| p.velocity)
                                .or_else(|| {
                                    guests
                                        .iter()
                                        .find(|(_, p)| p.riding_vehicle == Some(m.id))
                                        .map(|(_, p)| p.velocity)
                                })
                        })
                        .flatten();
                    // Its own Pre, right before its tick.
                    if !checked && round.pre() {
                        tell_pre = Some((m.id, std::sync::Arc::clone(&m.kind), i));
                        break;
                    }
                    // A mod canceled its tick: it stays as it is.
                    if !resuming && round.canceled(m.id) {
                        m.despawn = mod_api::Despawn::default();
                        cursor += 1;
                        continue;
                    }
                    if !resuming
                        && !retried
                        && m.extra.mount.is_some_and(|vehicle| {
                            self.persist.removed_vehicles.contains_key(&vehicle)
                        })
                    {
                        retry_rider = Some(m.id);
                        break;
                    }
                    if !resuming
                        && enchantment_tick_at != Some(slot)
                        && entries
                        && mod_api::EquipmentSlot::ALL.into_iter().any(|slot| {
                            sunrise_client::mobs::finalize::equipment_slot(m, slot).is_some_and(
                                |stack| {
                                    world_stations!(self)
                                        .enchantments
                                        .has_component(&stack.project(), "minecraft:tick")
                                },
                            )
                        })
                    {
                        tick_enchantments = Some((m.id, m.join));
                        break;
                    }
                    // getCurrentDifficultyAt(blockPosition()): the mob's own chunk,
                    // not the local player's (a server thread has no player).
                    let chunk_key = (bx >> 4, bz >> 4);
                    let difficulty = match cached_difficulty {
                        Some((key, difficulty)) if key == chunk_key => difficulty,
                        _ => {
                            let difficulty =
                                self.local_difficulty([m.position.x, m.position.y, m.position.z]);
                            cached_difficulty = Some((chunk_key, difficulty));
                            difficulty
                        }
                    };
                    sunrise_client::mobs::apply_mob_pushes(m, &mut pushes);
                    sunrise_client::mobs::blocks_changed(m, &path_changes);
                    if !resuming {
                        // LivingEntity.aiStep -> EntityEquipment.tick, before movement.
                        super::super::inventory_tick::equipment_inventory_tick(
                            &self.world,
                            self.res.facts(),
                            level_id,
                            &mut m.extra.equipment,
                        );
                    }
                    let was_removed = m.removed;
                    let from = if resuming { resume_from } else { events.len() };
                    // The guest's view is lent to the mob for its tick, not copied:
                    // the tick only reads it (`&MobWorld`), and it goes back to the
                    // guest's slot straight after. Its identity is captured with
                    // it, once: the player a goal resolves is the one the tick
                    // sees — a guest on a server, the game's own otherwise (#1077).
                    let lent = seen_guest.and_then(|conn| guest_slots.get(&conn).copied());
                    let active_player = match (seen_guest, lent) {
                        (None, _) => base.player.take(),
                        (Some(_), Some(k)) => spare
                            .take()
                            .map(|stand_in| std::mem::replace(&mut guests[k].1, stand_in)),
                        (Some(_), None) => None,
                    };
                    let active_identity = match seen_guest {
                        None => base.player_identity,
                        Some(conn) => self.guest_identity(conn),
                    };
                    // A listener can redirect to any real player in this level. Keep
                    // immutable views during the borrow so changing selection
                    // never invalidates a view a goal already holds. No allocation
                    // is made when nobody listens for target changes and no pet
                    // needs a player other than the one it sees.
                    let selection = if resuming {
                        let mut selection = resume_selection.take();
                        if matches!(
                            resume,
                            Some(
                                Pause::AfterDamage(_)
                                    | Pause::Equipment { .. }
                                    | Pause::RegisterBehaviors { .. }
                            )
                        ) && let Some(selection) = &mut selection
                        {
                            selection.refresh_views(|player| {
                                let (view, identity) = if player.connection == seen_guest {
                                    (active_player.as_ref(), active_identity)
                                } else if let Some(conn) = player.connection {
                                    (
                                        guest_slots.get(&conn).map(|&slot| &guests[slot].1),
                                        self.guest_identity(conn),
                                    )
                                } else {
                                    (base.player.as_ref(), base.player_identity)
                                };
                                view.filter(|_| {
                                    identity.is_some_and(|(_, id)| id == player.entity_id)
                                })
                                .cloned()
                            });
                        }
                        selection
                    } else if mod_api::__private::listened(mod_api::HookId::LivingChangeTarget)
                        || sunrise_client::mobs::tame::reads_another_player(
                            m,
                            active_player.as_ref(),
                            seen_guest.and(base.player.as_ref()).into_iter().chain(
                                guests
                                    .iter()
                                    .enumerate()
                                    .filter(|(k, _)| Some(*k) != lent)
                                    .map(|(_, (_, view))| view),
                            ),
                            |id| {
                                base.player_identity.is_some_and(|(_, own)| own == id)
                                    || guests.iter().any(|(conn, _)| {
                                        self.guest_identity(*conn)
                                            .is_some_and(|(_, guest)| guest == id)
                                    })
                            },
                        )
                    {
                        let mut players = Vec::new();
                        if let (Some(view), Some((_, entity_id))) =
                            (&active_player, active_identity)
                        {
                            players.push(TickPlayer {
                                entity_id,
                                connection: seen_guest,
                                view: view.clone(),
                            });
                        }
                        if seen_guest.is_some()
                            && let (Some(view), Some((_, entity_id))) =
                                (&base.player, base.player_identity)
                        {
                            players.push(TickPlayer {
                                entity_id,
                                connection: None,
                                view: view.clone(),
                            });
                        }
                        for (index, (conn, view)) in guests.iter().enumerate() {
                            if Some(index) == lent {
                                continue;
                            }
                            if let Some((_, entity_id)) = self.guest_identity(*conn) {
                                players.push(TickPlayer {
                                    entity_id,
                                    connection: Some(*conn),
                                    view: view.clone(),
                                });
                            }
                        }
                        PlayerSelection::new(players, active_identity.map(|(_, id)| id))
                    } else {
                        None
                    };

                    let mut w = MobWorld {
                        others: &snapshot,
                        others_grid: Some(&grid),
                        others_index: Some(&index),
                        others_villagers: Some(&villager_slots),
                        others_solid: Some(&solid_slots),
                        difficulty,
                        nearest_player,
                        player: active_player,
                        player_identity: active_identity,
                        player_selection: selection.as_ref(),
                        teams: std::sync::Arc::clone(&base.teams),
                        section_writer: None,
                        ..base
                    };
                    w.section_writer = live_section_observer.as_ref().map(|observer| {
                        let epoch = observer.borrow().epoch();
                        let residency = self.entity_residency(mod_api::EntityRef::new(epoch, m.id));
                        observer.borrow_mut().set_residency(m.id, residency);
                        std::rc::Rc::clone(observer)
                            as crate::mobs::living::section_writes::Observer
                    });
                    w.controlling_player_movement = controlling_player_movement;
                    ticked.insert(m.id);
                    // `sunrise-report`'s cost by mob type (tick_profile.rs).
                    let sampled = if resuming {
                        sunrise_threads::work::set_entity(sunrise_threads::work::entity_id(
                            &mobs[i].kind.id,
                        ));
                        resume_sampled.take()
                    } else {
                        super::super::tick_profile::start_mob(&mobs[i].kind)
                    };
                    let run_tick = || match resume.take() {
                        Some(Pause::Breathing { air, known_speed }) => {
                            sunrise_client::mobs::riders::resume_after_breathing(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                air,
                                known_speed,
                                resume_air_answer.take(),
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Drink { resume, .. }) => {
                            sunrise_client::mobs::riders::resume_after_drink(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                resume,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Equipment { .. }) => {
                            sunrise_client::mobs::riders::resume_after_equipment(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::RegisterBehaviors { stage }) => {
                            sunrise_client::mobs::riders::resume_after_register_behaviors(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                stage,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::AfterDamage(damage)) => {
                            sunrise_client::mobs::riders::resume_after_damage(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                damage,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Breeding { resume, .. }) => {
                            sunrise_client::mobs::riders::resume_after_birth(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                resume,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Trample(_) | Pause::FallToPassengers(_)) => {
                            sunrise_client::mobs::riders::resume_movement(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                &mut movement,
                                &mut events,
                            )
                        }
                        Some(Pause::InsideEffects(paused)) => {
                            sunrise_client::mobs::riders::resume_after_inside_effects(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                paused,
                                &mut events,
                            )
                        }
                        Some(Pause::Consumption { .. }) => {
                            sunrise_client::mobs::riders::resume_after_consumption(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                &mut events,
                                &mut movement,
                            )
                        }
                        None => sunrise_client::mobs::riders::tick_until_consumption(
                            &mut mobs,
                            i,
                            &w,
                            &mut level,
                            &mut events,
                            &mut movement,
                        ),
                    };
                    let progress = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_tick));
                    // Screen updates must run before the next callback. Take them out of this
                    // villager's event tail now, before any continuation remembers an index in it.
                    if mobs[i].species() == Species::Villager {
                        for event in events.extract_if(from.., |event| {
                            matches!(event, MobEvent::VillagerRestockStep(_))
                        }) {
                            let MobEvent::VillagerRestockStep(restock) = event else {
                                unreachable!()
                            };
                            state.restocks.push(restock);
                        }
                    }
                    let progress = match progress {
                        Ok(progress) => progress,
                        Err(payload) => {
                            mobs[i].despawn = mod_api::Despawn::default();
                            if seen_guest.is_none() {
                                base.player = w.player;
                            } else if let Some(k) = lent {
                                spare = w.player.map(|v| std::mem::replace(&mut guests[k].1, v));
                            }
                            sunrise_client::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                            route_player_events(
                                &mut events,
                                from,
                                seen_guest,
                                &guest_ids,
                                &mut guest_events,
                                mobs[i].id,
                            );
                            dispatch_panic = Some(payload);
                            break;
                        }
                    };
                    let kept = match progress {
                        sunrise_client::mobs::ai::TickProgress::Complete(kept) => {
                            movement = None;
                            kept
                        }
                        sunrise_client::mobs::ai::TickProgress::Paused(paused) => {
                            if matches!(paused, Pause::Breeding { .. }) {
                                // In particular AnimalMakeLove's earlier lock-gaze is already
                                // visible to the partner when spawnChildFromBreeding runs.
                                sunrise_client::mobs::apply_brain_events(
                                    &mut mobs,
                                    &mut level,
                                    &mut events,
                                    from,
                                    &base.teams,
                                );
                            }
                            // Return every player loan before ending the world borrow.
                            if seen_guest.is_none() {
                                base.player = w.player;
                            } else if let Some(k) = lent
                                && let Some(player) = w.player
                            {
                                guests[k].1 = player;
                            }
                            resume_from = from;
                            resume_sampled = sampled;
                            resume_guest = seen_guest;
                            resume_selection = selection;
                            resume_entity = Some((mobs[i].id, mobs[i].join));
                            pause = Some(paused);
                            sunrise_threads::work::clear_work();
                            break;
                        }
                    };
                    // Shulker.onPeekAmountChange shoves with the shulker's new
                    // bounds, before the next stand or mob starts its own tick.
                    if mobs[i].species() == Species::Shulker
                        && let Some(offset) = events[from..]
                            .iter()
                            .position(|event| matches!(event, MobEvent::LidPush { .. }))
                        && let MobEvent::LidPush {
                            shulker,
                            area,
                            delta,
                        } = events.remove(from + offset)
                    {
                        lid_push = Some((shulker, area, delta));
                    }
                    super::super::tick_profile::end_mob(level_id, sampled);
                    let m = &mut mobs[i];
                    // Its `xo` (`previous`) is where its tick began, unless a teleport in the
                    // tick (`snapTo`) moved it along: that teleport, then where the tick left
                    // it, are two position updates (`EntityInLevelCallback.onMove` for each).
                    if round.sections() || entries {
                        let start = section_moves.len();
                        let captured = section_writes::drain_for(m.id, &mut section_moves);
                        if kept && !m.removed {
                            if captured == 0 && m.previous != m.position {
                                // A position changed before the lifted tick (or a raw test fixture)
                                // still has the old filed section as its first write.
                                if round.sections() {
                                    section_moves
                                        .push(PositionWrite::baseline(m.id, m.join, m.previous));
                                }
                                section_moves.push(PositionWrite::captured(
                                    m.id, m.join, m.previous, m.position,
                                ));
                            }
                            if section_scope.is_some()
                                && section_moves[start..].iter().any(|write| !write.delivered)
                            {
                                section_hint = Some(i);
                            }
                        }
                        // One integer compare per captured write; a crossing is one push.
                        if entries {
                            for write in &section_moves[start..] {
                                if let Some(from) = write.from
                                    && crate::game::entity_sections::crossed(
                                        [from.x, from.y, from.z],
                                        [write.to.x, write.to.y, write.to.z],
                                    )
                                {
                                    section_entered.push((write.id, write.to.components()));
                                }
                            }
                        }
                    }
                    if let Some(selection) = &selection
                        && selection.has_switched()
                        && let Some(player) = selection.selected()
                    {
                        if let Some(conn) = player.connection {
                            bindings.insert(m.id, conn);
                        } else {
                            bindings.remove(&m.id);
                        }
                    }
                    m.despawn = mod_api::Despawn::default();
                    if seen_guest.is_none() {
                        base.player = w.player;
                    } else if let Some(k) = lent {
                        spare = w.player.map(|v| std::mem::replace(&mut guests[k].1, v));
                    }
                    sunrise_client::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                    if !kept && !m.removed && !m.alive() {
                        // LivingEntity.tickDeath: after twenty ticks the body is removed.
                        let split = m.death_time >= 20
                            && m.extra.size > 1
                            && matches!(
                                m.species(),
                                Species::Slime | Species::MagmaCube | Species::SulfurCube
                            );
                        if m.death_time >= 20 {
                            let (width, height) = (f64::from(m.width()), f64::from(m.height()));
                            gone.push((m.id, m.position, width, height));
                        }
                        // AbstractCubeMob.remove splits before LivingEntity.remove runs
                        // removal effects. Finish both while the dying parent still resolves.
                        if split || !m.extra.effects.is_empty() {
                            removal = Some((m.join, split));
                        }
                    }
                    route_player_events(
                        &mut events,
                        from,
                        seen_guest,
                        &guest_ids,
                        &mut guest_events,
                        m.id,
                    );
                    // Killed during its own tick (drowning, burning, falling...).
                    if !m.alive() && !m.removed && died.insert(m.id) {
                        dead.push(m.clone());
                    }
                    if !kept {
                        died.remove(&m.id);
                    }
                    keep[slot] = kept;
                    gone_rows |= !kept;
                    // Its own Post, right after its tick, also when it died or went in it.
                    if round.post() {
                        tell_post = Some((m.id, std::sync::Arc::clone(&m.kind)));
                    }
                    // ServerLevel ticks entities one after another: the ones
                    // after this mob see it as it is now.
                    snapshot[slot] = seen(m, uuid, world_items!(self));
                    snapshot[slot].alive &= kept;
                    snapshot[slot].removed |= !kept;
                    grid.update(slot, snapshot[slot].bounds);
                    let id = m.id;
                    brain_from = from;
                    let before_brain = mobs.stamp();
                    sting = sunrise_client::mobs::apply_brain_events_until_hurt(
                        &mut mobs,
                        &mut level,
                        &mut events,
                        from,
                        &base.teams,
                        true,
                    );
                    views_dirty |= mobs.stamp() != before_brain;
                    if sting.is_none()
                        && events[from..]
                            .iter()
                            .any(|e| matches!(e, MobEvent::EffectMob { .. }))
                    {
                        let mut idx = from;
                        while idx < events.len() {
                            if matches!(events[idx], MobEvent::EffectMob { .. }) {
                                pending_effects.push(events.remove(idx));
                            } else {
                                idx += 1;
                            }
                        }
                    }
                    cursor += 1;
                    if !kept
                        && !was_removed
                        && (mobs[i].extra.mount.is_some()
                            || self.has_seated_passengers_with_lent_mobs(id, &mobs))
                    {
                        removing_vehicle = Some(id);
                        break;
                    }
                    if sting.is_some()
                        || section_hint.is_some()
                        || tell_post.is_some()
                        || !pending_effects.is_empty()
                        || removal.is_some()
                        || lid_push.is_some()
                    {
                        break;
                    }
                }
            }
            self.creatures.lan_sounds.append(&mut hit_sounds);
            if sting.is_some() {
                resume_brain = Some((brain_from, section_hint.take(), tell_post.take()));
            }
            let stinging = sting.is_some();
            if pause.is_none()
                && !stinging
                && retry_rider.is_none()
                && removing_vehicle.is_none()
                && section_hint.is_none()
                && stand_cut.is_none()
                && lid_push.is_none()
                && tell_pre.is_none()
                && tick_enchantments.is_none()
                && tell_post.is_none()
                && pending_effects.is_empty()
                && removal.is_none()
            {
                break;
            }
            #[cfg(test)]
            BORROWED_MOB_YIELDS.with(|count| count.set(count.get() + 1));
            // No MobWorld, DamageLevel or navigation loan crosses this boundary.
            // The callback sees the real owning rows and level RNG/attachments,
            // and its writes are the state the continuation will resume from.
            section_writes::set_enabled(false);
            lent.give_back(&mut self.world);
            // Earlier mobs' pushes become visible before a real pause callback
            // can read or replace another actor's velocity. A section-only
            // or a told-alone yield must not change the normal end-of-pass
            // push ordering.
            // Equipment detection alone, like a tick hook, does not settle queued pushes.
            let settle = (pause.is_some() && !matches!(pause, Some(Pause::Equipment { .. })))
                || stinging
                || retry_rider.is_some()
                || removing_vehicle.is_some()
                || removal.is_some()
                || stand_cut.is_some()
                || lid_push.is_some();
            let applied_pushes = !pushes.is_empty() && settle;
            if applied_pushes {
                for m in &mut mobs {
                    sunrise_client::mobs::apply_mob_pushes(m, &mut pushes);
                }
            }
            // A yield just to tell a tick hook or a crossing doesn't by itself
            // change the list. Keep the same slots unless rows have left or joined.
            let same_rows = !settle && !gone_rows && pending_effects.is_empty();
            if !same_rows {
                retain_mob_rows(&mut mobs, &mut player_credit, |m| {
                    // Give the departing vehicle or cube/effect removal back for its pre-removal
                    // effects, splitting and marking it removed before dismount hooks.
                    removing_vehicle == Some(m.id)
                        || removal.is_some_and(|(join, _)| join == m.join)
                        || index
                            .get(&m.id)
                            .and_then(|&slot| keep.get(slot as usize))
                            .copied()
                            .unwrap_or(true)
                });
                gone_rows = false;
            }
            // Mobs that joined during the pass go first on a restore: the rows move.
            let restored_offset = self.mobs.len();
            let same_rows = same_rows && restored_offset == 0;
            self.mobs
                .restore(mobs, sunrise_client::join_seq::RestoreOrder::SpawnsFirst);
            let mut restored = Some(self.mobs.stamp());
            self.file_mob_section_entries(&mut section_entered);
            std::mem::swap(&mut self.last_hurt_by_player, &mut player_credit);
            std::mem::swap(&mut self.died, &mut died);
            std::mem::swap(&mut self.creatures.ticked, &mut ticked);
            if let Some(host) = self.lan.as_mut() {
                std::mem::swap(&mut host.mob_players, &mut bindings);
            }
            // Yield callbacks run on the owning game, not the borrowed mob.
            // Swap the live section table back in so every section update,
            // removal and nested listener move hits one authoritative table.
            let breeding = matches!(pause, Some(Pause::Breeding { .. }));
            if let Some(sections) = section_table.as_ref() {
                std::mem::swap(
                    &mut self.native_lifecycle.entity_sections,
                    &mut sections.borrow_mut(),
                );
            }
            // A work step's restock reaches the screen before any callback can edit it.
            let callback_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                for restock in state.restocks.drain(..) {
                    self.villager_offers_open(restock);
                }
                if let Some(index) = section_hint {
                    let written = self.entity_sections_ticked_live(
                        &mut section_moves,
                        Some(restored_offset + index),
                    );
                    // Restaging the committed position isn't a listener write.
                    restored = (!written).then(|| self.mobs.stamp());
                }
                if !pending_effects.is_empty() {
                    let _ = self.handle_mob_events(std::mem::take(&mut pending_effects));
                }
                if removal.is_some() || breeding {
                    // Consume completed removals before calling a listener which can unwind.
                    self.emit_death_poofs(gone.drain(..));
                }
                let mut vehicle_split = false;
                if let Some((join, split)) = removal {
                    if removing_vehicle.is_some_and(|id| {
                        self.mob_index
                            .find(&self.mobs, id)
                            .is_some_and(|i| self.mobs[i].join == join)
                    }) {
                        vehicle_split = split;
                    } else {
                        self.complete_mob_removal(join, split);
                    }
                }
                if let Some(mut sting) = sting {
                    let before_hurt = events.drain(brain_from..sting.at).collect();
                    let _ = self.handle_mob_events(before_hurt);
                    if let Some(i) = self.mob_index.find(&self.mobs, sting.target)
                        && sting.same_target(&self.mobs[i])
                    {
                        if let Some(hit) = paused_hurt.take() {
                            self.paused_mob_hurt = Some(hit);
                            if let Some((i, hurt)) = self.complete_mob_hurt(i, false) {
                                sting.resolved_hit(hurt);
                                self.finish_mob_damage_at(i);
                            } else {
                                sting.resolved_hit(false);
                            }
                        } else {
                            self.finish_mob_damage_at(i);
                        }
                        self.settle_creaking_hurt(sting.target);
                    }
                    if self
                        .mob_object(sting.fish)
                        .is_some_and(|mob| sting.same_fish(mob))
                    {
                        if let Some(ticks) = sting.poison_ticks()
                            && self
                                .mob_object(sting.target)
                                .is_some_and(|mob| sting.same_target(mob))
                        {
                            self.add_mob_effect(
                                sting.target,
                                sunrise_client::effects::EffectInstance::new("poison", ticks, 0),
                                self.entity(sting.fish),
                            );
                        }
                        if let Some(sound) = self
                            .mob_object(sting.fish)
                            .and_then(|fish| sting.sound(fish))
                        {
                            let _ = self.handle_mob_events(vec![sound]);
                        }
                    } else {
                        events.truncate(brain_from);
                        resume_brain = None;
                    }
                } else if let Some(id) = retry_rider {
                    self.dismount_mob(id);
                } else if let Some((paused, (id, join))) = pause.as_mut().zip(resume_entity)
                    && let Some(i) = self.mob_index.find(&self.mobs, id)
                    && self.mobs[i].join == join
                    && !self.mobs[i].removed
                {
                    match paused {
                        Pause::Consumption { decision, .. } => {
                            self.apply_fox_consumption(decision);
                        }
                        Pause::Drink { stack, .. } => {
                            self.finish_mob_drink(id, stack);
                        }
                        Pause::Breathing { air, .. } => {
                            air_answer = self.ask_mob_breath(air);
                        }
                        Pause::Equipment { first, .. } => {
                            self.detect_mob_equipment_updates(id, join, *first);
                        }
                        Pause::RegisterBehaviors { .. } => {
                            self.flush_resident_mob_parts(id, join);
                        }
                        Pause::Breeding { event, .. } => {
                            if let Some(event) = event.take() {
                                self.deliver_breed(*event, true);
                            }
                        }
                        Pause::AfterDamage(_) => {
                            if self.mobs[i].has_protected_hurt()
                                || sunrise_client::mobs::creaking::has_pending_hurt(&self.mobs[i])
                            {
                                // The earlier emissions (e.g. drowning bubbles) precede this hurt.
                                let before_hurt = events.split_off(resume_from);
                                let _ = self.handle_mob_events(before_hurt);
                            }
                            if let Some(i) = self.mob_index.find(&self.mobs, id)
                                && self.mobs[i].join == join
                                && !self.mobs[i].removed
                            {
                                self.paused_mob_hurt = paused_hurt.take();
                                self.finish_mob_damage_at(i);
                                self.settle_creaking_hurt(id);
                            }
                        }
                        Pause::Trample(trample) => {
                            self.trample_farmland(id, false, *trample);
                        }
                        Pause::FallToPassengers(fall) => {
                            self.fall_to_passengers(id, fall.distance, fall.modifier);
                        }
                        Pause::InsideEffects(inside) => {
                            let effects = std::mem::take(&mut inside.effects);
                            self.flush_mob_inside_live(inside.mob, inside.join, effects);
                        }
                    }
                }
                if let Some(id) = removing_vehicle {
                    // LivingEntity.tickDeath broadcasts event 60 before remove;
                    // cube splitting and the killed effects also precede setRemoved.
                    let animation = gone.pop_if(|death| death.0 == id);
                    let killed_effects = removal.is_some();
                    let animate = |game: &mut Self| {
                        if let Some((id, at, width, height)) = animation {
                            game.emit_death_poofs([(id, at, width, height)]);
                        }
                        if vehicle_split && let Some(i) = game.mob_index.find(&game.mobs, id) {
                            let mut dying = game.mobs[i].clone();
                            game.split_cube_children(&mut dying);
                        }
                    };
                    if self.mob_object(id).is_some_and(|mob| mob.removed) {
                        animate(self);
                        self.finish_mob_removal(id);
                    } else {
                        self.remove_mob_after(id, killed_effects, animate);
                    }
                    if let Some(index) = self.mobs.iter().position(|mob| mob.id == id) {
                        self.mobs.remove(index);
                    }
                }
                if let Some((shulker, area, delta)) = lid_push {
                    self.apply_shulker_lid_push(shulker, area, delta);
                }
                if let Some(cut) = stand_cut {
                    *state.stand_result = self.tick_armor_stands_before(cut);
                }
                if !stinging && let Some((id, kind)) = &tell_post {
                    self.native_tick_post_one(round, *id, &kind.id);
                }
                if let Some((id, join)) = tick_enchantments
                    && self
                        .mob_object(id)
                        .is_some_and(|mob| mob.join == join && !mob.removed)
                {
                    self.tick_equipment_enchantments(crate::game::commands::Target::Mob(id), id);
                    // A tick effect may move its actor or change state without changing list membership.
                    views_dirty = true;
                }
                // Let through, it checks its sleep as a mob told with its family does.
                if let Some((id, kind, at)) = &tell_pre
                    && self.native_tick_pre_one(round, *id, &kind.id)
                    && mod_api::__private::listened(mod_api::HookId::CanContinueSleeping)
                    && let Some(i) =
                        super::super::native_ticks::place(&self.mobs, *at, |m| m.id == *id)
                {
                    self.sync_sleeping_mob(i);
                }
            }))
            .err();
            if let Some(sections) = section_table.as_ref() {
                std::mem::swap(
                    &mut self.native_lifecycle.entity_sections,
                    &mut sections.borrow_mut(),
                );
            }
            let unchanged = same_rows && !views_dirty && restored == Some(self.mobs.stamp());
            if tell_pre.is_some() || tick_enchantments.is_some() {
                told_at = Some(cursor);
            }
            if tick_enchantments.is_some() {
                enchantment_tick_at = Some(cursor);
            }
            tell_post = None;
            if retry_rider.is_some() || removing_vehicle.is_some() {
                blocked = std::borrow::Cow::Owned(self.mobs_below_off_level());
            }
            mobs = self.mobs.lift();
            std::mem::swap(&mut self.last_hurt_by_player, &mut player_credit);
            std::mem::swap(&mut self.died, &mut died);
            std::mem::swap(&mut self.creatures.ticked, &mut ticked);
            if let Some(host) = self.lan.as_mut() {
                std::mem::swap(&mut host.mob_players, &mut bindings);
            }
            if callback_panic.is_some() || state.stand_result.is_err() {
                lent = sunrise_client::health::LentLevel::take(&mut self.world);
                dispatch_panic = callback_panic;
                break;
            }
            guests = self.lan_guest_views();
            guest_slots = guest_indices(&guests);
            guest_ids = self
                .lan
                .as_ref()
                .map(|host| {
                    host.hub
                        .guests
                        .iter()
                        .map(|g| (g.conn, g.entity_id as u32))
                        .collect()
                })
                .unwrap_or_default();
            spare = guests.first().map(|g| g.1.clone());
            // Apply actual writes, flushed pushes, and movement or damage pauses
            // to the rest of the pass. Index the live rows by the existing snapshot slots.
            if !unchanged || applied_pushes {
                positions.clear();
                positions.resize(snapshot.len(), None);
                for (i, m) in mobs.iter().enumerate() {
                    let current = seen(m, uuid, world_items!(self));
                    match index.entry(m.id) {
                        Entry::Occupied(entry) => {
                            let slot = *entry.get() as usize;
                            snapshot[slot] = current;
                            positions[slot] = Some(i);
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(snapshot.len() as u32);
                            snapshot.push(current);
                            positions.push(Some(i));
                        }
                    }
                }
                villager_slots.clear();
                solid_slots.clear();
                for (slot, (s, position)) in snapshot.iter_mut().zip(&positions).enumerate() {
                    let present = position.is_some() && keep.get(slot).copied().unwrap_or(true);
                    s.alive &= present;
                    s.removed |= !present;
                    if s.species == Species::Villager {
                        villager_slots.push(slot as u32);
                    }
                    if s.may_block_mobs() {
                        solid_slots.push(slot as u32);
                    }
                }
                grid = sunrise_client::mobs::seen_grid::SeenGrid::new(
                    snapshot.iter().map(|s| s.bounds),
                );
            }
            if let Some(writes) = self.world.path_writes.as_mut() {
                path_writes.append(writes);
            }
            lent = sunrise_client::health::LentLevel::take(&mut self.world);
            resume = pause;
            resume_air_answer = air_answer;
            retried_rider = retry_rider;
            section_writes::set_enabled(capture_sections);
        }
        laps.lap("creature_loop");
        if dispatch_panic.is_none() && state.stand_result.is_ok() {
            if positions.is_empty() {
                let mut flags = keep.into_iter();
                retain_mob_rows(&mut mobs, &mut player_credit, |_| {
                    flags.next().unwrap_or(true)
                });
            } else {
                retain_mob_rows(&mut mobs, &mut player_credit, |m| {
                    index
                        .get(&m.id)
                        .and_then(|&slot| keep.get(slot as usize))
                        .copied()
                        .unwrap_or(true)
                });
            }
            for m in &mut mobs {
                sunrise_client::mobs::apply_mob_pushes(m, &mut pushes);
            }
        }
        section_writes::finish();
        drop(live_section_observer);
        // Keep the reusable write storage for the next pass; the live-row callback has already
        // consumed every record that belonged to this family tick.
        section_moves.clear();
        if let Some(table) = section_table {
            let mut sections = table.borrow_mut();
            sections.ticked = section_moves;
            std::mem::swap(&mut self.native_lifecycle.entity_sections, &mut sections);
        } else {
            self.native_lifecycle.entity_sections.ticked = section_moves;
        }
        self.file_mob_section_entries(&mut section_entered);
        self.native_lifecycle.entity_sections.entered = section_entered;
        *state.pushes = pushes;
        *state.mobs = mobs;
        *state.snapshot = snapshot;
        *state.events = events;
        *state.dead = dead;
        *state.died = died;
        *state.ticked = ticked;
        *state.guests = guests;
        *state.spare = spare;
        *state.bindings = bindings;
        *state.guest_ids = guest_ids;
        *state.guest_events = guest_events;
        *state.player_credit = player_credit;
        *state.path_writes = path_writes;
        *state.gone = gone;
        (lent, dispatch_panic)
    }

    /// Files the section entries of the pass so far, as `Callback.onMove`, in the order
    /// of the writes.
    fn file_mob_section_entries(&mut self, entered: &mut Vec<(u32, [f64; 3])>) {
        let table = self.resident_entities.table.get_mut();
        for (id, to) in entered.drain(..) {
            table.entered_section(id, to);
        }
    }
}

impl Game<'_> {
    /// The killing blow of a mob as the death hooks name it: its damage
    /// type and the causing entity's id (0 for none).
    pub(in crate::game) fn mob_damage_source(&self, m: &Mob) -> (&'static str, u32) {
        let (damage_type, attacker) = match m.last_damage {
            Some((d, _)) => (source_damage_type(d), d.attacker),
            None => ("minecraft:generic", None),
        };
        let attacker = match attacker {
            Some(Who::Mob(id)) => id,
            Some(Who::Player) => self
                .last_hurt_by_player
                .get(&m.id)
                .and_then(|credit| self.resolve_player_credit(*credit))
                .and_then(|conn| self.player_entity_id_by_connection(conn))
                .map_or(0, |id| id as u32),
            None => 0,
        };
        (damage_type, attacker)
    }
}

impl Game<'_> {
    fn mob_can_freeze(&self, m: &Mob) -> bool {
        if m.species() == Species::Skeleton {
            return false;
        }
        if m.species() == Species::SulfurCube && sunrise_client::mobs::sulfur_cube::has_body(m) {
            return false;
        }
        if world_items!(self)
            .entity_type_tags
            .contains("minecraft:freeze_immune_entity_types", &m.kind.id)
        {
            return false;
        }
        let immune = world_items!(self)
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        ![
            m.extra.body_id(),
            m.extra.head_id(),
            m.extra.slot_id(sunrise_items::equipment::CHEST),
            m.extra.slot_id(sunrise_items::equipment::LEGS),
            m.extra.slot_id(sunrise_items::equipment::FEET),
        ]
        .into_iter()
        .flatten()
        .any(|item| immune.is_some_and(|t| t.contains(item)))
    }

    pub(in crate::game) fn flush_mob_inside_live(
        &mut self,
        mob_id: u32,
        join: sunrise_client::join_seq::JoinSeq,
        mut effects: mod_api::InsideEffects,
    ) {
        if effects.is_empty() {
            return;
        }
        let Some(slot) = self.mob_index.find(&self.mobs, mob_id) else {
            return;
        };
        let mut access = GameMobInsideAccess {
            game: self,
            mob_id,
            join,
            slot,
        };
        effects.apply_and_clear(&mut access);
    }

    pub(super) fn retained_passenger_snapshot(&self, uuid: Option<[i32; 4]>) -> Vec<Seen> {
        if self.persist.off_level.is_empty() {
            Vec::new()
        } else {
            self.persist
                .off_level
                .values()
                .filter_map(crate::game::persist::OffLevel::as_mob)
                .map(|m| seen(m, uuid, self.items()))
                .collect()
        }
    }
}

struct GameMobInsideAccess<'a, 'g> {
    game: &'a mut Game<'g>,
    mob_id: u32,
    join: sunrise_client::join_seq::JoinSeq,
    slot: usize,
}

impl GameMobInsideAccess<'_, '_> {
    fn slot(&self) -> Option<usize> {
        super::super::native_ticks::place(&self.game.mobs, self.slot, |m| {
            m.id == self.mob_id && m.join == self.join && !m.removed
        })
    }

    fn mob(&self) -> Option<&Mob> {
        Some(&self.game.mobs[self.slot()?])
    }

    fn mob_mut(&mut self) -> Option<&mut Mob> {
        self.slot = self.slot()?;
        Some(&mut self.game.mobs[self.slot])
    }
}

impl mod_api::InsideEntityAccess for GameMobInsideAccess<'_, '_> {
    #[inline]
    fn id(&self) -> u32 {
        self.mob_id
    }

    #[inline]
    fn kind(&self) -> &str {
        self.mob().map_or("", |m| m.kind.id.as_str())
    }

    #[inline]
    fn is_alive(&self) -> bool {
        self.mob().is_some_and(Mob::alive)
    }

    #[inline]
    fn position(&self) -> [f64; 3] {
        self.mob()
            .map_or([0.0; 3], |m| [m.position.x, m.position.y, m.position.z])
    }

    #[inline]
    fn velocity(&self) -> [f64; 3] {
        self.mob()
            .map_or([0.0; 3], |m| [m.velocity.x, m.velocity.y, m.velocity.z])
    }

    #[inline]
    fn fire_ticks(&self) -> i32 {
        self.mob().map_or(0, |m| m.fire_ticks)
    }

    #[inline]
    fn is_on_fire(&self) -> bool {
        self.mob().is_some_and(Mob::is_on_fire)
    }

    #[inline]
    fn fall_distance(&self) -> f64 {
        self.mob().map_or(0.0, |m| m.fall_distance)
    }

    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        use crate::game::api_values::TryIntoApi as _;
        let level = self.game.level(self.game.dim.id())?;
        self.game
            .level_block_state(level, pos)
            .and_then(|id| id.try_into_api().ok())
    }

    #[inline]
    fn push(&mut self, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(m) = self.mob_mut() else {
            return false;
        };
        m.velocity = m
            .velocity
            .add_vector(Vec3::new(delta[0], delta[1], delta[2]));
        true
    }

    #[inline]
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(m) = self.mob_mut() else {
            return false;
        };
        m.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }

    #[inline]
    fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool {
        if !speed_multiplier.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(m) = self.mob_mut() else {
            return false;
        };
        m.fall_distance = 0.0;
        m.stuck_multiplier = Vec3::new(
            speed_multiplier[0],
            speed_multiplier[1],
            speed_multiplier[2],
        );
        true
    }

    #[inline]
    fn reset_fall_distance(&mut self) {
        if let Some(m) = self.mob_mut() {
            m.fall_distance = 0.0;
        }
    }

    #[inline]
    fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        if let Some(m) = self.mob_mut() {
            m.fire_ticks = ticks;
        }
    }

    #[inline]
    fn add_effect(&mut self, effect: mod_api::EffectKey, duration: i32, amplifier: u8) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_client_core::potions::effect_by_index)
        else {
            return false;
        };
        let instance =
            sunrise_client::effects::EffectInstance::from_def(def, duration, i32::from(amplifier));
        self.game.add_mob_effect(self.mob_id, instance, None)
    }

    #[inline]
    fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
    ) -> bool {
        let Some(m) = self.mob() else {
            return false;
        };
        let subject_pos = [m.position.x, m.position.y, m.position.z];
        let Some((direct, causing, from)) = self.game.resolve_inside_damage_sources(
            self.mob_id,
            crate::game::commands::Target::Mob(self.mob_id),
            subject_pos,
            direct,
            causing,
            None,
        ) else {
            return false;
        };
        self.game.hurt_target(
            crate::game::commands::Target::Mob(self.mob_id),
            amount,
            damage_type,
            direct,
            causing,
            from,
        )
    }

    #[inline]
    fn set_block(&mut self, pos: [i32; 3], state: mod_api::BlockState) -> bool {
        use crate::game::api_values::TryIntoEngine as _;
        let Some(level) = self.game.level(self.game.dim.id()) else {
            return false;
        };
        let Ok(id) = state.try_into_engine(&*self.game) else {
            return false;
        };
        let flags = mod_api::BlockUpdateFlags::NEIGHBORS | mod_api::BlockUpdateFlags::CLIENTS;
        self.game
            .level_set_block(level, pos, id, flags)
            .unwrap_or(false)
    }

    #[inline]
    fn destroy_block(&mut self, pos: [i32; 3], drop: bool) -> bool {
        let Some(level) = self.game.level(self.game.dim.id()) else {
            return false;
        };
        self.game
            .level_destroy_block(level, pos, drop, None, None, false)
            .is_some()
    }

    #[inline]
    fn apply_effect(&mut self, effect: mod_api::InsideEffect) {
        match effect {
            mod_api::InsideEffect::Freeze => {
                let can_freeze = self.mob().is_some_and(|m| self.game.mob_can_freeze(m));
                if let Some(m) = self.mob_mut() {
                    m.in_powder_snow = true;
                    if can_freeze {
                        m.ticks_frozen = (m.ticks_frozen + 1)
                            .min(sunrise_client::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
                    }
                }
            }
            mod_api::InsideEffect::ClearFreeze => {
                if let Some(m) = self.mob_mut() {
                    m.ticks_frozen = 0;
                }
            }
            mod_api::InsideEffect::FireIgnite => {
                if let Some(m) = self.mob_mut() {
                    m.fire_ignite_contact();
                }
            }
            mod_api::InsideEffect::LavaIgnite => {
                if let Some(m) = self.mob_mut()
                    && !m.fire_immune()
                {
                    m.ignite_for_ticks(300);
                }
            }
            mod_api::InsideEffect::Extinguish => {
                if let Some(m) = self.mob_mut() {
                    m.fire_ticks = m.fire_ticks.min(0);
                }
            }
        }
    }

    #[inline]
    fn apply_builtin(&mut self, action: mod_api::__private::BuiltinInsideAction) {
        match action {
            mod_api::__private::BuiltinInsideAction::PowderSnowMelt(pos) => {
                if self.mob().is_some_and(Mob::is_on_fire)
                    && self.game.can_entity_grief(Some(self.mob_id))
                    && let Some(level) = self.game.level(self.game.dim.id())
                {
                    let _ = self
                        .game
                        .level_destroy_block(level, pos, false, None, None, false);
                }
            }
            mod_api::__private::BuiltinInsideAction::LayeredCauldronExtinguish { pos, state } => {
                if self.mob().is_some_and(Mob::is_on_fire) {
                    let [x, y, z] = pos;
                    self.game.cauldron_extinguish_at(
                        sunrise_core::position::BlockPos::new(x, y, z),
                        sunrise_world::registry::BlockStateId(state),
                    );
                }
            }
            mod_api::__private::BuiltinInsideAction::FireHurt(damage) => {
                self.hurt_with_source("minecraft:in_fire", damage, None, None);
            }
            mod_api::__private::BuiltinInsideAction::LavaHurt => {
                if self.mob().is_some_and(|m| !m.fire_immune())
                    && self.hurt_with_source("minecraft:lava", 4.0, None, None)
                    && let Some(m) = self.mob_mut().filter(|m| !m.silent)
                {
                    let at = m.position.components();
                    let pitch = 2.0 + m.random.next_float() * 0.4;
                    self.game
                        .audio
                        .play("minecraft:entity.generic.burn", Some(at), 0.4, pitch);
                }
            }
        }
    }
}

#[cfg(test)]
mod off_level_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn borrowed_tick_resolves_the_retained_controller() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            use sunrise_client::mobs::goals::{JUMP, LOOK, MOVE};
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let spider = game
                .res
                .mob_types()
                .get("minecraft:spider")
                .unwrap()
                .clone();
            let skeleton = game
                .res
                .mob_types()
                .get("minecraft:skeleton")
                .unwrap()
                .clone();
            let at = Vec3::new(1.0, 80.0, 0.5);
            let mut vehicle = Mob::new(71, spider, at, 0.0);
            vehicle.extra.rider = Some(72);
            let mut rider = Mob::new(72, skeleton, at, 0.0);
            rider.extra.mount = Some(71);
            rider.age = 19;
            game.mobs.push(vehicle);
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(rider));
            game.tick_creature_entities().unwrap();
            assert_eq!(
                game.mob_object(71).unwrap().ai.goals.disabled & (MOVE | JUMP | LOOK),
                MOVE | JUMP | LOOK
            );
            assert_eq!(game.mob_object(72).unwrap().age, 19);
            assert_eq!(game.mobs.len(), 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resident_descendant_does_not_tick_until_detached() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut parent = Mob::new(72, pig.clone(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            parent.extra.mount = Some(71);
            let mut child = Mob::new(73, pig, parent.position, 0.0);
            child.extra.mount = Some(72);
            child.no_ai = true;
            child.age = 19;
            child.fire_ticks = 20;
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(parent));
            game.mobs.push(child);
            game.tick_creature_entities().unwrap();
            let child = game.mob_object(73).unwrap();
            assert_eq!(child.age, 19);
            assert_eq!(child.fire_ticks, 20);
            game.mob_object_mut(73).unwrap().extra.mount = None;
            game.tick_creature_entities().unwrap();
            assert_eq!(game.mob_object(73).unwrap().age, 20);
        });
    }
}

#[cfg(test)]
mod hoglin_routing_tests {
    use super::*;

    #[test]
    fn broadcast_keeps_source_player_connection() {
        for (seen_guest, tag, expected) in [
            (Some(12), None, Some(12)),
            (None, None, None),
            (Some(12), Some(Some(13)), Some(13)),
            (Some(12), Some(None), None),
        ] {
            let event = MobEvent::HoglinAttackTarget {
                mob: 9001,
                target: Who::Player,
                player: None,
            };
            let mut events = vec![match tag {
                Some(conn) => MobEvent::ForPlayer {
                    conn,
                    event: Box::new(event),
                },
                None => event,
            }];
            route_player_events(&mut events, 0, seen_guest, &[], &mut Vec::new(), 9000);
            assert!(matches!(events[0], MobEvent::HoglinAttackTarget {
                player, ..
            } if player == expected));
        }
    }
}

#[cfg(test)]
#[path = "mob_tick_access_tests.rs"]
mod mob_tick_access_tests;
