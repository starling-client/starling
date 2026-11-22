//! `living_knockback` heard by a trait mod through the game's own bridge: who the hook names
//! as the attacker, what the attacker holds, what a mod may cancel, and thorns' wearer.
use super::entity::damage_level_of;
use super::native_entity_refs::tests::with_game;
use mod_api::{
    self, HitHost, Hooks, HurtSource, LevelRandomHost, LivingIncomingDamageEvent,
    LivingKnockbackEvent,
};
use std::sync::{Arc, Mutex};
use sunrise_client::health::{Attack, Hurting, Knock, NO_ATTACK};
use sunrise_client::items::ItemStack;
use sunrise_client::mobs::{Damage, DamageKind, Mob, Who};
use sunrise_core::{geometry::Vec3, random::RandomSource};

/// What a knockback heard: the attacker and the stacks it holds (by item id).
type Held = (i32, Option<String>, Option<String>);

#[derive(Default)]
pub(in crate::game) struct Heard {
    knocks: Vec<Held>,
    pub(in crate::game) credits: Vec<(i32, Option<String>)>,
    /// The entry of `LivingEntity.knockback` each push came through, as the hook named it.
    entries: Vec<mod_api::KnockbackEntry>,
    incoming_causing: Vec<i32>,
    cancel: bool,
    repeat_short: bool,
    pub(in crate::game) reduce_sword_credit: bool,
}

struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_knockback(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingKnockbackEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        let id = |s: Option<&mod_api::ItemStack>| {
            s.and_then(|s| mod_api::__private::item_key_id(s.item()).map(str::to_owned))
        };
        heard.knocks.push((
            event.attacker,
            id(event.attacker_main),
            id(event.attacker_off),
        ));
        heard
            .credits
            .push((event.kill_credit, id(event.kill_credit_main)));
        heard.entries.push(event.entry);
        if heard.reduce_sword_credit {
            if id(event.kill_credit_main).as_deref() == Some("minecraft:iron_sword") {
                event.strength *= 0.5;
            }
        } else {
            event.strength *= 2.0;
        }
        if heard.repeat_short && event.entry == mod_api::KnockbackEntry::Short {
            event.strength *= 2.0;
        }
        event.x = -event.x;
        if heard.cancel {
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .lock()
            .unwrap()
            .incoming_causing
            .push(event.source.causing);
        mod_api::Flow::Continue
    }
}

pub(in crate::game) fn heard(test: impl FnOnce(&mut super::Game<'_>, &Mutex<Heard>)) -> Heard {
    hear(|log| with_game(|game| test(game, log)))
}

fn guest_heard(test: impl FnOnce(&mut super::Game<'_>, &Mutex<Heard>)) -> Heard {
    hear(|log| super::native_guest_turn_tests::with_guests(|game| test(game, log)))
}

fn hear(test: impl FnOnce(&Mutex<Heard>)) -> Heard {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(&log));
    std::mem::take(&mut *log.lock().unwrap())
}

fn source(causing: i32) -> HurtSource<'static> {
    HurtSource {
        damage_type: "minecraft:player_attack",
        causing,
        direct: 0,
        position: None,
    }
}

const PUSH: Knock = Knock {
    strength: 0.4,
    x: 1.0,
    z: 0.0,
};

#[test]
#[ignore = "daily"]
fn the_bridge_names_the_attacker_and_what_it_holds() {
    let heard = heard(|game, _| {
        let mob = {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(12, kind, Vec3::ZERO, 0.0);
            mob.ai.last_hurt_by = Some(Who::Mob(99));
            mob
        };
        let by_player = Damage {
            attacker: Some(Who::Player),
            ..Damage::of(DamageKind::Generic)
        };
        let by_mob = Damage {
            attacker: Some(Who::Mob(3)),
            ..by_player
        };
        let by_nobody = Damage {
            attacker: None,
            ..by_player
        };
        let mut push = |pushed: Hurting<'_>, attack: &Attack| {
            let victim = match &pushed {
                Hurting::Mob(mob, _) => mob.id as i32,
                _ => 12,
            };
            let plan = game.prepare_knockback_credit(victim, None, game.combat.attack());
            let mut credit = plan.lend(&mut game.last_hurt_by_player);
            let mut knock = PUSH;
            let canceled = super::native_hooks::knockback(
                damage_level_of!(game, attack, &mut credit, None).reborrow(),
                pushed,
                &mut knock,
            );
            (canceled, knock)
        };
        let by_seven = Attack {
            attacker: 7,
            projectile: None,
            direct: None,
            player_uuid: None,
            held: Default::default(),
            ..Default::default()
        };
        for pushed in [
            Hurting::Mob(&mob, &by_player),
            Hurting::Player(12, source(0)),
        ] {
            // The probe doubles the strength and mirrors the direction.
            let (canceled, knock) = push(pushed, &by_seven);
            assert!(!canceled);
            assert_eq!(
                knock,
                Knock {
                    strength: 0.8,
                    x: -1.0,
                    z: 0.0
                }
            );
        }
        push(Hurting::Mob(&mob, &by_mob), &by_seven);
        push(Hurting::Mob(&mob, &by_nobody), &by_seven);
        push(Hurting::Dragon(12, source(5)), &by_seven);
        push(Hurting::ArmorStand(12, source(0)), &by_seven);
        let sword = Attack {
            attacker: 7,
            projectile: None,
            direct: None,
            player_uuid: None,
            held: [Some(ItemStack::new("minecraft:iron_sword", 1)), None],
            ..Default::default()
        };
        push(Hurting::Player(12, source(0)), &sword);
        push(Hurting::Player(12, source(0)), &NO_ATTACK);
    });
    let sword = Some("minecraft:iron_sword".to_string());
    let attackers = [7, 7, 3, 99, 5, 7, 7, 0].map(|attacker| (attacker, None, None));
    let mut expected = attackers.to_vec();
    expected[6].1 = sword;
    assert_eq!(heard.knocks, expected);
}

#[test]
#[ignore = "daily"]
fn a_canceled_push_is_reported() {
    let heard = heard(|game, log| {
        let plan = game.prepare_knockback_credit(12, None, game.combat.attack());
        let mut credit = plan.lend(&mut game.last_hurt_by_player);
        let level = damage_level_of!(game, &NO_ATTACK, &mut credit, None);
        log.lock().unwrap().cancel = true;
        let mut knock = PUSH;
        assert!(super::native_hooks::knockback(
            level.reborrow(),
            Hurting::Player(12, source(7)),
            &mut knock
        ));
    });
    assert_eq!(heard.knocks.len(), 1);
}

#[test]
#[ignore = "daily"]
fn thorns_name_the_wearer_not_the_incoming_attacker() {
    use sunrise_client::motion::PlayerMotion;
    let heard = heard(|game, _| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 7;
        game.player = Some(player);
        let mut chest = ItemStack::new("minecraft:diamond_chestplate", 1);
        chest.data_mut().enchantments = vec![("minecraft:thorns".into(), 255)];
        game.armor[1] = Some(chest);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
        // The mob's own hit on the player is in progress: its attack names it.
        let incoming = game.attack_of(42);
        game.with_attack(incoming, |game| {
            game.thorns(42);
            assert_eq!(game.current_attack().attacker, 42);
        });
        assert_eq!(game.current_attack(), NO_ATTACK);
    });
    assert_eq!(heard.incoming_causing, [7]);
}

fn player_with_sword(game: &mut super::Game<'_>) {
    use sunrise_client::motion::PlayerMotion;
    let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
    player.entity_id = 7;
    game.player = Some(player);
    game.inventory[game.selected] = Some(ItemStack::new("minecraft:iron_sword", 1));
}

fn zombie_at(game: &mut super::Game<'_>, id: u32, at: Vec3) {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    game.mobs.push(Mob::new(id, kind, at, 0.0));
}

/// A player's melee hit goes `attack_mob` → `hurt_by_player` → `Mob::damage` → the hook:
/// the hook names the player and the sword it holds, and nothing is left behind afterwards.
#[test]
#[ignore = "daily"]
fn a_player_melee_hit_names_the_player_and_the_held_sword() {
    let heard = heard(|game, _| {
        player_with_sword(game);
        zombie_at(game, 42, Vec3::new(0.0, 0.0, 1.0));
        game.attack_mob(0);
        assert_eq!(game.current_attack(), NO_ATTACK);
    });
    let sword = Some("minecraft:iron_sword".to_string());
    assert!(
        heard.knocks.contains(&(7, sword, None)),
        "heard {:?}",
        heard.knocks
    );
    assert_eq!(heard.incoming_causing, [7]);
}

/// A sweep names the player and its sword too, for each mob it hits.
#[test]
#[ignore = "daily"]
fn a_sweep_names_the_player_and_the_held_sword() {
    let heard = heard(|game, _| {
        player_with_sword(game);
        zombie_at(game, 42, Vec3::new(0.0, 0.0, 1.0));
        let area = sunrise_core::geometry::Aabb::new(
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(1.0, 2.0, 2.0),
        );
        game.sweep_damage(area, super::mace::Spared::Nothing, 1.0);
    });
    let sword = Some("minecraft:iron_sword".to_string());
    // The hit's own push, then the sweep's 0.4 along the player's yaw.
    assert_eq!(
        heard.knocks,
        [(7, sword.clone(), None), (7, sword, None)],
        "heard {:?}",
        heard.knocks
    );
}

/// The attack the player-victim path lands in is lent by `with_attack`, restored when it
/// returns, and not seen across a real guest swap (the guest's combat tracker has its own).
#[test]
#[ignore = "daily"]
fn the_attack_scope_nests_restores_and_a_guest_swap_hides_it() {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_guest_turn_tests::with_guests(|game| {
            player_with_sword(game);
            let outer = game.attack_of(7);
            assert_eq!(
                outer.held[0],
                Some(ItemStack::new("minecraft:iron_sword", 1))
            );
            game.with_attack(outer.clone(), |game| {
                assert_eq!(game.current_attack(), outer);
                let seen_by_guest = game
                    .as_guest(12, |game| {
                        assert_eq!(game.current_attack(), NO_ATTACK, "the outer attack leaked");
                        let inner = Attack {
                            attacker: 9,
                            projectile: None,
                            direct: None,
                            player_uuid: None,
                            held: Default::default(),
                            ..Default::default()
                        };
                        game.with_attack(inner.clone(), |game| {
                            assert_eq!(game.current_attack(), inner);
                        });
                        game.current_attack()
                    })
                    .unwrap();
                assert_eq!(seen_by_guest, NO_ATTACK);
                assert_eq!(game.current_attack(), outer, "the swap did not restore it");
            });
            assert_eq!(game.current_attack(), NO_ATTACK, "the scope leaked");
        });
    });
}

/// What the mob's own random gives next after a push with no direction (the push then draws a
/// random one, after the hook): with or without a listening mod, and with the push canceled.
fn next_draw_after_a_directionless_push(cancel: Option<bool>) -> i32 {
    let drawn = Mutex::new(0);
    let draw = |game: &mut super::Game<'_>| {
        zombie_at(game, 42, Vec3::ZERO);
        let hit = Damage::of(DamageKind::Generic);
        let mut mob = game.mobs.pop().unwrap();
        with_detached_credit(game, 42, |level| {
            mob.knockback(level, 0.4, 0.0, 0.0, &hit, 0.0)
        });
        *drawn.lock().unwrap() = mob.random.next_int();
    };
    match cancel {
        None => with_game(draw),
        Some(cancel) => {
            let log = Arc::new(Mutex::new(Heard::default()));
            log.lock().unwrap().cancel = cancel;
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("probe", Box::new(Probe(log)))].into();
            mod_platform::testing::with_mods(mods, || with_game(draw));
        }
    }
    drawn.into_inner().unwrap()
}

#[test]
#[ignore = "daily"]
fn the_push_draws_the_same_stream_with_and_without_a_mod_and_a_canceled_one_draws_nothing() {
    let without = next_draw_after_a_directionless_push(None);
    assert_eq!(next_draw_after_a_directionless_push(Some(false)), without);
    let canceled = next_draw_after_a_directionless_push(Some(true));
    assert_ne!(canceled, without, "a canceled push must not draw");
}

/// A guest's blow on another guest lands through `pvp_land`: the hook names the attacking guest
/// and the sword in its hand, though the victim is the player swapped in when the push runs.
#[test]
#[ignore = "daily"]
fn a_guest_blow_on_a_guest_names_the_attacker_and_the_sword() {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_guest_turn_tests::with_guests(|game| {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.inventory[guest.selected] =
                        Some(ItemStack::new("minecraft:iron_sword", 1));
                })
                .unwrap();
            let mut victim =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0))
                    .unwrap();
            victim.entity_id = 13;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |victim_guest| {
                    victim_guest.motion = Some(Box::new(victim));
                    // The guest has finished loading in.
                    victim_guest.load_timeout = 0;
                })
                .unwrap();
            // A fresh body is a creative one: change the mode away and back so the survival
            // abilities apply to it.
            game.as_guest(13, |g| {
                use sunrise_client::command::args::GameMode;
                g.set_game_mode(GameMode::Creative);
                g.set_game_mode(GameMode::Survival);
            })
            .unwrap();
            let strike = super::lan_pvp::Strike::test_blow(12);
            let hurt = game.pvp_land(super::lan_pvp::Victim::Guest(13), &strike);
            let seen = log.lock().unwrap().incoming_causing.clone();
            assert!(hurt, "not hurt; damage heard from {seen:?}");
            assert_eq!(game.current_attack(), NO_ATTACK);
        });
    });
    let heard = std::mem::take(&mut *log.lock().unwrap());
    let sword = Some("minecraft:iron_sword".to_string());
    assert_eq!(heard.credits, vec![(12, sword.clone()); heard.knocks.len()]);
    assert!(
        !heard.knocks.is_empty(),
        "no push reached the hook; damage heard from {:?}",
        heard.incoming_causing
    );
    assert!(
        heard.knocks.iter().all(|k| *k == (12, sword.clone(), None)),
        "heard {:?}",
        heard.knocks
    );
}

/// A mob's fireball on the player goes through `projectile_hit`, which lends the owner's attack:
/// the hook names the shooter and the bow it holds.
#[test]
#[ignore = "daily"]
fn a_mobs_fireball_on_the_player_names_the_shooter_and_its_bow() {
    use sunrise_client::projectiles::{Hit, Projectile, ProjectileKind};
    let heard = heard(|game, _| {
        player_with_sword(game);
        game.set_game_mode(sunrise_client::command::args::GameMode::Survival);
        zombie_at(game, 42, Vec3::new(0.0, 0.0, 6.0));
        game.mobs
            .last_mut()
            .unwrap()
            .extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:bow", 1)));
        let arrow = Projectile::new(
            ProjectileKind::SmallFireball,
            Vec3::new(0.0, 1.0, 0.5),
            Some(Who::Mob(42)),
        );
        game.projectile_hit(
            &arrow,
            Hit::Entity {
                who: Who::Player,
                at: Vec3::new(0.0, 1.0, 0.0),
            },
        )
        .unwrap();
        assert_eq!(game.current_attack(), NO_ATTACK);
    });
    let bow = Some("minecraft:bow".to_string());
    assert!(
        !heard.knocks.is_empty(),
        "no push reached the hook; damage heard from {:?}",
        heard.incoming_causing
    );
    assert!(
        heard.knocks.iter().all(|k| *k == (42, bow.clone(), None)),
        "heard {:?}",
        heard.knocks
    );
}

/// The hook fires once per call of the full entry and names the entry the push came through:
/// `Short` for `knockback(power, x, z)` (which delegates to the full entry), `Full` for a direct
/// call of the full entry. A listener that wants a mixin's double change on the short entry
/// applies its change twice on `Short`.
#[test]
#[ignore = "daily"]
fn the_hook_names_the_entry_a_push_came_through_once_per_full_entry_call() {
    use mod_api::KnockbackEntry::{Full, Short};
    let heard = heard(|game, log| {
        log.lock().unwrap().repeat_short = true;
        let hit = Damage::of(DamageKind::Generic);
        zombie_at(game, 42, Vec3::ZERO);
        let mut mob = game.mobs.pop().unwrap();
        mob.velocity = Vec3::ZERO;
        mob.ground.on_ground = false;
        with_detached_credit(game, 42, |level| {
            mob.knockback(level, 0.4, 1.0, 0.0, &hit, 0.0)
        });
        assert_eq!(mob.velocity, Vec3::new(1.6, 0.0, 0.0));
        mob.velocity = Vec3::ZERO;
        assert_eq!(log.lock().unwrap().entries, [Short], "mob, short entry");
        with_detached_credit(game, 42, |level| {
            mob.knockback_full(level, 0.4, 1.0, 0.0, &hit, 0.0, false)
        });
        assert_eq!(
            log.lock().unwrap().entries,
            [Short, Full],
            "mob, full entry"
        );
        assert_eq!(mob.velocity, Vec3::new(0.8, 0.0, 0.0));
        with_detached_credit(game, 42, |level| {
            mob.knockback_full(level, 0.4, 1.0, 0.0, &hit, 0.0, true)
        });
        assert_eq!(
            log.lock().unwrap().entries,
            [Short, Full, Full],
            "mob, full entry for an effect push"
        );

        player_with_sword(game);
        game.player.as_mut().unwrap().ground.on_ground = false;
        let attack = &NO_ATTACK;
        game.knock_back_player(0.4, 1.0, 0.0, source(0), attack, 0.0);
        assert_eq!(
            game.player.as_ref().unwrap().velocity,
            Vec3::new(1.6, 0.0, 0.0)
        );
        game.player.as_mut().unwrap().velocity = Vec3::ZERO;
        game.knock_back_player_full(0.4, 1.0, 0.0, source(0), attack, 0.0, false);
        assert_eq!(
            game.player.as_ref().unwrap().velocity,
            Vec3::new(0.8, 0.0, 0.0)
        );
        let entries = log.lock().unwrap().entries.clone();
        assert_eq!(
            &entries[3..],
            [Short, Full],
            "player, short then full entry"
        );
        // The short entry leaves nothing behind: a full call after it is `Full` again.
        with_detached_credit(game, 42, |level| {
            mob.knockback_full(level, 0.4, 1.0, 0.0, &hit, 0.0, false)
        });
        assert_eq!(log.lock().unwrap().entries.last(), Some(&Full));
    });
    assert_eq!(heard.entries.len(), 6, "one event per full-entry call");
}

fn with_detached_credit(
    game: &mut super::Game<'_>,
    victim: i32,
    push: impl FnOnce(&mut sunrise_client::health::DamageLevel<'_>),
) {
    let plan = game.prepare_knockback_credit(victim, None, &NO_ATTACK);
    let mut credit = plan.lend(&mut game.last_hurt_by_player);
    push(damage_level_of!(game, &NO_ATTACK, &mut credit, None));
}

pub(in crate::game) fn credit_pig(game: &mut super::Game<'_>) {
    game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Normal;
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
    game.tickets
        .update(Some(sunrise_client::ticket_levels::Sources {
            players: vec![(0, 0)],
            forced: Vec::new(),
            portal: Vec::new(),
            dragon: None,
            viewed: Vec::new(),
            view_distance: 2,
            simulation_distance: 2,
        }));
    assert!(game.entity_ticking_area()(0, 0));
    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    let mut pig = Mob::new(100, kind, Vec3::ZERO, 0.0);
    pig.no_ai = true;
    pig.no_gravity = true;
    game.mobs.push(pig);
}

fn credit_push(game: &mut super::Game<'_>) {
    let plan = game.prepare_knockback_credit(100, None, game.combat.attack());
    let mut credit = plan.lend(&mut game.last_hurt_by_player);
    let mob = &mut game.mobs[0];
    mob.velocity = Vec3::ZERO;
    mob.ground.on_ground = false;
    mob.knockback(
        damage_level_of!(game, &NO_ATTACK, &mut credit, None),
        0.4,
        1.0,
        0.0,
        &Damage::of(DamageKind::Generic),
        0.0,
    );
    assert_eq!(mob.velocity, Vec3::new(0.8, 0.0, 0.0));
}

#[test]
#[ignore = "daily"]
fn accepted_wolf_hits_commit_owner_replacement_clearing_and_unresolved_precedence_before_push() {
    let heard = guest_heard(|game, log| {
        log.lock().unwrap().reduce_sword_credit = true;
        credit_pig(game);
        let kind = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
        let mut wolf = Mob::new(101, kind, Vec3::new(1.0, 0.0, 0.0), 0.0);
        wolf.extra.tame = true;
        wolf.extra.owner = Some(sunrise_client::lan::playerdata::uuid_ints(12));
        wolf.extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:bone", 1)));
        game.mobs.push(wolf);
        game.as_guest(13, |game| game.remember_mob_player_credit(100))
            .unwrap();
        game.as_guest(12, |game| {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:iron_sword", 1))
        })
        .unwrap();
        for (owner, strength) in [(Some(12), 0.2), (None, 0.4), (Some(999), 0.4)] {
            game.mobs[1].extra.owner = owner.map(sunrise_client::lan::playerdata::uuid_ints);
            game.mobs[0].invulnerable = 0;
            game.mobs[0].velocity = Vec3::ZERO;
            assert!(game.mob_hits_mob(100, 1.0, Damage::mob(101, [1.0, 0.0, 0.0])));
            assert!(
                (game.mobs[0].velocity.x - strength).abs() < 1e-7,
                "the listener's credit-dependent strength edit did not apply"
            );
            if owner == Some(12) {
                game.mobs[1].extra.owner = Some(sunrise_client::lan::playerdata::uuid_ints(999));
                assert!(!game.mob_hits_mob(100, 0.5, Damage::mob(101, [1.0, 0.0, 0.0])));
                let credit = game.last_hurt_by_player[&100];
                assert_eq!(
                    game.resolve_player_credit(credit),
                    Some(Some(12)),
                    "refused hit stamped a new owner"
                );
            }
        }
        assert_eq!(game.mobs[0].ai.last_hurt_by_player, 100);
        assert_eq!(log.lock().unwrap().credits.len(), 3);
    });
    assert_eq!(
        heard.credits,
        [
            (12, Some("minecraft:iron_sword".into())),
            (101, Some("minecraft:bone".into())),
            (0, None),
        ]
    );
}

#[test]
#[ignore = "daily"]
fn remembered_player_wins_after_forty_ticks_then_recent_mob_credit_expires_at_one_hundred() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::native_knockback_tests::remembered_player_wins_after_forty_ticks_then_recent_mob_credit_expires_at_one_hundred",
        "SUNRISE_KNOCKBACK_EXPIRY_CHILD",
        &["expiry"],
    ) {
        return;
    }
    let heard = guest_heard(|game, log| {
        credit_pig(game);
        zombie_at(game, 101, Vec3::new(1.0, 0.0, 0.0));
        game.mobs[1].no_ai = true;
        game.mobs[1].no_gravity = true;
        game.mobs[1]
            .extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:bone", 1)));
        game.as_guest(12, |game| {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:iron_sword", 1));
            assert!(game.mob_hits_mob(100, 1.0, Damage::player([1.0, 0.0, 0.0])));
        })
        .unwrap();
        for _ in 0..40 {
            game.tick_creature_entities().unwrap();
        }
        assert_eq!(game.mobs[0].ai.last_hurt_by_player, 60);
        game.as_guest(12, |game| {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:iron_axe", 1))
        })
        .unwrap();
        assert!(game.mob_hits_mob(100, 1.0, Damage::mob(101, [1.0, 0.0, 0.0])));
        for _ in 0..60 {
            game.tick_creature_entities().unwrap();
        }
        assert_eq!(game.mobs[0].ai.last_hurt_by_player, 0);
        credit_push(game);
        for _ in 0..40 {
            game.tick_creature_entities().unwrap();
        }
        credit_push(game);
        game.tick_creature_entities().unwrap();
        credit_push(game);
        assert_eq!(log.lock().unwrap().credits.len(), 5);
    });
    assert_eq!(
        heard.credits,
        [
            (12, Some("minecraft:iron_sword".into())),
            (12, Some("minecraft:iron_axe".into())),
            (12, Some("minecraft:iron_axe".into())),
            (101, Some("minecraft:bone".into())),
            (0, None),
        ]
    );
}

#[test]
#[ignore = "daily"]
fn queued_player_hurts_select_the_current_source_not_an_older_hit_of_the_same_type() {
    let heard = heard(|game, _| {
        player_with_sword(game);
        super::native_entity_refs::tests::bind_local_player(game, 7, 777);
        game.set_game_mode(sunrise_client::command::args::GameMode::Creative);
        game.set_game_mode(sunrise_client::command::args::GameMode::Survival);
        zombie_at(game, 42, Vec3::new(1.0, 0.0, 0.0));
        zombie_at(game, 43, Vec3::new(-1.0, 0.0, 0.0));
        game.mobs[0]
            .extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:bone", 1)));
        game.mobs[1]
            .extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_axe", 1)));
        assert!(game.mob_hits_player_from(1.0, [1.0, 0.0, 0.0], "minecraft:mob_attack", false));
        // A stronger cooldown hit commits memory without another default push.
        assert!(game.mob_hits_player_from(2.0, [-1.0, 0.0, 0.0], "minecraft:mob_attack", false));
        game.player.as_mut().unwrap().velocity = Vec3::ZERO;
        game.player.as_mut().unwrap().ground.on_ground = false;
        game.knock_back_player(0.4, 1.0, 0.0, source(0), &NO_ATTACK, 0.0);
        assert_eq!(
            game.player.as_ref().unwrap().velocity,
            Vec3::new(0.8, 0.0, 0.0)
        );
    });
    assert_eq!(
        heard.credits,
        [
            (42, Some("minecraft:bone".into())),
            (43, Some("minecraft:iron_axe".into())),
        ]
    );
}

#[test]
#[ignore = "daily"]
fn borrowed_rider_environmental_damage_never_dispatches_a_living_push() {
    let heard = heard(|game, _| {
        credit_pig(game);
        game.mobs[0].fire_ticks = 40;
        let health = game.mobs[0].health;
        for _ in 0..20 {
            game.tick_creature_entities().unwrap();
        }
        assert!(
            game.mobs[0].health < health,
            "the borrowed damage path was not exercised"
        );
    });
    assert!(heard.knocks.is_empty());
}

/// An evoker (id 50) holding an iron sword, the owner the credit tests name; with the probe's
/// `reduce_sword_credit`, a push credited to it is halved.
pub(in crate::game) fn sword_evoker(game: &mut super::Game<'_>) {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:evoker")
        .unwrap()
        .clone();
    let mut evoker = Mob::new(50, kind, Vec3::new(5.0, 0.0, 0.0), 0.0);
    evoker.no_ai = true;
    evoker.no_gravity = true;
    evoker
        .extra
        .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_sword", 1)));
    game.mobs.push(evoker);
}

/// The listener heard the evoker and its sword as the credit, and its halving reached the
/// pig's motion (an uncredited push moves it 0.4).
pub(in crate::game) fn assert_sword_credit_reached_the_push(heard: &Heard, pushed: Vec3) {
    assert_eq!(
        heard.credits,
        [(50, Some("minecraft:iron_sword".to_string()))]
    );
    assert!(
        (pushed.x.hypot(pushed.z) - 0.2).abs() < 1e-6,
        "pushed with {pushed:?}, not the halved strength"
    );
}
