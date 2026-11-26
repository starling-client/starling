use super::*;
use crate::game::{
    native_entity_refs::tests::run_fresh_cases,
    native_projectiles::impact_probe::{Action, with_probe},
};
use mod_api::{Flow, Hooks, ProjectileImpactEvent, ServerHost};
use std::sync::{Arc, Mutex, atomic::Ordering};
use sunrise_client::{chunk::Chunk, join_seq::JoinSeq};
use sunrise_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "projectile_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

struct BoundProjectiles {
    _values: [Attachments; 2],
    index: Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>,
    registrations: [sunrise_mod_runtime::attachments::AttachmentRegistration; 2],
}

impl BoundProjectiles {
    fn bind_two(game: &mut Game<'_>) -> Self {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = [Attachments::default(), Attachments::default()];
        let registrations = std::array::from_fn(|i| {
            let id = game.creatures.projectiles[i].id;
            let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
            VALUE
                .set(&mut values[i], Holder::Entity(id), 10 + i as i32)
                .unwrap();
            game.creatures.projectiles.update(i, |p| {
                p.attachment_lifetime().bind(&mut values[i], &index, holder);
                p.attachment_lifetime().registration().unwrap()
            })
        });
        Self {
            _values: values,
            index,
            registrations,
        }
    }
}

#[derive(Clone, Default)]
struct Observed {
    impacts: Vec<u32>,
    spawned: bool,
    buffered: bool,
    remove: Option<u32>,
}

struct CancelImpacts(Arc<Mutex<Observed>>);

#[mod_api::__subscriptions]
impl Hooks for CancelImpacts {
    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        if event.event == "minecraft:projectile_shoot" {
            let mut observed = self.0.lock().unwrap();
            if !observed.buffered {
                // Shoot events fire after settlement while the tick batch is taken.
                if let Some(level) = host.level(event.dimension) {
                    let _ = level.summon(
                        host,
                        "minecraft:snowball",
                        [9.5, 70.0, 0.5],
                        Default::default(),
                    );
                }
                observed.buffered = true;
            }
        }
        Flow::Continue
    }

    fn projectile_impact(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ProjectileImpactEvent<'_>,
    ) -> Flow {
        let mut observed = self.0.lock().unwrap();
        observed.impacts.push(event.projectile);
        if !observed.spawned {
            // The projectile is installed even though summon has no projectile return target.
            if let Some(level) = host.level(event.dimension) {
                let _ = level.summon(
                    host,
                    "minecraft:snowball",
                    [8.5, 70.0, 0.5],
                    Default::default(),
                );
            }
            observed.spawned = true;
        }
        if observed.remove == Some(event.projectile) {
            let entity = host.entity(event.projectile).unwrap();
            assert!(entity.discard(host));
        }
        Flow::Cancel
    }
}

fn with_batch(remove: bool, test: impl FnOnce(&mut Game<'_>, [u32; 3], [JoinSeq; 3], u32, u32)) {
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "cancel-projectile-batch",
        Box::new(CancelImpacts(observed.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 64, 0, stone).unwrap();
            let ids = std::array::from_fn(|i| game.next_entity + i as u32 + 1);
            game.next_entity += 3;
            // Snowballs leave on impact; arrows stay in their slot, stuck in the wall.
            // The third projectile misses, so its survivor order must be preserved.
            for (i, kind) in [
                ProjectileKind::Snowball,
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                ProjectileKind::Snowball,
            ]
            .into_iter()
            .enumerate()
            {
                let x = if i == 2 { 5.5 } else { 0.5 };
                let mut p = Projectile::new(kind, Vec3::new(x, 64.5, 0.5), None);
                p.id = ids[i];
                p.velocity = Vec3::new(3.0, 0.0, 0.0);
                game.creatures.projectiles.push(p);
            }
            let joins = std::array::from_fn(|i| game.creatures.projectiles[i].join);
            let bound = BoundProjectiles::bind_two(game);
            let index = &bound.index;
            let key = ("test".into(), "projectile_lifetime".into());
            let before = index.lock().unwrap().snapshot(&key);
            assert_eq!(before.len(), 2);
            observed.lock().unwrap().remove = remove.then_some(ids[1]);
            game.audio.capture = Some(Vec::new());
            game.tick_projectiles().unwrap();
            assert!(
                game.audio
                    .capture
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|sound| { sound.event != "minecraft:entity.arrow.hit" })
            );
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 2);
            for (id, original) in ids[..2].iter().zip(&bound.registrations) {
                let i = game
                    .creatures
                    .projectiles
                    .iter()
                    .position(|p| p.id == *id)
                    .unwrap();
                let current = game
                    .creatures
                    .projectiles
                    .update(i, |p| p.attachment_lifetime().registration().unwrap());
                assert!(original.same_registration(&current));
            }
            let observed = observed.lock().unwrap().clone();
            assert_eq!(observed.impacts, [ids[0], ids[1]]);
            assert!(observed.spawned && observed.buffered);
            let spawned = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.position.x == 8.5)
                .unwrap()
                .id;
            let buffered = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.position.x == 9.5)
                .unwrap();
            assert_eq!(buffered.age, 0);
            let buffered = buffered.id;
            test(game, ids, joins, spawned, buffered);
            assert_eq!(
                index.lock().unwrap().snapshot(&key).len(),
                if remove { 1 } else { 2 }
            );
            game.creatures.projectiles.clear();
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert!(
                before
                    .iter()
                    .all(|row| matches!(row.entry.read(&VALUE), Ok(10 | 11)))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn no_impact_expiration_retires_only_the_expired_owner() {
    super::super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for (id, age, x) in [(71, 5999, 0.5), (72, 0, 5.5)] {
            let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(x, 70.0, 0.5), None);
            p.id = id;
            p.age = age;
            p.velocity = Vec3::new(0.25, 0.0, 0.0);
            game.creatures.projectiles.push(p);
        }
        let bound = BoundProjectiles::bind_two(game);
        let key = ("test".into(), "projectile_lifetime".into());
        let before = bound.index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 2);
        game.tick_projectiles().unwrap();
        assert_eq!(
            game.creatures
                .projectiles
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            [72]
        );
        assert_eq!(game.creatures.projectiles[0].age, 1);
        let current = game
            .creatures
            .projectiles
            .update(0, |p| p.attachment_lifetime().registration().unwrap());
        assert!(bound.registrations[1].same_registration(&current));
        let after = bound.index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].entry.read(&VALUE), Ok(&11));
        game.creatures.projectiles.clear();
        assert!(bound.index.lock().unwrap().snapshot(&key).is_empty());
        assert!(
            before
                .iter()
                .all(|row| matches!(row.entry.read(&VALUE), Ok(10 | 11)))
        );
    });
}

#[test]
#[ignore = "daily"]
fn canceled_impacts_keep_projectile_batch_joins_and_survivor_order() {
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::canceled_impacts_keep_projectile_batch_joins_and_survivor_order",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["cancel"],
    ) {
        return;
    }
    with_batch(false, |game, ids, joins, spawned, buffered| {
        let list = &mut game.creatures.projectiles;
        // The replayed snowball is appended; the arrow keeps its original slot.
        assert_eq!(
            list.iter().map(|p| p.id).collect::<Vec<_>>(),
            [buffered, ids[1], ids[2], spawned, ids[0]]
        );
        for (id, join) in ids.into_iter().zip(joins) {
            let p = list.iter().find(|p| p.id == id).unwrap();
            assert_eq!(p.join, join);
            assert_eq!(p.age, 1);
            assert!(p.position.x > 2.0);
            assert!(!p.stuck);
        }
        let snowball = list.iter().position(|p| p.id == ids[0]).unwrap();
        list.update(snowball, |p| {
            assert!(p.has_been_shot);
            assert!(p.take_shoot_event().is_none());
        });
        let fresh = list.iter().find(|p| p.id == spawned).unwrap();
        assert!(joins.into_iter().all(|join| join < fresh.join));
        assert_eq!(fresh.age, 0);
    });
}

#[test]
#[ignore = "daily"]
fn a_projectile_removed_in_its_impact_hook_stays_removed_after_replay() {
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::a_projectile_removed_in_its_impact_hook_stays_removed_after_replay",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["discard"],
    ) {
        return;
    }
    with_batch(true, |game, ids, joins, spawned, buffered| {
        assert!(game.entity(ids[1]).is_none());
        let discarded = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == ids[1])
            .unwrap();
        assert_eq!(
            discarded.position.x, 2.0,
            "discard must not replay the flight"
        );
        game.native_removals_flush();
        let list = &game.creatures.projectiles;
        assert_eq!(
            list.iter().map(|p| p.id).collect::<Vec<_>>(),
            [buffered, ids[2], spawned, ids[0]]
        );
        assert_eq!(list[1].join, joins[2]);
        assert_eq!(list[3].join, joins[0]);
        game.tick_projectiles().unwrap();
        assert!(!game.creatures.projectiles.iter().any(|p| p.id == ids[1]));
    });
}

fn terminal_egg(action: Action, abort: bool) {
    use sunrise_core::random::LegacyRandom;

    with_probe(action, true, |game, observed| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(2, 64, 0, stone).unwrap();
        let mut egg = Projectile::new(ProjectileKind::Egg, Vec3::new(0.5, 64.5, 0.5), None);
        egg.id = 71;
        egg.velocity = Vec3::new(3.0, 0.0, 0.0);
        game.creatures.projectiles.push(egg);
        let join = game.creatures.projectiles[0].join;
        let handle = game.entity(71).unwrap();
        game.native_joins_skip();
        game.audio.capture = Some(Vec::new());
        *game.world.random = LegacyRandom::new(4135);
        let random = game.world.random.state();
        let particles = game.particles.len();
        game.tick_projectiles().unwrap();
        game.native_leaves_flush();
        {
            let hits = observed.hits.lock().unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].projectile, 71);
            assert_eq!(hits[0].kind, "minecraft:egg");
            assert_eq!(hits[0].block, [2, 64, 0]);
        }
        assert_eq!(observed.lands.load(Ordering::Relaxed), 0);
        assert_eq!(observed.deaths.load(Ordering::Relaxed), 0);
        assert!(game.lan_entity_events.is_empty());
        assert_eq!(game.particles.len(), particles);
        assert_eq!(game.world.random.state(), random);
        assert!(game.mobs.is_empty());
        if abort {
            assert!(observed.leaves.lock().unwrap().is_empty());
            assert_eq!(game.entity(71), Some(handle));
            let egg = &game.creatures.projectiles[0];
            assert_eq!(egg.join, join);
            assert_eq!(egg.age, 1);
            assert!(egg.position.x > 2.0, "cancel alone flies through the block");
        } else {
            assert!(game.entity_position(handle).is_none());
            assert!(game.creatures.projectiles.is_empty());
            game.tick_projectiles().unwrap();
            game.native_leaves_flush();
            assert!(game.creatures.projectiles.is_empty());
            assert_eq!(*observed.leaves.lock().unwrap(), [71]);
        }
    });
}

#[test]
#[ignore = "daily"]
fn discarding_an_egg_during_impact_skips_the_hit_and_replay() {
    if !run_fresh_cases(
        "game::creatures::projectile_batch_tests::discarding_an_egg_during_impact_skips_the_hit_and_replay",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["egg-discard"],
    ) {
        terminal_egg(Action::DiscardAndCancel, false);
    }
}

#[test]
#[ignore = "daily"]
fn aborting_an_impact_discard_keeps_the_same_egg_in_flight() {
    if !run_fresh_cases(
        "game::creatures::projectile_batch_tests::aborting_an_impact_discard_keeps_the_same_egg_in_flight",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["egg-abort"],
    ) {
        terminal_egg(Action::AbortDiscard, true);
    }
}

#[test]
#[ignore = "daily"]
fn two_discarded_eggs_leave_once_without_dying() {
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::two_discarded_eggs_leave_once_without_dying",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["two-discards"],
    ) {
        return;
    }
    with_probe(Action::Discard, true, |game, observed| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(2, 64, 0, stone).unwrap();
        let handles = [71, 72].map(|id| {
            let mut egg = Projectile::new(ProjectileKind::Egg, Vec3::new(0.5, 64.5, 0.5), None);
            egg.id = id;
            egg.velocity = Vec3::new(3.0, 0.0, 0.0);
            game.creatures.projectiles.push(egg);
            game.entity(id).unwrap()
        });
        game.native_joins_skip();
        game.tick_projectiles().unwrap();
        assert!(
            handles
                .into_iter()
                .all(|egg| game.entity_position(egg).is_none())
        );
        assert_eq!(game.creatures.projectiles.len(), 2);
        game.native_leaves_flush();
        assert!(game.creatures.projectiles.is_empty());
        assert_eq!(*observed.leaves.lock().unwrap(), [71, 72]);
        assert_eq!(observed.deaths.load(Ordering::Relaxed), 0);
        assert_eq!(observed.lands.load(Ordering::Relaxed), 0);
    });
}

#[test]
#[ignore = "daily"]
fn an_arrow_does_not_stick_when_the_impact_listener_clears_its_block() {
    const CASE: &str = "SUNRISE_PROJECTILE_BATCH_CASE";
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::an_arrow_does_not_stick_when_the_impact_listener_clears_its_block",
        CASE,
        &["clear", "allow"],
    ) {
        return;
    }
    let clear = std::env::var(CASE).as_deref() == Ok("clear");
    let action = if clear {
        Action::ClearBlockOnImpact
    } else {
        Action::Allow
    };
    with_probe(action, true, |game, observed| {
        game.audio.capture = Some(Vec::new());
        shoot_arrow_at_wall(game, 2, 0.5);
        let hits = observed.hits.lock().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].at, [2.0, 64.5, 0.5]);
        assert_eq!(hits[0].projectile_pos, hits[0].at);
        let p = &game.creatures.projectiles[0];
        let arrow = p.arrow.as_ref().unwrap();
        if clear {
            assert_eq!(
                p.position.components(),
                hits[0].at,
                "refusal must not replay the flight"
            );
            assert_eq!(p.velocity, Vec3::new(3.0 * f64::from(0.99_f32), -0.05, 0.0));
            assert_eq!(arrow.shake, 0);
        } else {
            assert_eq!(p.position.x, 2.0 - f64::from(0.05_f32));
            assert_eq!(p.velocity, Vec3::ZERO);
            assert_eq!(arrow.shake, 7);
        }
        assert_eq!(arrow.in_ground, !clear);
        assert_eq!(observed.lands.load(Ordering::Relaxed), usize::from(!clear));
        assert_eq!(
            game.audio
                .capture
                .as_ref()
                .unwrap()
                .iter()
                .any(|sound| { sound.event == "minecraft:entity.arrow.hit" }),
            !clear
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_first_shot_listener_can_clear_the_eggs_captured_block_before_landing() {
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::a_first_shot_listener_can_clear_the_eggs_captured_block_before_landing",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["clear-on-shoot"],
    ) {
        return;
    }
    with_probe(
        Action::ClearBlockOnShoot([2, 64, 0]),
        true,
        |game, observed| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 64, 0, stone).unwrap();
            let mut egg = Projectile::new(ProjectileKind::Egg, Vec3::new(0.5, 64.5, 0.5), None);
            egg.id = 71;
            egg.velocity = Vec3::new(3.0, 0.0, 0.0);
            game.creatures.projectiles.push(egg);
            let handle = game.entity(71).unwrap();
            let join = game.creatures.projectiles[0].join;
            game.tick_projectiles().unwrap();
            let hits = observed.hits.lock().unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].block, [2, 64, 0]);
            assert_ne!(game.world.block(2, 64, 0), stone);
            assert_eq!(observed.lands.load(Ordering::Relaxed), 0);
            assert!(game.lan_entity_events.is_empty());
            assert_eq!(game.entity(71), Some(handle));
            assert_eq!(game.creatures.projectiles[0].join, join);
            assert_eq!(
                game.creatures.projectiles[0].position.components(),
                hits[0].at
            );
        },
    );
}

fn shoot_arrow_at_wall(game: &mut Game<'_>, wall_x: i32, start_x: f64) {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(wall_x, 64, 0, stone).unwrap();
    let mut arrow = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::new(start_x, 64.5, 0.5),
        None,
    );
    arrow.id = 71;
    arrow.velocity = Vec3::new(3.0, 0.0, 0.0);
    game.creatures.projectiles.push(arrow);
    game.native_joins_skip();
    game.tick_projectiles().unwrap();
}

#[test]
#[ignore = "daily"]
fn an_arrow_lands_before_its_first_shot_event_at_the_backed_off_position() {
    const CASE: &str = "SUNRISE_PROJECTILE_BATCH_CASE";
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::an_arrow_lands_before_its_first_shot_event_at_the_backed_off_position",
        CASE,
        &["allow", "no-impacts"],
    ) {
        return;
    }
    let hears = std::env::var(CASE).as_deref() == Ok("allow");
    with_probe(Action::Allow, hears, |game, observed| {
        shoot_arrow_at_wall(game, 2, 0.5);
        assert_eq!(observed.hits.lock().unwrap().len(), usize::from(hears));
        assert_eq!(
            *observed.game_events.lock().unwrap(),
            [
                ("land", [2.5, 64.5, 0.5]),
                ("shoot", [2.0 - f64::from(0.05_f32), 64.5, 0.5])
            ],
        );
    });
}

#[test]
#[ignore = "daily"]
fn an_arrow_files_both_section_crossings_at_a_section_edge_wall() {
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::an_arrow_files_both_section_crossings_at_a_section_edge_wall",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["wall-at-section-edge"],
    ) {
        return;
    }
    with_probe(Action::Allow, true, |game, observed| {
        game.world
            .chunks
            .insert((1, 0), Arc::new(Chunk::empty(1, 0)));
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(1, 0))
            .unwrap();
        shoot_arrow_at_wall(game, 16, 14.5);
        assert_eq!(observed.hits.lock().unwrap()[0].at, [16.0, 64.5, 0.5]);
        assert_eq!(
            *observed.sections.lock().unwrap(),
            [(71, [0, 4, 0], [1, 4, 0]), (71, [1, 4, 0], [0, 4, 0])],
        );
        assert_eq!(
            game.creatures.projectiles[0].position.x,
            16.0 - f64::from(0.05_f32)
        );
    });
}

#[test]
#[ignore = "daily"]
fn wind_charges_retrace_only_after_gameplay_breaks_a_vehicle() {
    const CASE: &str = "SUNRISE_PROJECTILE_BATCH_CASE";
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::wind_charges_retrace_only_after_gameplay_breaks_a_vehicle",
        CASE,
        &[
            "allow",
            "no-impacts",
            "cancel-second",
            "miss",
            "discard-target",
        ],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let action = match case.as_str() {
        "discard-target" => Action::DiscardTarget(71),
        "cancel-second" => Action::CancelTarget(72),
        _ => Action::Allow,
    };
    let (charges, boats, drops, survivors, bursts, targets): (_, _, _, _, _, &[u32]) =
        match case.as_str() {
            "miss" => (6, 0, 1, 1, 5, &[71; 5]),
            "discard-target" => (2, 1, 0, 2, 0, &[71]),
            "cancel-second" => (10, 1, 1, 5, 5, &[71, 71, 71, 71, 71, 72, 72, 72, 72, 72]),
            "no-impacts" => (10, 0, 2, 0, 10, &[]),
            _ => (10, 0, 2, 0, 10, &[71, 71, 71, 71, 71, 72, 72, 72, 72, 72]),
        };
    with_probe(action, case != "no-impacts", |game, observed| {
        // Five hits break a boat, as in the vehicle parity harness. Stagger the pair so
        // the reflight must update both the target and the contact coordinates.
        for (id, y) in [(71, 64.0), (72, 63.875)]
            .into_iter()
            .take(if case == "miss" { 1 } else { 2 })
        {
            game.boats.push(crate::game::boats::Boat::new(
                id,
                Arc::from("oak"),
                Vec3::new(8.5, y, 0.5),
                0.0,
            ));
        }
        for i in 0..charges {
            let mut p = Projectile::new(
                ProjectileKind::BreezeWindCharge,
                Vec3::new(8.5, 64.8, 0.5),
                None,
            );
            p.id = 80 + i;
            p.velocity = Vec3::new(0.0, -0.5, 0.0);
            p.acceleration = 0.0;
            game.creatures.projectiles.push(p);
        }
        game.native_joins_skip();
        let joins = game
            .creatures
            .projectiles
            .iter()
            .map(|p| p.join)
            .collect::<Vec<_>>();
        game.tick_projectiles().unwrap();
        game.native_removals_flush();
        let hits = observed.hits.lock().unwrap();
        assert_eq!(hits.len(), targets.len());
        for (i, (hit, target)) in hits.iter().zip(targets).enumerate() {
            assert_eq!(hit.projectile, 80 + i as u32);
            assert_eq!(hit.entity, Some(*target));
            assert_eq!(hit.entity_kind, "minecraft:oak_boat");
            assert_eq!(hit.projectile_pos, hit.at);
        }
        if hits.len() == 10 {
            assert!(hits[5].at[1] < hits[0].at[1], "the second boat is lower");
        }
        assert_eq!(observed.bursts.load(Ordering::Relaxed), bursts);
        assert_eq!(game.boats.len(), boats);
        if boats == 1 {
            assert_eq!(game.boats[0].id, 72);
            assert_eq!(game.boats[0].damage, 0.0);
        }
        assert_eq!(
            game.dropped
                .iter()
                .filter(|drop| &*drop.stack.item == "minecraft:oak_boat")
                .map(|drop| drop.stack.count)
                .sum::<u32>(),
            drops
        );
        assert_eq!(game.creatures.projectiles.len(), survivors);
        let at = if case == "discard-target" {
            hits[0].at
        } else {
            [8.5, 64.8 - 0.5, 0.5]
        };
        for charge in game.creatures.projectiles.iter() {
            assert_eq!(charge.join, joins[(charge.id - 80) as usize]);
            assert_eq!(charge.age, 1);
            assert_eq!(charge.position.components(), at);
        }
    });
}

/// A subscriber that was off when the tick began and is turned on by a section callback during
/// it hears the flights that follow: the live loop asks `native_hears_impacts` at each flight,
/// not once for the family.
#[test]
#[ignore = "daily"]
fn a_subscriber_enabled_by_a_section_callback_hears_the_flights_after_it() {
    struct Counter(Arc<Mutex<Vec<u32>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Counter {
        fn projectile_impact(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ProjectileImpactEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(event.projectile);
            Flow::Continue
        }
    }
    struct Trigger(Arc<std::sync::atomic::AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Trigger {
        fn entity_entering_section(
            &self,
            _: mod_api::HostSide<'_>,
            _: &mut mod_api::EntityEnteringSectionEvent,
        ) {
            self.0.fetch_add(1, Ordering::SeqCst);
            mod_api::__private::set_mod_enabled("reenabled-counter", true);
        }
    }
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::a_subscriber_enabled_by_a_section_callback_hears_the_flights_after_it",
        "SUNRISE_PROJECTILE_REENABLE_CASE",
        &["off"],
    ) {
        return;
    }
    let crossings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let heard: Arc<Mutex<Vec<u32>>> = Arc::default();
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("reenabled-counter", Box::new(Counter(heard.clone()))),
        mod_api::ModEntry::new("section-trigger", Box::new(Trigger(crossings.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for chunk in [(0, 0), (1, 0)] {
                game.world
                    .chunks
                    .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
                game.block_ticks
                    .add_empty_chunk(sunrise_core::position::ChunkPos::new(chunk.0, chunk.1))
                    .unwrap();
            }
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("forceload add 16 0"));
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 8.5))
                    .unwrap(),
            );
            crate::game::native_entity_refs::tests::bind_local_player(game, 7, 7);
            game.next_entity = 100;
            assert!(mod_api::__private::set_mod_enabled(
                "reenabled-counter",
                false
            ));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            // Arrow 71 flies across into the next section (the trigger's callback turns the counter
            // on); arrow 72 lands on a wall in the first section.
            game.world.set(2, 64, 0, stone).unwrap();
            for (id, start) in [(71, 14.5), (72, 0.5)] {
                let mut arrow = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    Vec3::new(start, 64.5, 0.5),
                    None,
                );
                arrow.id = id;
                arrow.velocity = Vec3::new(3.0, 0.0, 0.0);
                game.creatures.projectiles.push(arrow);
            }
            game.native_joins_skip();
            game.tick_projectiles().unwrap();
        });
    });
    assert!(
        crossings.load(Ordering::SeqCst) > 0,
        "a section callback ran"
    );
    // The counter was off when the tick began and the callback switched it on: it hears the
    // flight after the callback, and only that one.
    assert_eq!(*heard.lock().unwrap(), [72]);
}

#[derive(Default)]
struct WindContinuationObserved {
    launches: Mutex<Vec<u32>>,
    alone_ticks: Mutex<Vec<u32>>,
    buffered: Mutex<bool>,
}

struct WindContinuationObserver {
    spawn_on_shoot: bool,
    observed: Arc<WindContinuationObserved>,
}

#[mod_api::__subscriptions]
impl Hooks for WindContinuationObserver {
    fn entity_tick_post(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        self.observed.alone_ticks.lock().unwrap().push(event.entity);
    }

    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::EntityJoinLevelEvent<'_>,
    ) -> Flow {
        if host.projectile(event.subject).is_some() {
            self.observed
                .launches
                .lock()
                .unwrap()
                .push(event.subject.id());
        }
        Flow::Continue
    }

    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        if self.spawn_on_shoot && event.event == "minecraft:projectile_shoot" {
            let mut buffered = self.observed.buffered.lock().unwrap();
            if !*buffered {
                // Spawn into the lifted buffer so SpawnsFirst restores it ahead of survivors.
                if let Some(level) = host.level(event.dimension) {
                    let _ = level.summon(
                        host,
                        "minecraft:snowball",
                        [12.5, -55.0, 8.5],
                        Default::default(),
                    );
                }
                *buffered = true;
            }
        }
        Flow::Continue
    }
}

fn load_wind_continuation_witness_26_3() -> String {
    let raw = std::env::var_os("SUNRISE_WIND_CONTINUATION_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_WIND_CONTINUATION_WITNESS is unset; run tools/test_bounced_projectile_tick_order_parity.py"
        )
    });
    let path = std::path::PathBuf::from(raw);
    let path = if path.is_dir() {
        path.join("wind_charge_continuation_tick_order.txt")
    } else {
        path
    };
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (run tools/test_bounced_projectile_tick_order_parity.py): {e}",
            path.display()
        )
    })
}

fn wind_continuation_witness_row_26_3<'a>(witness: &'a str, tick: usize, tag: &str) -> &'a str {
    let prefix = format!("tick {tick} {tag} ");
    let line = witness
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("missing 26.3 witness row for {prefix}"));
    &line[prefix.len()..]
}

fn wind_continuation_witness_26_3(
    witness: &str,
    tick: usize,
    tag: &str,
) -> Option<([f64; 3], [f64; 3])> {
    let rest = wind_continuation_witness_row_26_3(witness, tick, tag);
    if rest == "gone" {
        return None;
    }
    let rest = rest.strip_prefix("pos ").expect("pos prefix");
    let (pos_str, mot_str) = rest.split_once(" motion ").expect("motion separator");
    let parse3 = |s: &str| {
        let mut it = s
            .split_whitespace()
            .map(|n| n.parse::<f64>().expect("witness float"));
        [it.next().unwrap(), it.next().unwrap(), it.next().unwrap()]
    };
    Some((parse3(pos_str), parse3(mot_str)))
}

fn wind_continuation_boat_witness_26_3(witness: &str, tick: usize, tag: &str) -> Option<f32> {
    let rest = wind_continuation_witness_row_26_3(witness, tick, tag);
    if rest == "gone" {
        return None;
    }
    let damage = rest
        .strip_prefix("alive damage ")
        .expect("alive damage prefix")
        .parse::<f32>()
        .expect("witness boat damage float");
    Some(damage)
}

fn assert_boat_matches_witness(witness: &str, game: &Game<'_>, tick: usize, tag: &str) {
    let actual = game.boats.iter().find(|b| b.id == 71).map(|b| b.damage);
    let expected = wind_continuation_boat_witness_26_3(witness, tick, tag);
    assert_eq!(
        actual.map(f32::to_bits),
        expected.map(f32::to_bits),
        "tick {tick} {tag}: boat state got {actual:?} expected {expected:?}"
    );
    assert_eq!(
        game.boats.len(),
        usize::from(expected.is_some()),
        "tick {tick} {tag}: boat count mismatch"
    );
}

fn assert_vec3_bits(actual: Vec3, expected: [f64; 3], label: &str) {
    for (axis, (got, want)) in actual.components().into_iter().zip(expected).enumerate() {
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "{label} axis {axis}: got {got} expected {want}"
        );
    }
}

#[test]
#[ignore = "run by tools/test_bounced_projectile_tick_order_parity.py"]
fn continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass() {
    run_continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass();
}

pub(in crate::game) fn run_continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass()
 {
    let witness = load_wind_continuation_witness_26_3();
    const CASE: &str = "SUNRISE_PROJECTILE_BATCH_CASE";
    if run_fresh_cases(
        "game::creatures::projectile_batch_tests::continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass",
        CASE,
        &["main", "ctrl4", "reversed", "mixed-batch"],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let observed = Arc::new(WindContinuationObserved::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "wind-continuation-observer",
        Box::new(WindContinuationObserver {
            spawn_on_shoot: case == "mixed-batch",
            observed: observed.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for cz in 0..=2 {
                game.world
                    .chunks
                    .insert((0, cz), Arc::new(Chunk::empty(0, cz)));
                game.block_ticks
                    .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, cz))
                    .unwrap();
            }
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0 15 47"));
            game.next_entity = 100;
            let (z, breakers, boat_tag, w6_tag, a2_tag) = match case.as_str() {
                "ctrl4" => (24.5, 4_u32, "boat_ctrl4", "w6_ctrl4", None),
                "reversed" => (40.5, 5_u32, "boat_rev", "w6_rev", Some("a2_rev")),
                _ => (8.5, 5_u32, "boat_main", "w6_main", Some("a2_main")),
            };
            game.boats.push(crate::game::boats::Boat::new(
                71,
                Arc::from("oak"),
                Vec3::new(4.5, -60.0, z),
                0.0,
            ));
            for i in 0..breakers {
                let mut p =
                    Projectile::new(ProjectileKind::WindCharge, Vec3::new(4.5, -59.2, z), None);
                p.id = 80 + i;
                p.velocity = Vec3::new(0.0, -0.5, 0.0);
                p.acceleration = 0.0;
                game.creatures.projectiles.push(p);
            }
            let make_w6 = || {
                let mut w6 = Projectile::new(
                    ProjectileKind::BreezeWindCharge,
                    Vec3::new(3.5, -59.7, z),
                    None,
                );
                w6.id = 85;
                w6.velocity = Vec3::new(1.5, 0.0, 0.0);
                w6.acceleration = 0.0;
                w6
            };
            let make_a2 = || {
                let mut a2 = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    Vec3::new(8.0, -58.6, z),
                    None,
                );
                a2.id = 86;
                a2.velocity = Vec3::new(0.0, -0.2, 0.0);
                a2
            };
            let (w6_join, a2_join) = if case == "reversed" {
                game.creatures.projectiles.push(make_a2());
                let a2_join = game.creatures.projectiles.last().unwrap().join;
                game.creatures.projectiles.push(make_w6());
                let w6_join = game.creatures.projectiles.last().unwrap().join;
                (w6_join, Some(a2_join))
            } else {
                game.creatures.projectiles.push(make_w6());
                let w6_join = game.creatures.projectiles.last().unwrap().join;
                let a2_join = a2_tag.map(|_| {
                    game.creatures.projectiles.push(make_a2());
                    game.creatures.projectiles.last().unwrap().join
                });
                (w6_join, a2_join)
            };
            game.native_joins_skip();
            assert_boat_matches_witness(&witness, game, 0, boat_tag);
            game.ticked_entities_through = u32::MAX;
            game.tick_projectiles().unwrap();
            if case != "reversed" {
                assert!(
                    observed.alone_ticks.lock().unwrap().contains(&85),
                    "w6 must fire entity_tick_post on its own tick"
                );
            }
            if case == "ctrl4" {
                assert!(wind_continuation_witness_26_3(&witness, 1, w6_tag).is_none());
                assert!(game.creatures.projectiles.is_empty());
                assert_boat_matches_witness(&witness, game, 1, boat_tag);
                for tick in 2..=4 {
                    game.ticked_entities_through = u32::MAX;
                    game.tick_projectiles().unwrap();
                    assert!(wind_continuation_witness_26_3(&witness, tick, w6_tag).is_none());
                    assert!(game.creatures.projectiles.is_empty());
                    assert_boat_matches_witness(&witness, game, tick, boat_tag);
                }
                assert!(observed.launches.lock().unwrap().is_empty());
                return;
            }
            let spawned = (case == "mixed-batch").then(|| {
                assert!(*observed.buffered.lock().unwrap());
                game.creatures
                    .projectiles
                    .iter()
                    .find(|p| p.position.x == 12.5)
                    .expect("nested snowball restored ahead of survivors")
                    .id
            });
            let expected_ids: Vec<u32> = match (case.as_str(), spawned) {
                ("reversed", None) => vec![86, 85],
                (_, Some(id)) => vec![id, 85, 86],
                (_, None) => vec![85, 86],
            };
            let actual_ids: Vec<u32> = game.creatures.projectiles.iter().map(|p| p.id).collect();
            assert_eq!(actual_ids, expected_ids);
            let expected_launches: Vec<u32> = spawned.into_iter().collect();
            assert_eq!(*observed.launches.lock().unwrap(), expected_launches);
            let w6_live = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == 85)
                .unwrap();
            let a2_live = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == 86)
                .unwrap();
            assert_eq!(w6_live.join, w6_join);
            assert_eq!(Some(a2_live.join), a2_join);
            let a2_tag = a2_tag.unwrap();
            let check_tick = |game: &Game<'_>, tick: usize| {
                assert_boat_matches_witness(&witness, game, tick, boat_tag);
                let (w6_pos, w6_mot) =
                    wind_continuation_witness_26_3(&witness, tick, w6_tag).expect("w6 alive");
                let (a2_pos, a2_mot) =
                    wind_continuation_witness_26_3(&witness, tick, a2_tag).expect("a2 alive");
                let w6 = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == 85)
                    .unwrap();
                let a2 = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == 86)
                    .unwrap();
                assert_vec3_bits(w6.position, w6_pos, "w6 pos");
                assert_vec3_bits(w6.velocity, w6_mot, "w6 motion");
                assert_vec3_bits(a2.position, a2_pos, "a2 pos");
                assert_vec3_bits(a2.velocity, a2_mot, "a2 motion");
            };
            check_tick(game, 1);
            for tick in 2..=4 {
                game.ticked_entities_through = u32::MAX;
                game.tick_projectiles().unwrap();
                check_tick(game, tick);
            }
            assert_eq!(*observed.launches.lock().unwrap(), expected_launches);
        });
    });
}
