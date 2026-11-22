use crate::game::{
    Game,
    native_entity_refs::tests::{run_fresh_cases, with_game},
    native_guest_turn_tests::with_guests,
};
use mod_api::__private::{HookPanicReport, take_hook_panics};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use sunrise_client::{
    chunk::Chunk,
    mobs::{Mob, Who},
    motion::PlayerMotion,
};
use sunrise_core::geometry::Vec3;

const ZOMBIE: u32 = 71;
const TARGET_GUEST: u32 = 12;
const OTHER_GUEST: u32 = 13;

#[test]
#[ignore = "daily"]
fn pending_pushes_deliver_once_and_do_not_cross_world_reset() {
    if run_fresh_cases(
        "game::creatures::retained_limits_tests::pending_pushes_deliver_once_and_do_not_cross_world_reset",
        "SUNRISE_PENDING_MOB_PUSHES_CASE",
        &["delivery-and-reset"],
    ) {
        return;
    }

    with_game(|game| {
        const RECIPIENT: u32 = 72;
        const BYSTANDER: u32 = 73;
        let level = game.dim.id();
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mob = |id, x| Mob::new(id, kind.clone(), Vec3::new(x, 80.0, 0.5), 0.0);
        let velocity =
            |game: &Game<'_>, id| game.mobs.iter().find(|m| m.id == id).unwrap().velocity;

        // Only test delivery of already accepted pushes: skip ordinary mob travel in
        // the unloaded chunk so drag and new pushes cannot obscure the impulses.
        assert!(!game.world.loaded(64, 0));
        game.mobs.push(mob(BYSTANDER, 64.5));
        game.mobs.push(mob(RECIPIENT, 65.5));
        let pushes = &mut game.creatures.pending.entry(level).or_default().pushes;
        pushes.push(RECIPIENT, Vec3::new(0.25, 0.0, 0.0));
        pushes.push(RECIPIENT, Vec3::new(0.0, 0.0, -0.125));

        let expected = Vec3::new(0.25, 0.0, -0.125);
        for tick in ["delivery", "next tick"] {
            game.tick_creature_entities().unwrap();
            assert_eq!(
                velocity(game, RECIPIENT),
                expected,
                "{tick}: lost or replayed push"
            );
            assert_eq!(
                velocity(game, BYSTANDER),
                Vec3::ZERO,
                "{tick}: wrong recipient"
            );
        }

        game.creatures
            .pending
            .entry(level)
            .or_default()
            .pushes
            .push(RECIPIENT, Vec3::new(-0.125, 0.0, 0.0));
        game.clear_entities();
        game.mobs.push(mob(RECIPIENT, 66.5));
        game.tick_creature_entities().unwrap();
        assert_eq!(
            velocity(game, RECIPIENT),
            Vec3::ZERO,
            "a queued push crossed the world reset to a reused ID"
        );
    });
}

struct PanicInMovement(Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicInMovement {
    fn living_speed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingSpeedEvent<'_>,
    ) {
        if event.entity == ZOMBIE {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("zombie speed listener failed");
        }
    }
}

struct ObserveMovement(Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ObserveMovement {
    fn living_speed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingSpeedEvent<'_>,
    ) {
        if event.entity == ZOMBIE {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
#[ignore = "daily"]
fn same_mob_guest_attack_survives_movement_panic() {
    if run_fresh_cases(
        "game::creatures::retained_limits_tests::same_mob_guest_attack_survives_movement_panic",
        "SUNRISE_SAME_MOB_GUEST_ATTACK_CASE",
        &["movement-panic"],
    ) {
        return;
    }

    crate::entry::install_client_hooks();
    let faulty_calls = Arc::new(AtomicUsize::new(0));
    let healthy_calls = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("same-mob", Box::new(PanicInMovement(faulty_calls.clone()))),
            mod_api::ModEntry::new(
                "same-mob-observer",
                Box::new(ObserveMovement(healthy_calls.clone())),
            ),
        ]
        .into(),
        || {
            with_guests(|game| {
                game.world_root = PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join("same-mob-guest-attack");
                game.options.dedicated = Some(Vec::new());
                game.load_server_loot();
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Normal;
                game.world.game_time = 1000;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 79, z, stone).unwrap();
                    }
                }

                let far = Vec3::new(12.0, 80.0, 12.0);
                game.player = Some(PlayerMotion::new(game.motion, far).unwrap());
                let host = game.lan.as_mut().unwrap();
                for (conn, position) in [
                    (TARGET_GUEST, [1.5, 80.0, 0.5]),
                    (OTHER_GUEST, far.components()),
                ] {
                    host.hub
                        .update_guest(conn, |guest| {
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.position = position;
                        })
                        .unwrap();
                }
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(ZOMBIE, kind, Vec3::new(1.0, 80.0, 0.5), 0.0);
                mob.ground.on_ground = true;
                mob.ai.speed = 0.2;
                mob.ai.target = Some(Who::Player);
                game.mobs.push(mob);
                // Who::Player names the selected player, so bind it to the nearby guest.
                host.mob_players.insert(ZOMBIE, TARGET_GUEST);

                let health = |game: &Game<'_>| {
                    let hub = &game.lan.as_ref().unwrap().hub;
                    (
                        game.player.as_ref().unwrap().health.health(),
                        hub.guest(TARGET_GUEST).unwrap().health,
                        hub.guest(OTHER_GUEST).unwrap().health,
                    )
                };
                let (host_before, target_before, other_before) = health(game);
                game.tick_creature_entities().unwrap();
                let after = health(game);
                assert!(
                    after.1 < target_before,
                    "the zombie must damage its guest in the same tick"
                );
                assert_eq!(after.0, host_before, "the host was not targeted");
                assert_eq!(after.2, other_before, "the other guest was not targeted");
                assert_eq!(faulty_calls.load(Ordering::SeqCst), 1);
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "same-mob",
                        hook: "living_speed"
                    }],
                );
                let healthy_after_first = healthy_calls.load(Ordering::SeqCst);
                assert!(healthy_after_first > 0, "the other listener still runs");

                // Keep the zombie's grounded speed path active. Move the guest beyond
                // melee range without unloading the chunk and clear the hurt cooldown,
                // so unchanged health cannot be explained by invulnerability.
                game.mobs[0].ground.on_ground = true;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(TARGET_GUEST, |guest| {
                        guest.position = far.components();
                        guest.motion.as_mut().unwrap().health.invulnerable_time = 0;
                    })
                    .unwrap();
                game.tick_creature_entities().unwrap();
                assert!(healthy_calls.load(Ordering::SeqCst) > healthy_after_first);
                assert_eq!(faulty_calls.load(Ordering::SeqCst), 1);
                assert!(take_hook_panics().is_empty());
                assert_eq!(health(game), after, "the guest attack must not replay");
            });
        },
    );
}
