//! `ServerPlayer.die`'s `killer.awardKillScore(this, source)` and
//! `this.createWitherRose(killer)` for a dying player: the killer's kill
//! counts and `player_kills` whatever killed them, and the wither rose a
//! wither's kill leaves where they fell.
use super::{chat::PLAYER_NAME, native_entity_refs::tests::run_fresh_cases, *};
use mod_api::{Flow, Hooks, PlayerStatAwardEvent, ServerHost, StatType};
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
    scoreboard::{RenderType, stat_criteria},
    text::Text,
};
use sunrise_protocol::packets::configuration::ClientInformation;

/// The guest's connection: the victim of every test.
const CONN: u32 = 2;
const VICTIM_ID: u32 = 2;
const VICTIM: &str = "H039Victim";
/// A second guest, the killer in the guest-on-guest tests.
const KILLER_CONN: u32 = 3;
const KILLER_ID: u32 = 3;
const KILLER: &str = "H039Killer";
/// The host's own body (`self.player`).
const HOST: u32 = 100;
const FRESH: &str = "SUNRISE_TEST_PLAYER_KILL_CREDIT";

fn fresh(name: &str, cases: &[&str]) -> bool {
    let (_, module) = module_path!().split_once("::").unwrap();
    run_fresh_cases(&format!("{module}::{name}"), FRESH, cases)
}

/// A host on a dirt floor, its own player at (4.5, 100, 6.5), and one
/// connected, placed guest standing on the floor at (22.5, 100, 9.5).
fn game<'a>(resources: &'a mut Option<Arc<crate::game::Resources>>) -> Game<'a> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(x, 0))
            .unwrap();
    }
    assert!(game.run_command("fill 0 99 0 31 99 15 dirt"));
    // A hostile mob spawns and the player takes damage on any but peaceful.
    assert!(game.run_command("difficulty normal"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 6.5)).unwrap());
    game.player.as_mut().unwrap().entity_id = HOST as i32;
    game.lan_host(
        super::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::lan(0, "h039-test".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    game.lan.as_mut().unwrap().hub.guests.push(guest(
        CONN,
        VICTIM_ID as i32,
        VICTIM,
        [22.5, 100.0, 9.5],
    ));
    game
}

/// A connected, placed guest at `at`, whose client has loaded the world
/// (`ServerGamePacketListenerImpl.hasClientLoaded`).
fn guest(conn: u32, entity_id: i32, name: &str, at: [f64; 3]) -> Guest {
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: u128::from(conn),
            name: name.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        entity_id,
        at,
        1,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest
}

enum Attacker {
    Player(u32, &'static str),
    Mob(u32),
    None,
}

/// Hurts the guest to death and checks that `ServerPlayer.die` ran inside the hit.
fn kill(game: &mut Game<'_>, attacker: Attacker, ty: &str) {
    game.as_guest(CONN, |g| {
        let known = match attacker {
            Attacker::Player(id, name) => {
                g.note_player_attacker(ty, id, Text::literal(name), None);
                None
            }
            Attacker::Mob(id) => Some(id),
            Attacker::None => None,
        };
        assert!(
            g.hurt_player_from(100.0, Some([22.5, 100.0, 9.5]), ty, Some(known), false),
            "the guest is hurt"
        );
        assert_eq!(g.player.as_ref().unwrap().health.health(), 0.0);
        assert!(g.lan.as_ref().unwrap().hub.guest(CONN).unwrap().dead);
    })
    .unwrap();
}

/// The score `holder` has on the objective `objective`, if any.
fn score(game: &Game<'_>, holder: &str, objective: &str) -> Option<i32> {
    game.score.board.score(holder, objective).map(|s| s.value)
}

fn add_objectives(game: &mut Game<'_>) {
    let player_kills = stat_criteria("minecraft:custom", "minecraft:player_kills");
    for (name, criteria) in [
        ("kills", "totalKillCount"),
        ("pvp", "playerKillCount"),
        ("player_kills_stat", &player_kills),
        ("teamkill", "teamkill.blue"),
        ("killedbyteam", "killedByTeam.red"),
    ] {
        assert!(game.score.board.add_objective(
            name,
            criteria,
            serde_json::json!(name),
            RenderType::Integer,
            name == "kills",
            None,
        ));
    }
}

fn guest_stats<'a>(game: &'a Game<'_>, conn: u32) -> &'a super::stats::Stats {
    &game.lan.as_ref().unwrap().play[&conn].stats
}

fn assert_player_kill(game: &Game<'_>, killer: &str) {
    assert_eq!(score(game, killer, "kills"), Some(1));
    assert_eq!(score(game, killer, "pvp"), Some(1));
    assert_eq!(score(game, VICTIM, "kills"), None);
    assert_eq!(score(game, VICTIM, "pvp"), None);
    let victim = guest_stats(game, CONN);
    assert_eq!(victim.get("custom", "minecraft:player_kills"), 0);
    assert_eq!(victim.get("custom", "minecraft:deaths"), 1);
    assert_eq!(victim.get("killed_by", "minecraft:player"), 1);
}

/// `ServerPlayer.awardKillScore` credits the killer even while their body is displaced.
#[test]
#[ignore = "daily"]
fn a_player_killing_another_player_credits_the_kill() {
    if fresh(
        "a_player_killing_another_player_credits_the_kill",
        &["local"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = game(&mut resources);
    add_objectives(&mut game);
    let host_team = game.score.board.add_team("host");
    host_team.color = Some("red".into());
    host_team.prefix = sunrise_client::scoreboard::literal("Host: ");
    game.score.board.add_team("guest").color = Some("blue".into());
    assert!(game.score.board.add_to_team(PLAYER_NAME, "host"));
    assert!(game.score.board.add_to_team(VICTIM, "guest"));

    kill(
        &mut game,
        Attacker::Player(HOST, PLAYER_NAME),
        "minecraft:player_attack",
    );
    assert_player_kill(&game, PLAYER_NAME);
    assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 1);
    assert_eq!(game.stats.get("custom", "minecraft:deaths"), 0);
    assert_eq!(game.stats.get("killed_by", "minecraft:player"), 0);
    assert_eq!(score(&game, PLAYER_NAME, "teamkill"), Some(1));
    assert_eq!(score(&game, VICTIM, "killedbyteam"), Some(1));
    let display = game
        .score
        .board
        .score(PLAYER_NAME, "kills")
        .unwrap()
        .display
        .as_ref()
        .unwrap();
    assert_eq!(
        sunrise_client::scoreboard::comp_text(display).plain(),
        format!("Host: {PLAYER_NAME}")
    );
    game.tick_scoreboard();
    assert_eq!(score(&game, PLAYER_NAME, "player_kills_stat"), Some(1));
    assert_eq!(score(&game, VICTIM, "player_kills_stat"), None);
}

/// `Player.doSweepAttack` uses the local player's source for a secondary victim.
#[test]
#[ignore = "daily"]
fn a_local_player_sweep_credits_the_secondary_kill() {
    if fresh(
        "a_local_player_sweep_credits_the_secondary_kill",
        &["sweep"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = game(&mut resources);
    add_objectives(&mut game);
    add_guest_killer(&mut game);
    game.player.as_mut().unwrap().position = Vec3::new(20.5, 100.0, 9.5);
    let primary = game.guest_index(KILLER_CONN).unwrap();
    let victim = game.guest_index(CONN).unwrap();
    let guests = &mut game.lan.as_mut().unwrap().hub.guests;
    guests.update(victim, |g| g.health = 1.0);
    guests.update(primary, |g| g.position = [21.5, 100.0, 9.5]);
    let target = super::lan_mobs::guest_bounds(&guests[primary]);
    game.sweep_damage(target, super::mace::Spared::Player(KILLER_ID as i32), 1.0);

    assert!(game.lan.as_ref().unwrap().hub.guest(CONN).unwrap().dead);
    assert!(
        !game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(KILLER_CONN)
            .unwrap()
            .dead
    );
    assert_player_kill(&game, PLAYER_NAME);
    assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 1);
    game.tick_scoreboard();
    assert_eq!(score(&game, PLAYER_NAME, "player_kills_stat"), Some(1));
    assert_eq!(score(&game, VICTIM, "player_kills_stat"), None);
}

/// The same for a LAN guest's kill: a hosted world's players are all guests,
/// the owner's window too, so the killer's own record takes the kill.
#[test]
#[ignore = "daily"]
fn a_guest_killing_another_guest_credits_the_kill() {
    if fresh(
        "a_guest_killing_another_guest_credits_the_kill",
        &["parked"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = game(&mut resources);
    add_guest_killer(&mut game);
    add_objectives(&mut game);
    kill(
        &mut game,
        Attacker::Player(KILLER_ID, KILLER),
        "minecraft:player_attack",
    );
    assert_player_kill(&game, KILLER);
    assert_eq!(
        guest_stats(&game, KILLER_CONN).get("custom", "minecraft:player_kills"),
        1
    );
    assert_eq!(score(&game, KILLER, "player_kills_stat"), Some(1));
    // The host's own body is not the killer.
    assert_eq!(score(&game, PLAYER_NAME, "kills"), None);
    assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 0);
}

fn add_guest_killer(game: &mut Game<'_>) {
    game.lan.as_mut().unwrap().hub.guests.push(guest(
        KILLER_CONN,
        KILLER_ID as i32,
        KILLER,
        [18.5, 100.0, 9.5],
    ));
}

struct RewriteKillerAward;

#[mod_api::__subscriptions]
impl Hooks for RewriteKillerAward {
    fn player_stat_award(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerStatAwardEvent<'_>,
    ) -> Flow {
        if event.player == u128::from(KILLER_CONN)
            && event.stat.as_str() == "minecraft:player_kills"
        {
            event.value *= 2;
        }
        Flow::Continue
    }
}

/// A killer displaced by a nested turn is not in their hub record either.
/// The statistic still goes through the killer's award hook.
#[test]
#[ignore = "daily"]
fn a_guest_killing_another_guest_while_displaced_credits_the_kill() {
    if fresh(
        "a_guest_killing_another_guest_while_displaced_credits_the_kill",
        &["nested"],
    ) {
        return;
    }
    let mods = vec![mod_api::ModEntry::new(
        "rewrite",
        Box::new(RewriteKillerAward),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut resources = None;
        let mut game = game(&mut resources);
        add_guest_killer(&mut game);
        add_objectives(&mut game);
        game.as_guest(KILLER_CONN, |g| {
            kill(
                g,
                Attacker::Player(KILLER_ID, KILLER),
                "minecraft:player_attack",
            );
            assert_player_kill(g, KILLER);
            assert_eq!(g.stats.get("custom", "minecraft:player_kills"), 2);
            assert!(g.stats.take_dirty().contains(&(
                "minecraft:custom".into(),
                "minecraft:player_kills".into(),
                2,
            )));
        })
        .unwrap();
        assert_eq!(
            guest_stats(&game, KILLER_CONN).get("custom", "minecraft:player_kills"),
            2
        );
        assert_eq!(score(&game, KILLER, "player_kills_stat"), Some(2));
        assert_eq!(score(&game, PLAYER_NAME, "kills"), None);
        assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 0);
    });
}

struct ReadKillerBeforeAward;

#[mod_api::__subscriptions]
impl Hooks for ReadKillerBeforeAward {
    fn player_stat_award(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerStatAwardEvent<'_>,
    ) -> Flow {
        if event.player == u128::from(KILLER_CONN)
            && event.stat.as_str() == "minecraft:player_kills"
            && let Some(killer) = host.players().player_by_name(event.name)
            && killer.uuid() == event.player
            && killer.stat(host, StatType::Custom, "minecraft:player_kills") == Some(7)
        {
            event.value = 3;
        }
        Flow::Continue
    }
}

/// NeoForge's `StatAwardEvent` sees the killer's unchanged count in either turn arrangement.
#[test]
#[ignore = "daily"]
fn a_stat_award_listener_reads_the_killers_pre_award_count() {
    if fresh(
        "a_stat_award_listener_reads_the_killers_pre_award_count",
        &["parked", "displaced"],
    ) {
        return;
    }
    let mods = vec![mod_api::ModEntry::new(
        "read_killer",
        Box::new(ReadKillerBeforeAward),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut resources = None;
        let mut game = game(&mut resources);
        add_guest_killer(&mut game);
        add_objectives(&mut game);
        game.as_guest(KILLER_CONN, |g| g.stats.custom("player_kills", 7))
            .unwrap();
        let hit = |g: &mut Game<'_>| {
            kill(
                g,
                Attacker::Player(KILLER_ID, KILLER),
                "minecraft:player_attack",
            );
        };
        match std::env::var(FRESH).unwrap().as_str() {
            "parked" => hit(&mut game),
            "displaced" => game.as_guest(KILLER_CONN, hit).unwrap(),
            case => panic!("unexpected fresh case {case}"),
        }
        assert_player_kill(&game, KILLER);
        assert_eq!(
            guest_stats(&game, KILLER_CONN).get("custom", "minecraft:player_kills"),
            10
        );
        assert_eq!(score(&game, KILLER, "player_kills_stat"), Some(10));
    });
}

/// The victim's `KilledTrigger` evaluates the actual killer's predicate data.
#[test]
#[ignore = "daily"]
fn a_victim_advancement_matches_the_displaced_killers_facts() {
    use sunrise_client::advancements::{Advancement, Criterion, Rewards, Tree};
    use sunrise_client::items::ItemStack;

    if fresh(
        "a_victim_advancement_matches_the_displaced_killers_facts",
        &["local", "nested"],
    ) {
        return;
    }
    const ADVANCEMENT: &str = "test:know_the_killer";
    const TRIGGER: &str = "minecraft:entity_killed_player";
    let mut resources = None;
    let mut game = game(&mut resources);
    add_guest_killer(&mut game);
    game.score.board.add_team("killers");
    assert!(game.score.board.add_to_team(PLAYER_NAME, "killers"));
    assert!(game.score.board.add_to_team(KILLER, "killers"));
    let mut tree = Tree::default();
    tree.list.push(Advancement {
        id: ADVANCEMENT.into(),
        parent: None,
        display: None,
        criteria: vec![Criterion {
            name: "killed".into(),
            trigger: TRIGGER.into(),
            conditions: serde_json::json!({ "entity": [{
                "condition": "minecraft:entity_properties",
                "entity": "this",
                "predicate": {
                    "minecraft:team": "killers",
                    "minecraft:equipment": {
                        "mainhand": { "items": "minecraft:iron_sword" },
                        "offhand": { "items": "minecraft:shield" },
                        "head": { "items": "minecraft:iron_helmet" }
                    },
                    "minecraft:type_specific/player": {
                        "level": 13,
                        "stats": [{"type": "minecraft:custom", "stat": "minecraft:jump", "value": 37}],
                        "recipes": {"minecraft:bread": true}
                    }
                }
            }] }).into(),
        }],
        requirements: vec![vec!["killed".into()]],
        rewards: Rewards::default(),
    });
    tree.by_id.insert(ADVANCEMENT.into(), 0);
    tree.by_trigger.insert(TRIGGER.into(), vec![(0, 0)]);
    tree.index_player_facts();
    game.adv.tree = Arc::new(tree);

    let hit = |g: &mut Game<'_>, id, name| {
        g.selected = 4;
        g.inventory[4] = Some(ItemStack::new("minecraft:iron_sword", 1));
        g.offhand = Some(ItemStack::new("minecraft:shield", 1));
        g.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
        g.experience.level = 13;
        g.stats.loaded = true;
        g.stats.custom("jump", 37);
        g.level.recipe_book.recipes.insert("minecraft:bread".into());
        kill(g, Attacker::Player(id, name), "minecraft:player_attack");
    };
    match std::env::var(FRESH).unwrap().as_str() {
        "local" => hit(&mut game, HOST, PLAYER_NAME),
        "nested" => game
            .as_guest(KILLER_CONN, |g| hit(g, KILLER_ID, KILLER))
            .unwrap(),
        case => panic!("unexpected fresh case {case}"),
    }
    let victim = game.lan.as_ref().unwrap().hub.guest(CONN).unwrap();
    assert!(victim.advancements.has(ADVANCEMENT, "killed"));
}

/// No kill score for the victim's own hit, or for damage with no killer.
#[test]
#[ignore = "daily"]
fn self_kills_and_uncredited_deaths_award_no_kill_score() {
    if fresh(
        "self_kills_and_uncredited_deaths_award_no_kill_score",
        &["self", "fall"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = game(&mut resources);
    add_objectives(&mut game);
    let (attacker, ty, killed_by_player) = match std::env::var(FRESH).unwrap().as_str() {
        "self" => (
            Attacker::Player(VICTIM_ID, VICTIM),
            "minecraft:player_attack",
            1,
        ),
        "fall" => (Attacker::None, "minecraft:fall", 0),
        case => panic!("unexpected fresh case {case}"),
    };
    kill(&mut game, attacker, ty);
    for holder in [PLAYER_NAME, VICTIM] {
        assert_eq!(score(&game, holder, "kills"), None);
        assert_eq!(score(&game, holder, "pvp"), None);
    }
    assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 0);
    let victim = guest_stats(&game, CONN);
    assert_eq!(victim.get("custom", "minecraft:player_kills"), 0);
    assert_eq!(victim.get("custom", "minecraft:deaths"), 1);
    assert_eq!(
        victim.get("killed_by", "minecraft:player"),
        killed_by_player
    );
}

/// `LivingEntity.createWitherRose`: a player a wither kills leaves a rose where
/// they died, and a mob's kill credits no score.
#[test]
#[ignore = "daily"]
fn a_wither_killing_a_player_leaves_a_rose() {
    if fresh("a_wither_killing_a_player_leaves_a_rose", &["block"]) {
        return;
    }
    let mut resources = None;
    let mut game = game(&mut resources);
    add_objectives(&mut game);
    assert!(
        game.run_command("summon wither 8.5 100 4.5 {NoAI:1b,Silent:1b,PersistenceRequired:1b}")
    );
    let wither = game.mobs.last().unwrap().id;
    kill(&mut game, Attacker::Mob(wither), "minecraft:mob_attack");
    let name = &*game.res.facts().get(game.world.block(22, 100, 9)).name;
    assert_eq!(name, "minecraft:wither_rose", "the rose block");
    assert_eq!(
        score(&game, PLAYER_NAME, "kills"),
        None,
        "a mob's kill credits no score"
    );
    assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 0);
    assert!(game.dropped.is_empty(), "the block leaves no item");
}

/// The same with mobGriefing off: the rose is an item at the death position.
#[test]
#[ignore = "daily"]
fn a_wither_killing_a_player_without_mob_griefing_drops_the_rose() {
    if fresh(
        "a_wither_killing_a_player_without_mob_griefing_drops_the_rose",
        &["item"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = game(&mut resources);
    // Experience to drop: `dropAllDeathLoot` drops it before the rose.
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.xp_level = 5);
    assert!(game.run_command("gamerule mob_griefing false"));
    assert!(
        game.run_command("summon wither 8.5 100 4.5 {NoAI:1b,Silent:1b,PersistenceRequired:1b}")
    );
    let wither = game.mobs.last().unwrap().id;
    kill(&mut game, Attacker::Mob(wither), "minecraft:mob_attack");
    let rose = game.dropped.last().expect("a rose item");
    assert_eq!(&*rose.stack.item, "minecraft:wither_rose");
    assert_eq!(rose.stack.count, 1);
    let p = rose.position;
    assert_eq!((p.x, p.y, p.z), (22.5, 100.0, 9.5));
    // `dropAllDeathLoot` drops the experience first: the orbs take their ids
    // and their own randoms' seeds before the rose's item entity.
    assert!(!game.orbs.is_empty(), "the experience orbs");
    assert!(
        game.orbs.iter().all(|o| o.id < rose.id),
        "the rose's item entity comes after the orbs"
    );
}
