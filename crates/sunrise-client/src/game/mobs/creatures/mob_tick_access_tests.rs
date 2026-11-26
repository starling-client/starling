//! #795, the mob slice: a handle fired inside a mob's own tick resolves the
//! mob the tick holds out of the game, without a copy or a placeholder.
use super::*;
use crate::game::Dim;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{Despawn, EntityTickPreEvent, HostSide, ModEntry, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use sunrise_client::chunk::Chunk;

/// The `mob_despawn` facts a listener reads through its handle: its own
/// position, and another mob's through a nested lookup.
#[derive(Debug, Default)]
struct DespawnRecords(Vec<(u32, Option<[f64; 3]>, Option<[f64; 3]>)>);

struct DespawnProbe {
    records: Arc<Mutex<DespawnRecords>>,
    allow: Arc<AtomicU32>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for DespawnProbe {
    fn mob_despawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let id = event.mob.entity().id();
        let own = event.mob.entity().position(host);
        let other_id = if id == 71 { 72 } else { 71 };
        let other = host
            .entity(other_id)
            .and_then(|entity| entity.position(host));
        self.records.lock().unwrap().0.push((id, own, other));
        if id == self.allow.load(Ordering::Relaxed) {
            *event.result = Despawn::Allow;
        }
    }
}

/// Reads and moves the mob in its `entity_tick_pre`, through its handle.
struct MoveOnPre {
    id: u32,
    seen: Arc<Mutex<Vec<Option<[f64; 3]>>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for MoveOnPre {
    fn entity_tick_pre(
        &self,
        host: HostSide<'_>,
        event: &mut EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        if event.entity == self.id {
            let entity = host.entity(self.id).expect("a live handle in its own Pre");
            let at = entity
                .position(host)
                .expect("the tick view reads its position");
            self.seen.lock().unwrap().push(Some(at));
            assert!(
                entity.teleport_to(host, [at[0], at[1] + 10.0, at[2]], None),
                "the tick view writes its position"
            );
        }
        mod_api::Flow::Continue
    }
}

/// Two idle pigs in an entity-ticking chunk, at x = 1 and x = 2, ready for
/// the real creature pass.
fn with_two_pigs(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::env::temp_dir().join(format!(
        "sunrise-mob-tick-access-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    with_game(|game| {
        game.world_root = root.clone();
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(
            game.entity_ticking_area()(1, 0),
            "fixture chunk is not entity-ticking"
        );
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        for (id, x) in [(71u32, 1.0f64), (72u32, 2.0f64)] {
            let mut mob = Mob::new(id, kind.clone(), Vec3::new(x, 80.0, 0.5), 0.0);
            mob.no_ai = true;
            mob.no_gravity = true;
            mob.ground.on_ground = true;
            game.mobs.push(mob);
        }
        test(game);
    });
    let _ = std::fs::remove_dir_all(&root);
}

/// Removes the mob it names, and records what every despawn callback reads of
/// that mob's handle: the victim's own callback runs before the removal, a
/// later callback after it.
struct RemovedPeerProbe {
    victim: u32,
    records: Arc<Mutex<Vec<(u32, Option<[f64; 3]>)>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for RemovedPeerProbe {
    fn mob_despawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let id = event.mob.entity().id();
        let victim = host
            .entity(self.victim)
            .and_then(|entity| entity.position(host));
        self.records.lock().unwrap().push((id, victim));
        if id == self.victim {
            *event.result = Despawn::Allow;
        }
    }
}

/// The view's removed state is authoritative: once the first mob is removed by
/// its own callback, a later callback asking for its handle gets nothing, even
/// though the removed row stays indexed with its old position.
#[test]
#[ignore = "daily"]
fn a_removed_mob_handle_resolves_nothing_in_a_later_callback() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-removed-peer",
        Box::new(RemovedPeerProbe {
            victim: 71,
            records: records.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            let records = records.lock().unwrap();
            assert_eq!(records.len(), 2, "both mobs heard their despawn check");
            // 71 runs first and is removed only after its own answer.
            assert_eq!(records[0].0, 71);
            assert_eq!(
                records[0].1,
                Some([1.0, 80.0, 0.5]),
                "the victim still resolves inside its own callback"
            );
            // 72 runs after 71's removal: its row is indexed but marked removed.
            assert_eq!(records[1].0, 72);
            assert_eq!(
                records[1].1, None,
                "a removed mob's handle resolves nothing"
            );
            assert!(
                !game.mobs.iter().any(|mob| mob.id == 71),
                "the allowed mob was removed"
            );
            assert!(
                game.mobs.iter().any(|mob| mob.id == 72),
                "the denied mob stayed"
            );
        });
    });
}

/// The mob the pass holds out of the game answers its own handle, and the
/// handle of the other mob (the tick snapshot) too; `Allow` removes the
/// mob and its handle then resolves nothing.
#[test]
#[ignore = "daily"]
fn mob_despawn_handles_resolve_the_ticking_mob_and_its_peers() {
    let records = Arc::new(Mutex::new(DespawnRecords::default()));
    let allow = Arc::new(AtomicU32::new(72));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-access",
        Box::new(DespawnProbe {
            records: records.clone(),
            allow: allow.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            let records = records.lock().unwrap();
            assert_eq!(records.0.len(), 2, "both mobs heard their despawn check");
            for (id, own, peer) in &records.0 {
                let (expected_own, expected_peer) = match id {
                    71 => ([1.0, 80.0, 0.5], [2.0, 80.0, 0.5]),
                    72 => ([2.0, 80.0, 0.5], [1.0, 80.0, 0.5]),
                    _ => panic!("unexpected mob {id}"),
                };
                assert_eq!(*own, Some(expected_own), "own position, mob {id}");
                assert_eq!(*peer, Some(expected_peer), "different peer, mob {id}");
            }
            // `Allow` removed 72 during the pass; 71 is still there.
            assert!(
                game.mobs.iter().any(|mob| mob.id == 71),
                "the denied mob stayed"
            );
            assert!(
                !game.mobs.iter().any(|mob| mob.id == 72),
                "the allowed mob was removed"
            );
        });
    });
}

/// A mob's own `entity_tick_pre` handle reads where it stands and moves it;
/// the real tick then starts from the position the handle wrote.
#[test]
#[ignore = "daily"]
fn entity_tick_pre_reads_and_moves_the_ticking_mob() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-pre-access",
        Box::new(MoveOnPre {
            id: 71,
            seen: seen.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                &[Some([1.0, 80.0, 0.5])],
                "the Pre handle read the mob's position once"
            );
            let mob = game.mobs.iter().find(|mob| mob.id == 71).unwrap();
            assert_eq!(
                mob.position.y, 90.0,
                "the tick started from the position the handle wrote"
            );
        });
    });
}

/// Observations made by the actual scoped despawn hook, not by the spawn
/// fixture or a replacement host.
#[derive(Debug, PartialEq)]
struct PassObservation {
    actor: u32,
    peer_velocity: Option<[f64; 3]>,
    spawned_position: Option<[f64; 3]>,
    spawned_handles: [bool; 3],
}

struct PassProbe {
    records: Arc<Mutex<Vec<PassObservation>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for PassProbe {
    fn mob_despawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let spawned = mod_api::EntityRef::new(event.mob.entity().epoch(), 74);
        self.records.lock().unwrap().push(PassObservation {
            actor: event.mob.entity().id(),
            peer_velocity: host.entity(72).and_then(|peer| peer.velocity(host)),
            spawned_position: spawned.position(host),
            spawned_handles: [
                host.entity(74).is_some(),
                host.living(spawned).is_some(),
                host.mob(spawned).is_some(),
            ],
        });
        *event.result = Despawn::Deny;
    }
}

/// NoAI still drops motion below 0.003 during its real turn. The observers
/// bracket that turn, so a frozen initial snapshot or an all-post-turn view
/// cannot satisfy both assertions. The existing lifted-pass spawn fixture
/// inserts a real mob after the pass has taken its snapshot and storage;
/// observations still go through the actual scoped hook on both passes.
#[test]
#[ignore = "daily"]
fn scoped_despawn_view_tracks_peer_turns_and_defers_mid_pass_spawns() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-pass-visibility",
        Box::new(PassProbe {
            records: records.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.mobs[1].position.x = 4.0;
            game.mobs[1].velocity = Vec3::new(0.001, 0.0, 0.0);
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut observer = Mob::new(73, kind, Vec3::new(7.0, 80.0, 0.5), 0.0);
            observer.no_ai = true;
            observer.no_gravity = true;
            observer.ground.on_ground = true;
            game.mobs.push(observer);
            super::super::super::entity::taken_pass_tests::with_spawn(
                |game| {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut spawned = Mob::new(74, kind, Vec3::new(10.0, 80.0, 0.5), 0.0);
                    spawned.no_ai = true;
                    spawned.no_gravity = true;
                    spawned.ground.on_ground = true;
                    game.mobs.push(spawned);
                    // The ordinary host can already read the spawn buffer.
                    // Only the scoped view must keep it outside this pass.
                    let entity = game.entity(74).expect("spawn has a resident handle");
                    assert!(game.mob(entity).is_some());
                    assert_eq!(entity.position(game), Some([10.0, 80.0, 0.5]));
                },
                || game.tick_creature_entities().unwrap(),
            );
            assert_eq!(
                records.lock().unwrap().as_slice(),
                &[
                    PassObservation {
                        actor: 71,
                        peer_velocity: Some([0.001, 0.0, 0.0]),
                        spawned_position: None,
                        spawned_handles: [false; 3],
                    },
                    PassObservation {
                        actor: 72,
                        peer_velocity: Some([0.001, 0.0, 0.0]),
                        spawned_position: None,
                        spawned_handles: [false; 3],
                    },
                    PassObservation {
                        actor: 73,
                        peer_velocity: Some([0.0, 0.0, 0.0]),
                        spawned_position: None,
                        spawned_handles: [false; 3],
                    },
                ]
            );
            let spawned = game.mobs.iter().find(|mob| mob.id == 74).unwrap();
            assert_eq!(spawned.age, 0, "the mid-pass spawn has not taken a turn");
            records.lock().unwrap().clear();
            game.tick_creature_entities().unwrap();
            let records = records.lock().unwrap();
            assert_eq!(records.len(), 4, "the spawn joins the next pass");
            for actor in [71, 72, 73, 74] {
                let observation = records.iter().find(|record| record.actor == actor).unwrap();
                assert_eq!(observation.peer_velocity, Some([0.0, 0.0, 0.0]));
                assert_eq!(observation.spawned_position, Some([10.0, 80.0, 0.5]));
                assert_eq!(observation.spawned_handles, [true; 3]);
            }
        });
    });
}

/// Extends the lifetime of the dropped item it names, through `extra_life`.
struct ExtendLife {
    id: Arc<AtomicU32>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for ExtendLife {
    fn item_expire(&self, _: &mut dyn ServerHost, event: &mut mod_api::ItemExpireEvent) {
        if event.item.entity().id() == self.id.load(Ordering::Relaxed) {
            event.extra_life += 100;
        }
    }
}

/// The item family keeps its own contract while the mob slice changes: the
/// item tick still lifts its list, and an `item_expire` listener still edits
/// the item's lifetime, which the engine applies when the tick ends.
#[test]
#[ignore = "daily"]
fn item_lifetime_edit_in_the_item_tick_still_lands() {
    let named = Arc::new(AtomicU32::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "item-lifetime-access",
        Box::new(ExtendLife { id: named.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let root = std::env::temp_dir().join(format!(
            "sunrise-item-lifetime-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        with_game(|game| {
            game.world_root = root.clone();
            game.enter_level(Dim::Overworld).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.push_item_entity(
                ItemStack::new("minecraft:apple", 1),
                [1.5, 64.0, 1.5],
                Some([0.0, 0.0, 0.0]),
                0,
                None,
            );
            let id = game.dropped.last().unwrap().id;
            named.store(id, Ordering::Relaxed);
            game.ticked_entities_through = id;
            game.dropped.last_mut().unwrap().age = 6000;
            let mut round = game.native_tick_round();
            game.tick_dropped_items(&mut round);
            let item = game
                .dropped
                .iter()
                .find(|item| item.id == id)
                .expect("the extended item stayed");
            assert_eq!(
                item.lifespan, 6100,
                "the listener's 100 ticks extended the item's lifespan"
            );
            assert_eq!(
                item.age, 6001,
                "extending lifespan leaves the item's age intact"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    });
}

/// Discards another mob from the first mob's `entity_tick_pre`: a hook that
/// re-enters the entity handles while a tick is running.
struct DiscardOnPre {
    id: u32,
    victim: u32,
    seen: Arc<Mutex<Vec<(u32, bool, bool)>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for DiscardOnPre {
    fn entity_tick_pre(
        &self,
        host: HostSide<'_>,
        event: &mut EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        if event.entity == self.id {
            let victim = host
                .entity(self.victim)
                .expect("the other mob is live at the Pre");
            let discarded = victim.discard(host);
            let self_live = host.entity(self.id).is_some();
            self.seen
                .lock()
                .unwrap()
                .push((self.victim, discarded, self_live));
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_tick_hook_discards_another_mob_nested() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-nested-discard",
        Box::new(DiscardOnPre {
            id: 71,
            victim: 72,
            seen: seen.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                &[(72, true, true)],
                "the nested discard ran, and the actor's handle still resolved"
            );
            assert!(game.mobs.iter().any(|mob| mob.id == 71));
            assert!(!game.mobs.iter().any(|mob| mob.id == 72));
        });
    });
}

/// A mob in a loaded chunk whose own status is below entity-ticking is not on the entity tick
/// list: it fires no `mob_despawn` and stays, while a mob in the forced chunk beside it checks
/// despawning as usual.
#[test]
#[ignore = "daily"]
fn a_mob_outside_the_entity_tick_list_checks_no_despawn() {
    let records = Arc::new(Mutex::new(DespawnRecords::default()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-listed",
        Box::new(DespawnProbe {
            records: records.clone(),
            allow: Arc::new(AtomicU32::new(72)),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.world
                .chunks
                .insert((1, 0), Arc::new(Chunk::empty(1, 0)));
            game.mobs[1].position = Vec3::new(24.5, 80.0, 0.5);
            // Only chunk (0, 0) is forced: (1, 0) is loaded at the next level out.
            game.tickets
                .update(Some(sunrise_client::ticket_levels::Sources {
                    forced: vec![(0, 0)],
                    ..Default::default()
                }));
            assert!(game.entity_tick_listed_area()(1, 0));
            assert!(game.world.loaded(24, 0) && !game.entity_tick_listed_area()(24, 0));
            game.tick_creature_entities().unwrap();
            let heard: Vec<u32> = records.lock().unwrap().0.iter().map(|r| r.0).collect();
            assert_eq!(heard, [71], "only the listed mob checks despawning");
            assert!(
                game.mobs.iter().any(|mob| mob.id == 72),
                "the unlisted mob stays, though its listener would allow the despawn"
            );
        });
    });
}

/// What a tick listener heard: each call's phase and entities, and at a Post where the first
/// of them stands.
struct TickTiming {
    replace_cancels: bool,
    heard: Arc<Mutex<TickCalls>>,
}

/// A tick listener's calls, in order: (phase, entities, the first one's position at a Post).
type TickCalls = Vec<(&'static str, Vec<u32>, Option<[f64; 3]>)>;

#[mod_api::__subscriptions]
impl mod_api::Hooks for TickTiming {
    fn entity_tick_pre(
        &self,
        _: HostSide<'_>,
        event: &mut EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.heard.lock().unwrap();
        heard.push(("pre", vec![event.entity], None));
        if self.replace_cancels && event.entity == 71 {
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
    fn entity_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        let at = host
            .entity(event.entity)
            .and_then(|entity| entity.position(host));
        let mut heard = self.heard.lock().unwrap();
        heard.push(("post", vec![event.entity], at));
    }
}

/// The two pigs' tick hooks through the real creature pass; the first pig falls in its tick.
fn pig_ticks(replace_cancels: bool) -> TickCalls {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-timing",
        Box::new(TickTiming {
            replace_cancels,
            heard: heard.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            let first = game.mobs.iter().position(|mob| mob.id == 71).unwrap();
            game.mobs.update(first, |mob| {
                mob.no_ai = false;
                mob.no_gravity = false;
                mob.ground.on_ground = false;
                // Already falling: a mob moves by its velocity before gravity adds to it, so
                // one at rest would not move in its first tick.
                mob.velocity = Vec3::new(0.0, -0.5, 0.0);
            });
            game.tick_creature_entities().unwrap();
        });
    });
    std::mem::take(&mut *heard.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn lethal_burning_applies_totem_effects_before_the_effect_pass() {
    with_two_pigs(|game| {
        game.build_registries();
        game.mobs.update(0, |mob| {
            mob.health = 1.0;
            mob.fire_ticks = 20;
            mob.add_effect("minecraft:poison", 25, 0);
            mob.extra
                .set_main_hand_stack(Some(sunrise_client::items::ItemStack::new(
                    "minecraft:totem_of_undying",
                    1,
                )));
        });
        game.tick_creature_entities().unwrap();
        let mob = game.mobs.iter().find(|mob| mob.id == 71).unwrap();
        assert!(mob.alive());
        assert!(mob.extra.main_hand_stack().is_none());
        assert!(!mob.has_effect("minecraft:poison"));
        assert_eq!(
            mob.extra
                .effects
                .iter()
                .find(|e| e.id == "minecraft:regeneration")
                .map(|e| e.duration),
            Some(899),
            "the new regeneration, not the pre-hit effects, gets this tick"
        );
    });
}

struct TraderDrinkPost(Arc<Mutex<Vec<(bool, bool)>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for TraderDrinkPost {
    fn entity_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        if event.entity == 71 {
            let living = host.living(host.entity(71).unwrap()).unwrap();
            let invisibility = host.effect_key("minecraft:invisibility").unwrap();
            let invisible = living.effect(host, invisibility).is_some();
            let holding_drink = living
                .with_hand_item(host, mod_api::Hand::Main, |_| ())
                .is_some();
            self.0.lock().unwrap().push((invisible, holding_drink));
        }
    }
}

#[test]
#[ignore = "daily"]
fn trader_milk_finishes_before_its_own_entity_tick_post() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "trader-drink-post",
        Box::new(TraderDrinkPost(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.build_registries();
            game.world.day_time = 6000;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:wandering_trader")
                .unwrap()
                .clone();
            let mut trader = Mob::new(71, kind, game.mobs[0].position, 0.0);
            trader.no_gravity = true;
            trader.ground.on_ground = true;
            trader.add_effect("minecraft:invisibility", 3600, 0);
            game.mobs.remove(0);
            game.mobs.push(trader);
            // Start the actual daylight drink goal, whose first tick still precedes completion.
            game.tick_creature_entities().unwrap();
            let i = game.mobs.iter().position(|mob| mob.id == 71).unwrap();
            assert_eq!(
                game.mobs[i].extra.main_hand_id(),
                Some("minecraft:milk_bucket")
            );
            game.mobs.update(i, |mob| mob.extra.using_item = 32);
            game.tick_creature_entities().unwrap();
        });
    });
    assert_eq!(*seen.lock().unwrap(), [(true, true), (false, false)]);
}

/// A type a mod names is told at NeoForge's moment: each pig alone, its Post right after its
/// own tick and before the next pig's Pre, and a Post handle reads where its tick took it.
#[test]
#[ignore = "daily"]
fn a_named_type_hears_each_entitys_own_pre_and_post() {
    let heard = pig_ticks(false);
    let phases: Vec<_> = heard
        .iter()
        .map(|(phase, entities, _)| (*phase, entities.clone()))
        .collect();
    assert_eq!(
        phases,
        [
            ("pre", vec![71]),
            ("post", vec![71]),
            ("pre", vec![72]),
            ("post", vec![72]),
        ]
    );
    let first = heard[1].2.expect("the Post reads the first pig");
    assert!(first[1] < 80.0, "the first pig fell in its tick: {first:?}");
    assert_eq!(heard[3].2, Some([2.0, 80.0, 0.5]), "the second pig stayed");
}

#[test]
#[ignore = "daily"]
fn a_named_pre_cannot_clear_an_earlier_turns_cancellation() {
    let phases: Vec<_> = pig_ticks(true)
        .into_iter()
        .map(|(phase, entities, _)| (phase, entities))
        .collect();
    assert_eq!(
        phases,
        [("pre", vec![71]), ("pre", vec![72]), ("post", vec![72])]
    );
}

/// Any subscribed listener hears each mob's own Pre and Post without any opt-in flag.
#[test]
#[ignore = "daily"]
fn an_unnamed_family_hears_one_pre_and_one_post() {
    let phases: Vec<_> = pig_ticks(false)
        .into_iter()
        .map(|(phase, entities, _)| (phase, entities))
        .collect();
    assert_eq!(
        phases,
        [
            ("pre", vec![71]),
            ("post", vec![71]),
            ("pre", vec![72]),
            ("post", vec![72]),
        ]
    );
}

/// Two arrows on the server: each bracketed individually, its Post right after its own
/// flight and before the next arrow's Pre, and a Post handle reads where its flight took it.
#[test]
#[ignore = "daily"]
fn named_projectiles_hear_each_their_own_pre_and_post() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "projectile-tick-timing",
        Box::new(TickTiming {
            replace_cancels: false,
            heard: heard.clone(),
        }),
    )]
    .into();
    let mut ids = Vec::new();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            for z in [0.5, 2.5] {
                assert!(game.run_command(&format!(
                    "summon minecraft:arrow 1.5 80.5 {z} {{NoGravity:1b,Motion:[0.5d,0.0d,0.0d]}}"
                )));
            }
            ids = game.creatures.projectiles.iter().map(|p| p.id).collect();
            game.ticked_entities_through = u32::MAX;
            game.tick_projectiles().unwrap();
        });
    });
    let heard = heard.lock().unwrap();
    let phases: Vec<_> = heard
        .iter()
        .map(|(phase, entities, _)| (*phase, entities.clone()))
        .collect();
    assert_eq!(
        phases,
        [
            ("pre", vec![ids[0]]),
            ("post", vec![ids[0]]),
            ("pre", vec![ids[1]]),
            ("post", vec![ids[1]]),
        ]
    );
    let first = heard[1].2.expect("the Post reads the first arrow");
    assert!(
        first[0] > 1.5,
        "the first arrow flew in its own tick: {first:?}"
    );
}

/// Moves pig 72 into a second arrow's path in the Post of the arrow `first`.
struct MoveOnArrowPost {
    first: Arc<AtomicU32>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for MoveOnArrowPost {
    fn entity_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        if event.entity == self.first.load(Ordering::Relaxed) {
            let pig = host.entity(72).expect("the pig");
            assert!(pig.teleport_to(host, [4.0, 80.0, 5.5], None));
        }
    }
}

/// As on NeoForge, where each projectile ticks in turn: a mod that moves a mob in one arrow's
/// Post moves it for the next arrow of the same tick, which hits it there.
#[test]
#[ignore = "daily"]
fn a_mob_moved_in_one_arrows_post_is_hit_by_the_next_arrow() {
    let first = Arc::new(AtomicU32::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "projectile-post-move",
        Box::new(MoveOnArrowPost {
            first: first.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            // The first arrow far from both pigs; the second flies east along z = 5.5, where
            // no pig stands until the first arrow's Post moves one there.
            for command in [
                "summon minecraft:arrow 1.5 80.5 12.5 {NoGravity:1b,Motion:[0.1d,0.0d,0.0d]}",
                "summon minecraft:arrow 1.5 80.5 5.5 {NoGravity:1b,Motion:[3.0d,0.0d,0.0d]}",
            ] {
                assert!(game.run_command(command), "{command}");
            }
            first.store(game.creatures.projectiles[0].id, Ordering::Relaxed);
            let health = game.mobs.iter().find(|m| m.id == 72).unwrap().health;
            game.ticked_entities_through = u32::MAX;
            game.tick_projectiles().unwrap();
            let pig = game.mobs.iter().find(|m| m.id == 72).unwrap();
            assert_eq!(pig.position.z, 5.5, "the first arrow's Post moved the pig");
            assert!(
                pig.health < health,
                "the second arrow hit the pig where the Post moved it: {} of {health}",
                pig.health
            );
        });
    });
}

/// A server mod naming pigs and arrows with only `entity_tick_pre` or only `entity_tick_post`
/// yields the borrowed mob list and hands the projectile list back only for the subscribed phase.
#[test]
#[ignore = "daily"]
fn named_mobs_and_projectiles_yield_only_for_subscribed_phase() {
    type Recorded = Arc<Mutex<Vec<(&'static str, Vec<u32>, Option<[f64; 3]>)>>>;

    fn record(heard: &Recorded, host: &HostSide<'_>, phase: &'static str, entities: &[u32]) {
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        let pos = entities
            .first()
            .and_then(|&id| host.entity(id))
            .and_then(|entity| entity.position(*host));
        heard.lock().unwrap().push((phase, entities.to_vec(), pos));
    }

    struct PreOnly(Recorded);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PreOnly {
        fn entity_tick_pre(
            &self,
            host: HostSide<'_>,
            event: &mut EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            record(&self.0, &host, "pre", std::slice::from_ref(&event.entity));
            mod_api::Flow::Continue
        }
    }

    struct PostOnly(Recorded);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PostOnly {
        fn entity_tick_post(
            &self,
            host: HostSide<'_>,
            event: &mut mod_api::EntityTickPostEvent<'_>,
        ) {
            record(&self.0, &host, "post", std::slice::from_ref(&event.entity));
        }
    }

    for pre_only in [true, false] {
        let heard: Recorded = Arc::default();
        let entry = if pre_only {
            ModEntry::new("server-pre-only", Box::new(PreOnly(heard.clone())))
        } else {
            ModEntry::new("server-post-only", Box::new(PostOnly(heard.clone())))
        };
        let mods: mod_api::Mods = vec![entry].into();
        let mut arrow_id = 0;
        let mut mob_yields = 0;
        let mut projectile_handbacks = 0;
        mod_platform::testing::with_mods(mods, || {
            with_two_pigs(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                assert!(game.run_command(
                    "summon minecraft:arrow 1.5 80.5 5.5 {NoGravity:1b,Motion:[1.0d,0.0d,0.0d]}"
                ));
                arrow_id = game.creatures.projectiles[0].id;
                game.ticked_entities_through = u32::MAX;
                BORROWED_MOB_YIELDS.with(|count| count.set(0));
                super::super::PROJECTILE_HANDBACKS.with(|count| count.set(0));
                game.tick_mobs().unwrap();
                mob_yields = BORROWED_MOB_YIELDS.with(std::cell::Cell::get);
                projectile_handbacks =
                    super::super::PROJECTILE_HANDBACKS.with(std::cell::Cell::get);
            });
        });
        assert_eq!(
            mob_yields, 2,
            "one yield per named pig, none for the absent phase"
        );
        assert_eq!(
            projectile_handbacks, 1,
            "one handback for the named arrow, none for the absent phase"
        );
        let heard = heard.lock().unwrap();
        let phase = if pre_only { "pre" } else { "post" };
        let phases: Vec<_> = heard
            .iter()
            .map(|(p, entities, _)| (*p, entities.clone()))
            .collect();
        assert_eq!(
            phases,
            [
                (phase, vec![71]),
                (phase, vec![72]),
                (phase, vec![arrow_id]),
            ]
        );
        let arrow_pos = heard[2].2.expect("arrow handle resolves during its hook");
        if pre_only {
            assert_eq!(arrow_pos, [1.5, 80.5, 5.5]);
        } else {
            assert!(arrow_pos[0] > 1.5);
        }
    }
}

/// Mob `living_equipment_change` runs inside the mob's own tick before `entity_tick_post`, skips
/// mobs whose tick was canceled in `entity_tick_pre`, and prepares nothing when unsubscribed.
#[test]
#[ignore = "daily"]
fn mob_equipment_change_fires_before_post_and_skips_canceled_ticks() {
    use std::sync::atomic::AtomicBool;

    type Log = Arc<
        Mutex<
            Vec<(
                &'static str,
                u32,
                Option<mod_api::EquipmentSlot>,
                Option<mod_api::ItemStack>,
            )>,
        >,
    >;

    struct OrderProbe {
        cancel_72: Arc<AtomicBool>,
        log: Log,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for OrderProbe {
        fn entity_tick_pre(
            &self,
            _: HostSide<'_>,
            event: &mut EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            let id = event.entity;
            self.log.lock().unwrap().push(("pre", id, None, None));
            if id == 72 && self.cancel_72.load(Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
        fn living_equipment_change(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::LivingEquipmentChangeEvent<'_>,
        ) {
            self.log.lock().unwrap().push((
                "equip",
                event.entity.id(),
                Some(event.slot),
                event.to.as_ref().and_then(|stack| stack.project()),
            ));
        }
        fn entity_tick_post(&self, _: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
            self.log
                .lock()
                .unwrap()
                .push(("post", event.entity, None, None));
        }
    }

    let helmet = sunrise_client::items::ItemStack::new("minecraft:iron_helmet", 1);
    let cancel_72 = Arc::new(AtomicBool::new(true));
    let log: Log = Arc::default();
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-equip-order",
        Box::new(OrderProbe {
            cancel_72: cancel_72.clone(),
            log: log.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            let api_helmet = Some(helmet.try_into_api().unwrap());
            for mob in game.mobs.iter_mut() {
                assert!(
                    mob.set_equipment_stack(sunrise_items::equipment::HEAD, Some(helmet.clone()),)
                );
            }
            game.tick_mobs().unwrap();
            assert_eq!(
                std::mem::take(&mut *log.lock().unwrap()),
                [
                    ("pre", 71, None, None),
                    (
                        "equip",
                        71,
                        Some(mod_api::EquipmentSlot::Head),
                        api_helmet.clone()
                    ),
                    ("post", 71, None, None),
                    ("pre", 72, None, None),
                ]
            );
            assert!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 71)
                    .unwrap()
                    .extra
                    .live
                    .has_worn()
            );
            assert!(
                !game
                    .mobs
                    .iter()
                    .find(|m| m.id == 72)
                    .unwrap()
                    .extra
                    .live
                    .has_worn(),
                "a canceled mob tick must not prepare or fire living_equipment_change"
            );

            // Once pig 72 is allowed to tick, it fires its own equipment change before its Post,
            // while pig 71 does not re-fire an unchanged slot.
            cancel_72.store(false, Ordering::Relaxed);
            game.tick_mobs().unwrap();
            assert_eq!(
                std::mem::take(&mut *log.lock().unwrap()),
                [
                    ("pre", 71, None, None),
                    ("post", 71, None, None),
                    ("pre", 72, None, None),
                    (
                        "equip",
                        72,
                        Some(mod_api::EquipmentSlot::Head),
                        api_helmet.clone()
                    ),
                    ("post", 72, None, None),
                ]
            );
        });
    });

    // An unsubscribed tick still advances vanilla history without yielding to the owning game.
    mod_platform::testing::with_mods(mod_api::Mods::default(), || {
        with_two_pigs(|game| {
            assert!(
                game.mobs[0]
                    .set_equipment_stack(sunrise_items::equipment::HEAD, Some(helmet.clone()),)
            );
            BORROWED_MOB_YIELDS.with(|count| count.set(0));
            game.tick_mobs().unwrap();
            assert!(game.mobs[0].extra.live.has_worn());
            assert_eq!(BORROWED_MOB_YIELDS.with(std::cell::Cell::get), 0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_equipped_mob_discarded_by_despawn_does_not_report_equipment() {
    struct EquipmentProbe(Arc<Mutex<Vec<u32>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for EquipmentProbe {
        fn living_equipment_change(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::LivingEquipmentChangeEvent<'_>,
        ) {
            self.0.lock().unwrap().push(event.entity.id());
        }
    }

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![
        ModEntry::new(
            "mob-equip-despawn",
            Box::new(DespawnProbe {
                records: Arc::default(),
                allow: Arc::new(AtomicU32::new(71)),
            }),
        ),
        ModEntry::new("mob-equip-observe", Box::new(EquipmentProbe(heard.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            for mob in game.mobs.iter_mut() {
                assert!(mob.set_equipment_stack(
                    sunrise_items::equipment::HEAD,
                    Some(sunrise_client::items::ItemStack::new(
                        "minecraft:iron_helmet",
                        1
                    )),
                ));
            }
            game.tick_creature_entities().unwrap();
            assert_eq!(game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(), [72]);
            assert_eq!(*heard.lock().unwrap(), [72]);
        });
    });
}
