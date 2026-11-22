//! Exercise the removal boundary through the live server, including the next actor's tick.
use super::{Game, commands::Target, native_entity_refs::tests::with_game};
use mod_api::{Flow, Hooks, HostSide, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sunrise_client::{
    entities::{ArmorStand, Cushion},
    mobs::Mob,
};
use sunrise_core::{geometry::Vec3, position::ChunkPos};

const AT: Vec3 = Vec3::new(4.5, 64.0, 4.5);

fn push_pig(game: &mut Game<'_>, id: u32) {
    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    game.mobs.push(Mob::new(id, kind, AT, 0.0));
}

fn tick_area(game: &mut Game<'_>) {
    super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
        game,
        ChunkPos::new(0, 0),
    );
    game.build_registries();
    assert!(game.as_console(|g| g.run_command("forceload add 0 0")));
    game.sync_tick_chunks();
    assert!(game.entity_ticking_area()(4, 4));
}

#[test]
#[ignore = "daily"]
fn nested_blast_skips_the_stand_or_cushion_already_being_removed() {
    use super::hook_meddler_tests::with_meddler;
    for stand in [true, false] {
        with_meddler(
            |host| {
                let level = host.level("minecraft:overworld").unwrap();
                level
                    .explode(
                        host,
                        None,
                        AT.components(),
                        1.0,
                        false,
                        mod_api::ExplosionInteraction::None,
                        Default::default(),
                    )
                    .unwrap();
            },
            |game, gate| {
                tick_area(game);
                game.next_entity = 100;
                push_pig(game, 72);
                game.mob_object_mut(72).unwrap().extra.mount = Some(71);
                if stand {
                    game.armor_stands.push(ArmorStand::new(71, AT, 0.0));
                } else {
                    game.cushions.push(Cushion {
                        attachment_lifetime: Default::default(),
                        join: Default::default(),
                        id: 71,
                        position: AT,
                        pos: [4, 64, 4],
                        yaw: 0.0,
                        color: 0,
                        check_interval: 0,
                        custom_name: None,
                    });
                }
                gate.arm();
                if stand {
                    assert!(game.break_armor_stand_by_anything(0, "minecraft:in_fire", 0));
                } else {
                    assert!(game.break_cushion_by(0, None));
                }
                assert!(gate.fired());
                assert!(game.armor_stands.is_empty() && game.cushions.is_empty());
                assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
            },
        );
    }
}

#[test]
#[ignore = "daily"]
fn stand_equipment_and_below_world_spills_are_visible_at_dismount() {
    use super::{
        boats::Boat,
        hook_meddler_tests::with_meddler,
        minecarts::{CartKind, Minecart},
    };
    use sunrise_client::items::ItemStack;

    for case in ["stand", "boat", "cart"] {
        with_meddler(
            move |host| {
                assert!(
                    host.entity(71).is_none(),
                    "the vehicle is logically removed"
                );
                for id in 101..=if case == "boat" { 102 } else { 101 } {
                    assert!(
                        host.entity(id).is_some(),
                        "{case} must drop before dismount"
                    );
                }
            },
            |game, gate| {
                game.next_entity = 100;
                let below = Vec3::new(AT.x, f64::from(game.world.shape.min_y - 65), AT.z);
                let stack = ItemStack::new("minecraft:diamond", 1);
                push_pig(game, 72);
                game.mob_object_mut(72).unwrap().extra.mount = Some(71);
                match case {
                    "stand" => {
                        let mut stand = ArmorStand::new(71, AT, 0.0);
                        stand.equipment[0] = Some(stack);
                        game.armor_stands.push(stand);
                    }
                    "boat" => {
                        let mut boat = Boat::new(71, "oak".into(), below, 0.0).with_chest();
                        boat.items[0] = Some(stack);
                        boat.leash = Some(sunrise_client::mobs::leash::Holder::Mob(300));
                        game.boats.push(boat);
                    }
                    _ => {
                        let mut cart = Minecart::new(71, CartKind::Chest, below);
                        cart.items[0] = Some(stack);
                        game.carts.push(cart);
                    }
                }
                gate.arm();
                match case {
                    "stand" => {
                        assert!(game.break_armor_stand_by_anything(0, "minecraft:in_fire", 0))
                    }
                    "boat" => assert!(game.boat_below_world(0)),
                    _ => {
                        let round = game.native_tick_round();
                        assert_eq!(
                            game.cart_in_world(71, 0, &round, "minecraft:chest_minecart"),
                            None
                        );
                    }
                }
                assert!(gate.fired(), "{case}");
                assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                assert_eq!(game.dropped[0].stack.item.as_ref(), "minecraft:diamond");
                if case == "boat" {
                    assert_eq!(game.dropped[1].stack.item.as_ref(), "minecraft:lead");
                }
            },
        );
    }
}

#[derive(Default)]
struct Observation {
    veto_dismount: AtomicBool,
    veto_rider_tick: AtomicBool,
    trace: Mutex<Vec<String>>,
}

struct Observe(Arc<Observation>);

#[mod_api::__subscriptions]
impl Hooks for Observe {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.entity_tick_each("minecraft:pig");
    }

    fn entity_mount(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::EntityMountEvent<'_>,
    ) -> Flow {
        if event.mounting {
            return Flow::Continue;
        }
        let removed = host
            .entity(if event.entity == 71 {
                event.entity
            } else {
                event.vehicle
            })
            .is_none();
        self.0
            .trace
            .lock()
            .unwrap()
            .push(format!("{}:removed={removed}", event.entity));
        if event.entity == 72 && self.0.veto_dismount.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn entity_tick_listened(&self, _: HostSide<'_>, event: &mut mod_api::EntityTickListenedEvent) {
        event.listened = true;
    }

    fn entity_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::EntityTickPreEvent<'_>) {
        if event.entities.contains(&72) && self.0.veto_rider_tick.load(Ordering::Relaxed) {
            event.canceled.push(72);
        }
        if event.entities.contains(&74)
            && let HostSide::Server(host) = host
        {
            let riding = host
                .entity(72)
                .and_then(|rider| rider.is_passenger(host))
                .unwrap();
            self.0
                .trace
                .lock()
                .unwrap()
                .push(format!("later:riding={riding}"));
        }
    }
}

fn with_removal_test_hooks(hooks: impl Hooks, test: impl FnOnce(&mut Game<'_>)) {
    let entry = mod_api::ModEntry::new("removal-test", Box::new(hooks));
    mod_platform::testing::with_mods(Arc::from([entry]), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            tick_area(game);
            game.next_entity = 100;
            test(game);
        });
    });
}

fn finish_death_on_next_tick(game: &mut Game<'_>, id: u32) {
    let mob = game.mob_object_mut(id).unwrap();
    mob.health = 0.0;
    mob.death_time = 19;
    game.died.insert(id);
    game.tick_creature_entities().unwrap();
}

#[test]
#[ignore = "daily"]
fn mob_removal_keeps_a_veto_until_the_next_eligible_pre_with_or_without_off_level_objects() {
    for off_level in [false, true] {
        let observation = Arc::new(Observation::default());
        observation.veto_dismount.store(true, Ordering::Relaxed);
        observation.veto_rider_tick.store(true, Ordering::Relaxed);
        with_removal_test_hooks(Observe(observation.clone()), |game| {
            push_pig(game, 71);
            push_pig(game, 72);
            assert!(game.try_mount_mob(72, 71));
            if off_level {
                push_pig(game, 73);
                let mut rejected = Mob::new(75, game.mobs[2].kind.clone(), AT, 0.0);
                rejected.extra.mount = Some(73);
                game.persist
                    .off_level
                    .insert(75, super::persist::OffLevel::Mob(rejected));
            }
            finish_death_on_next_tick(game, 71);
            assert!(game.entity(71).is_none());
            assert!(game.persist.removed_vehicles.contains_key(&71));
            assert!(!game.persist.off_level.contains_key(&71));
            assert_eq!(game.persist.off_level.contains_key(&75), off_level);
            assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
            assert_eq!(*observation.trace.lock().unwrap(), ["72:removed=true"]);

            observation.veto_dismount.store(false, Ordering::Relaxed);
            game.tick_creature_entities().unwrap();
            assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
            assert_eq!(*observation.trace.lock().unwrap(), ["72:removed=true"]);

            observation.veto_rider_tick.store(false, Ordering::Relaxed);
            game.tick_creature_entities().unwrap();
            assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
            assert!(!game.persist.removed_vehicles.contains_key(&71));
            assert_eq!(
                *observation.trace.lock().unwrap(),
                ["72:removed=true", "72:removed=true"]
            );
        });
    }
}

#[test]
#[ignore = "daily"]
fn wind_charged_mob_removal_cannot_reenter_through_its_own_burst() {
    #[derive(Default)]
    struct Events {
        bursts: AtomicUsize,
        leaves: AtomicUsize,
    }

    struct Reenter(Arc<Events>);
    #[mod_api::__subscriptions]
    impl Hooks for Reenter {
        fn explosion_start(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::ExplosionStartEvent<'_>,
        ) -> Flow {
            if event.explosion.source == Some(71)
                && self.0.bursts.fetch_add(1, Ordering::Relaxed) == 0
            {
                // Only try once so an unguarded removal is a bounded failure.
                let source = host.entity(71).expect("the burst's source still resolves");
                assert!(
                    !source.remove(host, mod_api::RemovalReason::Killed),
                    "the burst cannot remove its own source again"
                );
            }
            Flow::Continue
        }

        fn entity_leave_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::EntityLeaveLevelEvent<'_>,
        ) {
            if event.entity == 71 {
                assert!(host.entity(71).is_none());
                self.0.leaves.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    let events = Arc::new(Events::default());
    with_removal_test_hooks(Reenter(events.clone()), |game| {
        push_pig(game, 71);
        push_pig(game, 72);
        game.mob_object_mut(71).unwrap().extra.effects.push(
            sunrise_client::mobs::StoredEffect::new("minecraft:wind_charged", 600, 0),
        );
        game.native_joins_flush(false, 0);
        assert!(game.try_mount_mob(72, 71));

        finish_death_on_next_tick(game, 71);
        game.native_leaves_flush();

        assert!(game.entity(71).is_none());
        assert_eq!(game.native_vehicle_id(Target::Mob(72)), None);
        assert_eq!(events.bursts.load(Ordering::Relaxed), 1);
        assert_eq!(events.leaves.load(Ordering::Relaxed), 1);
        assert_eq!(
            game.audio.heard.get("minecraft:entity.breeze.wind_burst"),
            Some(&1)
        );
    });
}

/// The same trace as VanillaVehicleRemovalOracle: the departing pig's own
/// edge first, its direct riders in passenger-list order rather than row order,
/// no recursive dismount, and a later actor already seeing the new topology.
#[test]
#[ignore = "daily"]
fn mob_removal_order_witness() {
    let observation = Arc::new(Observation::default());
    with_removal_test_hooks(Observe(observation.clone()), |game| {
        game.boats
            .push(super::boats::Boat::new(70, "oak".into(), AT, 0.0));
        for id in 71..=75 {
            push_pig(game, id);
        }
        for (rider, vehicle) in [(71, 70), (73, 71), (72, 71), (75, 72)] {
            assert!(game.try_mount_mob(rider, vehicle));
        }
        finish_death_on_next_tick(game, 71);
        assert!(game.entity(71).is_none());
        assert_eq!(game.native_vehicle_id(Target::Mob(73)), None);
        assert_eq!(game.native_vehicle_id(Target::Mob(75)), Some(72));
    });
    let trace = observation.trace.lock().unwrap();
    assert_eq!(
        *trace,
        [
            "71:removed=true",
            "73:removed=true",
            "72:removed=true",
            "later:riding=false"
        ]
    );
    if let Some(path) = std::env::var_os("SUNRISE_VEHICLE_REMOVAL_TRACE") {
        std::fs::write(path, trace.join("\n") + "\n").unwrap();
    }
}
