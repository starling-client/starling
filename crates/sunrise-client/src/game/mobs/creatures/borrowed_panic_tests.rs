use super::*;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use mod_api::__private::{HookPanicReport, take_hook_panics};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_client::{
    chunk::Chunk,
    join_seq::residency::{
        Change, Journal, Kind, Owner,
        table::{EntityLookup, EntityTable, SourceEntity},
    },
};
use sunrise_core::random::LegacyRandom;
use sunrise_mod_runtime::attachments::{AttachmentType, Holder};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Damage,
    Shield,
    Totem,
    Movement,
    Goal,
    MountedGoal,
    Knockback,
    Landing,
}
impl Family {
    fn is_goal(self) -> bool {
        matches!(self, Self::Goal | Self::MountedGoal)
    }

    fn hook(self) -> &'static str {
        match self {
            Self::Damage => "living_incoming_damage",
            Self::Shield => "living_shield_block",
            Self::Totem => "living_use_totem",
            Self::Movement => "living_speed",
            Self::Knockback => "living_knockback",
            Self::Landing => "fall_on",
            Self::Goal | Self::MountedGoal => unreachable!("goals use the unwind fixture"),
        }
    }
}

#[derive(Debug)]
struct PanicToken(u64);
const LEVEL_SEED: i64 = 0x5eed;
const FAILING_MOD: &str = "borrowed-panic";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Proposal {
    Cancelled(bool),
    Blocked(bool),
    Speed(f32),
    FallMultiplier(f32),
}

#[derive(Debug, PartialEq, Eq)]
enum OutputNotice {
    Guest(i32),
    Egg,
}

struct Probe {
    family: Family,
    calls: AtomicUsize,
    posts: Mutex<Vec<u32>>,
    first_proposal: Mutex<Option<Proposal>>,
    healthy_proposals: Mutex<Vec<Proposal>>,
    draw: Mutex<Option<i64>>,
    outputs: Mutex<Vec<OutputNotice>>,
    goal_payload: Mutex<Option<Box<PanicToken>>>,
}
impl Probe {
    fn goal_callback(&self, family: Family, target: bool) {
        if self.family == family && target {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let payload = self.goal_payload.lock().unwrap().take();
            if let Some(payload) = payload {
                eprintln!("BORROWED_NATIVE_REACHED:{family:?}");
                std::panic::resume_unwind(payload);
            }
        }
    }

    fn enter_failing(&self, input: Proposal) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.first_proposal.lock().unwrap() = Some(input);
    }

    fn progress(&self, host: &mut dyn mod_api::LevelRandomHost) {
        *self.draw.lock().unwrap() = host.with_random(|random| random.next_long());
        let _ = VALUE.set(host.attachments(), Holder::Level, 314);
    }
}

fn assert_panic_report(mod_id: &'static str, hook: &'static str) {
    assert_eq!(take_hook_panics(), [HookPanicReport { mod_id, hook }]);
}
struct GoalSpec {
    probe: Arc<Probe>,
    target: bool,
}
impl mod_api::Goal for GoalSpec {
    fn name(&self) -> &'static str {
        if self.target {
            "BorrowedTarget"
        } else {
            "BorrowedProbe"
        }
    }
    fn flags(&self) -> mod_api::GoalFlags {
        if self.target {
            mod_api::GoalFlags::TARGET
        } else {
            mod_api::GoalFlags::MOVE
        }
    }
    fn can_use(&mut self, _: &mut mod_api::GoalContext<'_>) -> bool {
        true
    }
    fn tick(&mut self, cx: &mut mod_api::GoalContext<'_>) {
        if self.probe.family == Family::MountedGoal {
            if !self.target {
                assert!(cx.move_to([3.0, 80.0, 0.5], 1.25));
                eprintln!("BORROWED_MOUNTED_MOVE_SUCCEEDED");
                self.probe
                    .goal_callback(Family::MountedGoal, cx.mob().entity().id() == 72);
            }
            return;
        }
        assert_eq!(
            cx.mob().entity().position(cx.host()),
            Some([2.0, 80.0, 0.5])
        );
        if !self.target {
            self.probe
                .goal_callback(Family::Goal, cx.mob().entity().id() == 72);
        }
    }
}
struct GoalPart(Arc<Probe>);
#[mod_api::sunrise_part]
impl mod_api::MobEffectPart for GoalPart {
    fn should_apply_effect_tick_this_tick(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::ShouldApplyEffectTickThisTickEvent,
    ) {
        event.apply = self.0.family == Family::Knockback;
    }
    fn apply_effect_tick(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::ApplyEffectTickEvent<'_>,
    ) {
        if self.0.family == Family::Knockback && event.entity == Some(72) {
            eprintln!("BORROWED_REAL_EFFECT_HURT");
            event.hurt("minecraft:mob_attack", 1.0);
        }
    }
}
struct FailingLanding(Arc<Probe>);
#[mod_api::sunrise_part]
impl mod_api::BlockPart for FailingLanding {
    fn fall_on(
        &self,
        host: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::FallOnEvent<'_>,
    ) {
        if event.entity == 72 {
            self.0
                .enter_failing(Proposal::FallMultiplier(event.damage_multiplier));
            self.0.progress(host);
            event.damage_multiplier = 0.0;
            panic!("borrowed landing callback");
        }
    }
}

struct HealthyLanding(Arc<Probe>);
#[mod_api::sunrise_part]
impl mod_api::BlockPart for HealthyLanding {
    fn fall_on(&self, _: &mut dyn mod_api::LevelRandomHost, event: &mut mod_api::FallOnEvent<'_>) {
        if event.entity == 72 {
            self.0
                .healthy_proposals
                .lock()
                .unwrap()
                .push(Proposal::FallMultiplier(event.damage_multiplier));
        }
    }
}
#[mod_api::sunrise_part]
impl mod_api::EntityPart for GoalPart {
    fn register_goals(&self, goals: &mut mod_api::GoalRegistrar<'_>) {
        goals.add_target_goal(
            0,
            GoalSpec {
                probe: self.0.clone(),
                target: true,
            },
        );
        goals.add_goal(
            0,
            GoalSpec {
                probe: self.0.clone(),
                target: false,
            },
        );
    }
}
struct Hooks {
    probe: Arc<Probe>,
    panics: bool,
}
impl Hooks {
    fn hear(&self, family: Family, target: bool, input: Proposal) -> bool {
        if self.probe.family != family || !target {
            return false;
        }
        if self.panics {
            self.probe.enter_failing(input);
        } else {
            self.probe.healthy_proposals.lock().unwrap().push(input);
        }
        self.panics
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Hooks {
    fn item_spawn(&self, _: &mut dyn mod_api::ServerHost, event: &mut mod_api::ItemSpawnEvent<'_>) {
        if !self.panics && event.item == "minecraft:egg" {
            self.probe.outputs.lock().unwrap().push(OutputNotice::Egg);
        }
    }
    fn entity_tick_listened(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityTickListenedEvent,
    ) {
        if self.panics {
            event.listened = true;
        }
    }
    fn entity_tick_post(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        if self.panics {
            self.probe
                .posts
                .lock()
                .unwrap()
                .extend_from_slice(event.entities);
        }
    }
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        if self.probe.family == Family::Landing {
            let stone = r.block_key("minecraft:stone");
            if self.panics {
                r.block_part(stone, FailingLanding(self.probe.clone()));
            } else {
                r.block_part(stone, HealthyLanding(self.probe.clone()));
            }
        }
        if !self.panics {
            return;
        }
        r.register_attachment_type("borrowed-panic:retained", false)
            .unwrap();
        if self.probe.family.is_goal() {
            r.entity_part("minecraft:zombie", GoalPart(self.probe.clone()));
        }
        if self.probe.family == Family::Knockback {
            let effect = r.effect_key("borrowed-panic:hit");
            r.mob_effect_part(&effect, GoalPart(self.probe.clone()))
                .unwrap();
        }
    }
    fn living_knockback(
        &self,
        host: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::LivingKnockbackEvent<'_>,
    ) -> mod_api::Flow {
        if self.hear(
            Family::Knockback,
            event.entity == 72,
            Proposal::Cancelled(event.cancelled),
        ) {
            self.probe.progress(host);
            event.cancelled = true;
            panic!("borrowed knockback callback");
        }
        mod_api::Flow::Continue
    }
    fn living_incoming_damage(
        &self,
        host: &mut dyn mod_api::HitHost,
        event: &mut mod_api::LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        if !self.panics && event.kind == "minecraft:player" {
            self.probe
                .outputs
                .lock()
                .unwrap()
                .push(OutputNotice::Guest(event.entity));
        }
        if self.hear(
            Family::Damage,
            event.entity == 72,
            Proposal::Cancelled(event.cancelled),
        ) {
            self.probe.progress(host);
            event.cancelled = true;
            panic!("borrowed incoming damage callback");
        }
        mod_api::Flow::Continue
    }
    fn living_shield_block(
        &self,
        host: &mut dyn mod_api::HitHost,
        event: &mut mod_api::LivingShieldBlockEvent<'_>,
    ) {
        if self.hear(
            Family::Shield,
            event.entity == 72,
            Proposal::Blocked(event.block.blocked),
        ) {
            self.probe.progress(host);
            event.block.blocked = true;
            event.block.blocked_damage = event.damage;
            panic!("borrowed shield callback");
        }
    }
    fn living_speed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingSpeedEvent<'_>,
    ) {
        if self.hear(
            Family::Movement,
            event.entity == 72,
            Proposal::Speed(event.speed),
        ) {
            event.speed += 1.0;
            panic!("borrowed movement callback");
        }
    }
    fn living_use_totem(
        &self,
        host: &mut dyn mod_api::HitHost,
        event: &mut mod_api::LivingUseTotemEvent<'_>,
    ) -> mod_api::Flow {
        if self.hear(
            Family::Totem,
            event.entity == 72,
            Proposal::Cancelled(event.cancelled),
        ) {
            self.probe.progress(host);
            event.cancelled = true;
            panic!("borrowed totem callback");
        }
        mod_api::Flow::Continue
    }
}
const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "borrowed-panic",
    name: "retained",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    Quiet,
    Egg,
    Guest,
    Push,
    Mixed,
    Reset,
}
const FRESH_CASE: &str = "SUNRISE_BORROWED_PANIC_CASE";
const BOTH_LEVELS: &[&str] = &["overworld", "nether"];

fn fresh_dimension(test: &str, cases: &[&str]) -> Option<Dim> {
    let name = format!("game::creatures::borrowed_panic_tests::{test}");
    if run_fresh_cases(&name, FRESH_CASE, cases) {
        return None;
    }
    Some(match std::env::var(FRESH_CASE).unwrap().as_str() {
        "overworld" => Dim::Overworld,
        "nether" => Dim::Nether,
        other => panic!("unexpected borrowed callback case {other}"),
    })
}

fn fixture_game(output: Output, test: impl FnOnce(&mut Game<'_>)) {
    if matches!(output, Output::Guest | Output::Mixed) {
        crate::game::native_guest_turn_tests::with_guests(test);
    } else {
        with_game(test);
    }
}

fn eggs(game: &Game<'_>) -> u32 {
    game.dropped
        .iter()
        .filter(|d| d.stack.item.as_ref() == "minecraft:egg")
        .map(|d| d.stack.count)
        .sum()
}

fn tick_in_level(game: &mut Game<'_>) -> Result<()> {
    sunrise_client::block_contact::in_level(
        sunrise_client::parallel::TickThread::mint(),
        Some(game.dim.id()),
        || game.tick_creature_entities(),
    )
}

fn prime_hook(mob: &mut Mob, family: Family) {
    mob.invulnerable_time = 0;
    if matches!(family, Family::Knockback | Family::Totem) {
        // These hooks need the hit to pass LivingEntity.hurtServer's damageCooldownTime check.
        mob.invulnerable = 0;
    }
    match family {
        Family::Damage | Family::Shield | Family::Totem => {
            // Entity.baseTick deals an on-fire hit on the 20-tick boundary.
            mob.fire_ticks = 20;
            if family == Family::Totem {
                // A previous totem leaves fire resistance and absorption. Start another lethal hit.
                mob.retain_effects(|_| false);
                mob.absorption = 0.0;
                mob.health = 0.5;
                mob.extra
                    .set_main_hand(Some("minecraft:totem_of_undying".into()));
            }
        }
        Family::Knockback => {
            mob.add_effect(
                sunrise_client::brewing::effect("borrowed-panic:hit")
                    .unwrap()
                    .id,
                10,
                0,
            );
        }
        Family::Landing => {
            mob.no_ai = false;
            mob.position = Vec3::new(2.0, 80.1, 0.5);
            mob.velocity = Vec3::new(0.0, -1.0, 0.0);
            mob.fall_distance = 6.0;
            mob.ground.on_ground = false;
        }
        Family::Movement => {
            // NoAI skips aiStep's travel; let a moving, grounded mob read its speed.
            mob.no_ai = false;
            mob.position = Vec3::new(2.0, 80.0, 0.5);
            mob.velocity = Vec3::ZERO;
            mob.ground.on_ground = true;
            mob.ai.speed = 0.2;
        }
        Family::Goal | Family::MountedGoal => {}
    }
}

fn fixture_mob(game: &Game<'_>, id: u32, family: Family, output: Output) -> Mob {
    let kind = game
        .res
        .mob_types()
        .get(match id {
            72 => "minecraft:zombie",
            73 => "minecraft:chicken",
            _ => match output {
                Output::Egg | Output::Reset => "minecraft:chicken",
                Output::Guest | Output::Mixed => "minecraft:slime",
                Output::Quiet | Output::Push => "minecraft:pig",
            },
        })
        .unwrap()
        .clone();
    let mut mob = Mob::new(id, kind, Vec3::new(f64::from(id - 70), 80.0, 0.5), 0.0);
    mob.no_ai = true;
    if output == Output::Push && id == 70 {
        mob.position.x = 1.2;
    }
    if family == Family::MountedGoal {
        mob.no_ai = false;
        mob.ground.on_ground = true;
        // Collision on the vehicle's first tick must keep it grounded.
        mob.velocity.y = -0.08;
        mob.ai.nav.speed_modifier = if id == 72 { 3.75 } else { 9.25 };
        if id == 72 {
            mob.extra.mount = Some(71);
        } else {
            mob.extra.rider = Some(72);
        }
    }
    match id {
        71 => {
            mob.ai.last_hurt_by_player = 5;
            if matches!(output, Output::Egg | Output::Guest | Output::Push) {
                mob.no_ai = false;
                mob.ground.on_ground = true;
            }
            match output {
                Output::Egg => mob.extra.egg_time = 1,
                // NoAI chickens still lay; keep the replacement's timer above zero on both ticks.
                Output::Reset => mob.extra.egg_time = 3,
                Output::Guest | Output::Mixed => sunrise_client::mobs::cube::set_size(&mut mob, 2),
                Output::Quiet | Output::Push => {}
            }
        }
        72 => {
            if family.is_goal() {
                mob.no_ai = false;
                assert!(mob.ai.goals.has("BorrowedProbe"));
            }
            prime_hook(&mut mob, family);
        }
        73 => mob.extra.egg_time = 1,
        _ => {}
    }
    mob
}

fn guest_health(game: &Game<'_>) -> [f32; 2] {
    let guests = &game.lan.as_ref().unwrap().hub.guests;
    [guests[0].health, guests[1].health]
}

fn run_output(test: &str, family: Family, output: Output) {
    if let Some(dim) = fresh_dimension(test, BOTH_LEVELS) {
        run_fixture(family, output, dim);
    }
}

fn run_fixture(family: Family, output: Output, dim: Dim) {
    crate::entry::install_client_hooks();
    let _ =
        sunrise_client::block_contact::HOOKS.set(crate::game::native_hooks::BLOCK_CONTACT_HOOKS);
    if family == Family::Knockback {
        assert!(sunrise_client::brewing::register_mod_effects([(
            "borrowed-panic:hit".into(),
            0,
            sunrise_client::brewing::Category::Neutral,
            None,
            FAILING_MOD.into(),
        )]));
    }
    let scratch = tempfile::tempdir().unwrap();
    let goal_payload = family.is_goal().then(|| Box::new(PanicToken(0xdecaf)));
    let payload_address = goal_payload
        .as_deref()
        .map(|token| token as *const PanicToken as usize);
    let probe = Arc::new(Probe {
        family,
        calls: AtomicUsize::new(0),
        posts: Mutex::default(),
        first_proposal: Mutex::default(),
        healthy_proposals: Mutex::default(),
        draw: Mutex::default(),
        outputs: Mutex::default(),
        goal_payload: Mutex::new(goal_payload),
    });
    let entries = || {
        let mut entries = vec![mod_api::ModEntry::new(
            FAILING_MOD,
            Box::new(Hooks {
                probe: probe.clone(),
                panics: true,
            }),
        )];
        if !family.is_goal() {
            entries.push(mod_api::ModEntry::new(
                "healthy",
                Box::new(Hooks {
                    probe: probe.clone(),
                    panics: false,
                }),
            ));
        }
        entries
    };
    mod_platform::testing::with_mods(entries().into(), || {
        crate::game::api_values::install_parts(&entries()).unwrap();
        fixture_game(output, |game| {
            game.world_root = scratch.path().to_path_buf();
            game.enter_level(dim).unwrap();
            game.options.dedicated = Some(Vec::new());
            if output != Output::Quiet {
                game.load_server_loot();
            }
            let guests = matches!(output, Output::Guest | Output::Mixed);
            if guests {
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Normal;
                let host = game.lan.as_mut().unwrap();
                for i in 0..host.hub.guests.len() {
                    host.hub.guests.update(i, |g| {
                        g.dimension = dim.id().to_owned();
                        g.position = if g.conn == 12 {
                            [1.5, 80.0, 0.5]
                        } else {
                            [12.0, 80.0, 12.0]
                        };
                        g.loaded = true;
                        g.load_timeout = 0;
                    });
                }
                assert_eq!(game.lan_guest_views().len(), 2);
            }
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            assert!(game.entity_ticking_area()(1, 0));
            if output == Output::Reset {
                let old_chicken = fixture_mob(game, 71, family, output);
                game.mobs.push(old_chicken);
                queue_egg(game);
                game.clear_entities();
            }
            let journal = Journal::default();
            let owner = Owner {
                dimension: game.dim.key(),
                kind: Kind::Mob,
            };
            game.mobs
                .bind_residency(&journal, owner, |m| (m.id, m.join));
            let ids: &[u32] = match output {
                Output::Push => &[70, 71, 72],
                Output::Mixed => &[71, 72, 73],
                _ => &[71, 72],
            };
            for &id in ids {
                let mob = fixture_mob(game, id, family, output);
                game.mobs.push(mob);
            }
            if output == Output::Mixed {
                // A guest action already queued in an older batch must beat the egg produced by 73.
                game.creatures
                    .pending
                    .entry(game.dim.id())
                    .or_default()
                    .batches
                    .push_back(MobOutputBatch {
                        normal: NormalMobActions::default(),
                        guests: vec![(12, 71, MobEvent::AttackPlayer(2.0, [1.0, 80.0, 0.5]))],
                    });
            }
            game.last_hurt_by_player
                .insert(71, super::super::combat::PlayerCredit(99));
            let rows: Vec<_> = game.mobs.iter().map(|m| (m.id, m.join)).collect();
            VALUE
                .set(&mut game.world.attachments, Holder::Level, 42)
                .unwrap();
            game.world.attachment_syncs.insert(Holder::Level);
            let syncs = game.world.attachment_syncs.clone();
            game.world.path_writes = Some(Vec::new());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(4, 80, 4, stone).unwrap();
            let floor_depth = if output != Output::Quiet {
                5
            } else if family == Family::MountedGoal {
                3
            } else {
                0
            };
            for x in 0..5 {
                for z in 0..floor_depth {
                    game.world.set(x, 79, z, stone).unwrap();
                }
            }
            if matches!(family, Family::Movement | Family::Landing) {
                game.world.set(2, 79, 0, stone).unwrap();
            }
            let writes = game.world.path_writes.clone();
            assert!(writes.as_ref().is_some_and(|writes| !writes.is_empty()));
            game.world.random =
                sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(LEVEL_SEED));
            let guest_before = guests.then(|| guest_health(game));
            let push_before = game.mobs[0].velocity.x;
            let target = ids.iter().position(|&id| id == 72).unwrap();
            let health_before = game.mobs[target].health;

            let unwind = if family.is_goal() {
                // Registered goals keep their own unwind contract; this is not hook dispatch.
                Some(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick_in_level(game)))
                        .expect_err("native goal panic was swallowed"),
                )
            } else {
                tick_in_level(game).unwrap();
                None
            };
            assert_eq!(
                probe.calls.load(Ordering::SeqCst),
                1,
                "first callback entry"
            );
            if let Some(payload) = &unwind {
                assert!(
                    probe.posts.lock().unwrap().is_empty(),
                    "aborted tick fired Post"
                );
                let token = payload
                    .downcast_ref::<PanicToken>()
                    .expect("original typed payload");
                assert_eq!(Some(token as *const PanicToken as usize), payload_address);
                assert_eq!(token.0, 0xdecaf);
                assert_eq!(game.world.path_writes, writes);
            } else {
                assert_panic_report(FAILING_MOD, family.hook());
                assert_eq!(std::mem::take(&mut *probe.posts.lock().unwrap()), ids);
                let before = probe
                    .first_proposal
                    .lock()
                    .unwrap()
                    .expect("failing listener's input");
                let healthy = std::mem::take(&mut *probe.healthy_proposals.lock().unwrap());
                assert_eq!(
                    healthy.first(),
                    Some(&before),
                    "later callback did not see the last accepted proposal"
                );
                assert_eq!(
                    game.world.path_writes,
                    Some(Vec::new()),
                    "tick did not consume its path writes"
                );
                match family {
                    Family::Damage | Family::Shield | Family::Landing => {
                        assert!(
                            game.mobs[target].health < health_before,
                            "failed proposal stopped vanilla damage"
                        );
                    }
                    Family::Totem => {
                        assert!(
                            game.mobs[target].health > 0.0,
                            "failed proposal vetoed the totem"
                        );
                        assert!(
                            game.mobs[target].extra.main_hand_stack().is_none(),
                            "totem was not consumed"
                        );
                    }
                    _ => {}
                }
            }

            let mut pending = Vec::new();
            journal.drain_into(&mut pending);
            let (mut depth, mut lifts) = (0usize, 0usize);
            for e in pending {
                assert_eq!(e.owner, owner);
                match e.change {
                    Change::Lifted => {
                        depth += 1;
                        lifts += 1;
                    }
                    Change::Restored => depth = depth.checked_sub(1).unwrap(),
                    _ => {}
                }
                journal.push(e);
            }
            assert!(lifts > 0);
            assert_eq!(depth, 0, "borrowed region was not restored");
            assert_eq!(
                game.mobs.iter().map(|m| (m.id, m.join)).collect::<Vec<_>>(),
                rows
            );
            assert!(
                game.creatures.ticked.contains(&71),
                "earlier mob progress was lost"
            );
            let expected_value = if family.is_goal() || family == Family::Movement {
                42
            } else {
                314
            };
            assert_eq!(
                VALUE
                    .get_existing(&mut game.world.attachments, Holder::Level)
                    .unwrap(),
                Some(expected_value)
            );
            assert_eq!(game.last_hurt_by_player.get(&71).map(|c| c.0), Some(99));
            assert_eq!(game.world.attachment_syncs, syncs);
            let mut random = LegacyRandom::new(LEVEL_SEED);
            let draw = (expected_value == 314).then(|| random.next_long());
            assert_eq!(*probe.draw.lock().unwrap(), draw, "seeded callback draw");
            // The completed outputs don't advance this stream: egg loot uses its named
            // sequence, entity motion and sound pitches use their own streams, and the
            // directed guest hits and pig pushes need no random direction.
            assert_eq!(
                game.world.random.clone().next_long(),
                random.next_long(),
                "level RNG position after the first tick"
            );
            let mut table = EntityTable::new(journal);
            for &(id, join) in &rows {
                let found = table.entity_lookup(&owner.dimension, id, |at| {
                    let m = &game.mobs[game.mobs.locate(at.join)?];
                    Some(SourceEntity {
                        id: m.id,
                        value: m.join,
                    })
                });
                assert!(
                    matches!(found, EntityLookup::Found { location, value } if location.owner == owner && value == join)
                );
            }
            table.end_tick().unwrap();

            if unwind.is_some() {
                assert!(
                    game.mobs[1].ai.targets.is_running("BorrowedTarget"),
                    "actual native target selector was lost"
                );
                assert!(
                    game.mobs[1].ai.goals.is_running("BorrowedProbe"),
                    "actual native goal selector was lost"
                );
                if family == Family::MountedGoal {
                    assert_eq!(
                        game.mobs[1].ai.nav.speed_modifier, 3.75,
                        "private rider navigation not restored"
                    );
                    assert_eq!(
                        game.mobs[0].ai.nav.speed_modifier, 1.25,
                        "vehicle navigation lost requested progress (or routing never loaned)"
                    );
                    assert!(
                        game.mobs[0].ai.nav.path.is_some(),
                        "vehicle lost successful native path"
                    );
                    eprintln!(
                        "BORROWED_MOUNTED_RESTORED:{dim:?}:rider=3.75:vehicle=1.25:path=Some"
                    );
                }
                game.tick_creature_entities().unwrap();
                assert!(
                    !probe.posts.lock().unwrap().is_empty(),
                    "real post listener never ran on retry"
                );
                table.end_tick().unwrap();
                if family == Family::MountedGoal {
                    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
                    eprintln!("BORROWED_MOUNTED_RETRY_OK:{dim:?}");
                }
                assert_eq!(
                    VALUE
                        .get_existing(&mut game.world.attachments, Holder::Level)
                        .unwrap(),
                    Some(expected_value)
                );
                return;
            }

            let expected_outputs: &[OutputNotice] = match output {
                Output::Egg => &[OutputNotice::Egg],
                Output::Guest => &[OutputNotice::Guest(12)],
                Output::Mixed => &[OutputNotice::Guest(12), OutputNotice::Egg],
                _ => &[],
            };
            assert_eq!(probe.outputs.lock().unwrap().as_slice(), expected_outputs);
            let expected_eggs = u32::from(matches!(output, Output::Egg | Output::Mixed));
            assert_eq!(
                eggs(game),
                expected_eggs,
                "egg output after the returning tick"
            );
            if output == Output::Egg {
                assert!(
                    game.mobs[0].extra.egg_time > 1,
                    "laying did not advance the cooldown"
                );
            }
            let guest_after = guest_before.map(|[before12, before13]| {
                let [after12, after13] = guest_health(game);
                assert!(
                    after12 < before12,
                    "guest 12 did not take the hit on the returning tick"
                );
                assert_eq!(after13, before13, "the hit reached the wrong guest");
                game.mobs[0].no_ai = true;
                game.as_guest(12, |g| {
                    g.player.as_mut().unwrap().health.invulnerable_time = 0
                })
                .unwrap();
                [after12, after13]
            });
            if output == Output::Push {
                assert!(
                    game.mobs[0].velocity.x > push_before,
                    "completed push was lost"
                );
                game.mobs[1].no_ai = true;
                game.mobs[1].position.x = 8.0;
                // Isolate the old impulse from carry-over velocity and other contacts.
                game.mobs[0].velocity.x = 0.0;
                game.mobs[target].position.x = 4.0;
            }

            prime_hook(&mut game.mobs[target], family);
            tick_in_level(game).unwrap();
            assert_eq!(
                probe.calls.load(Ordering::SeqCst),
                1,
                "disabled callback ran again"
            );
            assert!(
                !probe.healthy_proposals.lock().unwrap().is_empty(),
                "same hook never reached the healthy callback again"
            );
            assert!(take_hook_panics().is_empty());
            assert_eq!(probe.posts.lock().unwrap().as_slice(), ids);
            table.end_tick().unwrap();
            assert_eq!(
                VALUE
                    .get_existing(&mut game.world.attachments, Holder::Level)
                    .unwrap(),
                Some(expected_value)
            );
            assert_eq!(
                eggs(game),
                expected_eggs,
                "egg replayed or survived clear_entities"
            );
            assert_eq!(
                probe.outputs.lock().unwrap().as_slice(),
                expected_outputs,
                "completed output replayed"
            );
            if let Some(health) = guest_after {
                assert_eq!(guest_health(game), health, "guest action replayed");
            }
            if output == Output::Push {
                assert_eq!(game.mobs[0].velocity.x, 0.0, "push replayed");
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn damage() {
    run_output("damage", Family::Damage, Output::Quiet);
}
#[test]
#[ignore = "daily"]
fn shield() {
    run_output("shield", Family::Shield, Output::Quiet);
}
#[test]
#[ignore = "daily"]
fn totem() {
    run_output("totem", Family::Totem, Output::Quiet);
}
#[test]
#[ignore = "daily"]
fn movement() {
    run_output("movement", Family::Movement, Output::Quiet);
}
#[test]
#[ignore = "daily"]
fn knockback() {
    run_output("knockback", Family::Knockback, Output::Quiet);
}
#[test]
#[ignore = "daily"]
fn landing() {
    run_output("landing", Family::Landing, Output::Quiet);
}

#[test]
#[ignore = "daily"]
fn goal() {
    run_output("goal", Family::Goal, Output::Quiet);
}
#[test]
#[ignore = "daily"]
fn mounted_goal() {
    run_output("mounted_goal", Family::MountedGoal, Output::Quiet);
}

#[test]
#[ignore = "daily"]
fn retained_egg() {
    run_output("retained_egg", Family::Damage, Output::Egg);
}
#[test]
#[ignore = "daily"]
fn retained_guest() {
    run_output("retained_guest", Family::Damage, Output::Guest);
}
#[test]
#[ignore = "daily"]
fn retained_mixed_order() {
    run_output("retained_mixed_order", Family::Damage, Output::Mixed);
}
#[test]
#[ignore = "daily"]
fn retained_push() {
    run_output("retained_push", Family::Damage, Output::Push);
}
#[test]
#[ignore = "daily"]
fn retained_reset() {
    run_output("retained_reset", Family::Damage, Output::Reset);
}

#[derive(Default)]
struct ConsumerProbe {
    calls: AtomicUsize,
    eggs: Mutex<Vec<u32>>,
    posts: Mutex<Vec<u32>>,
}

impl ConsumerProbe {
    fn assert_tick(&self, game: &Game<'_>, egg_count: u32) {
        assert_eq!(self.calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::mem::take(&mut *self.posts.lock().unwrap()), [71, 72]);
        assert_eq!(eggs(game), egg_count);
        let joined: Vec<_> = game
            .dropped
            .iter()
            .filter(|d| d.stack.item.as_ref() == "minecraft:egg")
            .map(|d| d.id)
            .collect();
        assert_eq!(joined.len(), egg_count as usize);
        assert!(
            joined
                .iter()
                .enumerate()
                .all(|(i, id)| !joined[..i].contains(id))
        );
        assert_eq!(
            *self.eggs.lock().unwrap(),
            joined,
            "healthy observer missed a joined egg"
        );
    }
}

struct Consumer {
    probe: Arc<ConsumerProbe>,
    panics: bool,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Consumer {
    fn item_spawn(&self, _: &mut dyn mod_api::ServerHost, event: &mut mod_api::ItemSpawnEvent<'_>) {
        if event.item != "minecraft:egg" {
            return;
        }
        if self.panics {
            self.probe.calls.fetch_add(1, Ordering::SeqCst);
            panic!("real item consumer");
        }
        self.probe.eggs.lock().unwrap().push(event.entity);
    }

    fn entity_tick_listened(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityTickListenedEvent,
    ) {
        if self.panics {
            event.listened = true;
        }
    }

    fn entity_tick_post(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        if self.panics {
            self.probe
                .posts
                .lock()
                .unwrap()
                .extend_from_slice(event.entities);
        }
    }
}

fn with_consumer_game(dim: Dim, test: impl FnOnce(&mut Game<'_>, &ConsumerProbe)) {
    crate::entry::install_client_hooks();
    let probe = Arc::new(ConsumerProbe::default());
    let scratch = tempfile::tempdir().unwrap();
    let mods = vec![
        mod_api::ModEntry::new(
            "consumer",
            Box::new(Consumer {
                probe: probe.clone(),
                panics: true,
            }),
        ),
        mod_api::ModEntry::new(
            "healthy",
            Box::new(Consumer {
                probe: probe.clone(),
                panics: false,
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.world_root = scratch.path().to_path_buf();
            game.enter_level(dim).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.load_server_loot();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game, &probe);
        });
    });
}

fn queue_egg(game: &mut Game<'_>) {
    game.creatures
        .pending
        .entry(game.dim.id())
        .or_default()
        .batches
        .push_back(MobOutputBatch {
            normal: NormalMobActions::new(vec![MobEvent::LayEgg {
                mob: 71,
                at: [1.0, 80.0, 0.5],
                pitch: 1.0,
            }]),
            guests: Vec::new(),
        });
}

#[test]
#[ignore = "daily"]
fn retained_consumer_tail() {
    let Some(dim) = fresh_dimension("retained_consumer_tail", BOTH_LEVELS) else {
        return;
    };
    with_consumer_game(dim, |game, probe| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:chicken")
            .unwrap()
            .clone();
        for id in [71, 72] {
            let mut mob = Mob::new(
                id,
                kind.clone(),
                Vec3::new(f64::from(id - 70), 80.0, 0.5),
                0.0,
            );
            mob.extra.egg_time = 1;
            game.mobs.push(mob);
        }
        game.tick_creature_entities().unwrap();
        assert_panic_report("consumer", "item_spawn");
        probe.assert_tick(game, 2);
        assert!(game.mobs.iter().all(|m| m.extra.egg_time > 1));

        // An ordinary tick neither loses nor replays either accepted egg.
        game.tick_creature_entities().unwrap();
        probe.assert_tick(game, 2);
        assert!(take_hook_panics().is_empty());

        game.mobs[0].extra.egg_time = 1;
        game.tick_creature_entities().unwrap();
        probe.assert_tick(game, 3);
        assert!(take_hook_panics().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn retained_conditional_context() {
    run_conditional("retained_conditional_context", false);
}

#[test]
#[ignore = "daily"]
fn retained_nested_tail() {
    run_conditional("retained_nested_tail", true);
}

fn run_conditional(test: &str, nested: bool) {
    let Some(dim) = fresh_dimension(test, &["overworld"]) else {
        return;
    };
    with_consumer_game(dim, |game, probe| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:chicken")
            .unwrap()
            .clone();
        for id in [71, 72] {
            let mut mob = Mob::new(
                id,
                kind.clone(),
                Vec3::new(f64::from(id - 70), 80.0, 0.5),
                0.0,
            );
            mob.no_ai = true;
            game.mobs.push(mob);
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(4, 80, 4, stone).unwrap();
        let conditional = || MobEvent::IfBroke {
            mob: 71,
            effects: vec![MobEvent::Jump { mob: 71 }],
            otherwise: vec![MobEvent::Jump { mob: 72 }],
        };
        let egg = MobEvent::LayEgg {
            mob: 71,
            at: [1.0, 80.0, 0.5],
            pitch: 1.0,
        };
        let mut events = vec![MobEvent::BreakBlock {
            pos: [4, 80, 4],
            drop: false,
            by: Some(71),
            breaker: None,
        }];
        if nested {
            game.world.set(4, 80, 3, stone).unwrap();
            events.push(MobEvent::IfBroke {
                mob: 71,
                effects: vec![
                    MobEvent::BreakBlock {
                        pos: [4, 80, 3],
                        drop: false,
                        by: Some(72),
                        breaker: None,
                    },
                    egg,
                    MobEvent::IfBroke {
                        mob: 72,
                        effects: vec![MobEvent::Jump { mob: 71 }],
                        otherwise: Vec::new(),
                    },
                ],
                otherwise: Vec::new(),
            });
        } else {
            events.extend([egg, conditional()]);
        }
        // The same frame keeps its completed breaks; a new batch does not inherit them.
        let batch = game.creatures.pending.entry(game.dim.id()).or_default();
        batch.batches.push_back(MobOutputBatch {
            normal: NormalMobActions::new(events),
            guests: Vec::new(),
        });
        batch.batches.push_back(MobOutputBatch {
            normal: NormalMobActions::new(vec![conditional()]),
            guests: Vec::new(),
        });

        game.tick_creature_entities().unwrap();
        assert_panic_report("consumer", "item_spawn");
        probe.assert_tick(game, 1);
        assert_eq!(game.world.block(4, 80, 4), game.world.air);
        if nested {
            assert_eq!(game.world.block(4, 80, 3), game.world.air);
        }
        assert!(
            game.mobs[0].ai.jumping,
            "same frame lost its completed break"
        );
        assert!(
            game.mobs[1].ai.jumping,
            "fresh batch inherited an earlier break"
        );

        for mob in &mut game.mobs {
            mob.ai.jumping = false;
        }
        queue_egg(game);
        game.tick_creature_entities().unwrap();
        probe.assert_tick(game, 2);
        assert!(
            game.mobs.iter().all(|m| !m.ai.jumping),
            "completed conditional replayed"
        );
        assert!(take_hook_panics().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn retained_stale_push_target() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("sunrise-stale-push-{}", std::process::id())));
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.world_root = scratch.0.clone();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.creatures
            .pending
            .entry(game.dim.id())
            .or_default()
            .pushes
            .push(999, Vec3::new(0.25, 0.0, 0.0));
        game.tick_creature_entities().unwrap();
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut mob = Mob::new(999, kind, Vec3::new(1.0, 80.0, 0.5), 0.0);
        mob.no_ai = true;
        game.mobs.push(mob);
        game.tick_creature_entities().unwrap();
        assert_eq!(
            game.mobs[0].velocity.x, 0.0,
            "obsolete target push reached a later occupant"
        );
    });
}

/// `living_breathe` with the owning host: a listener reads the mob's air and edits what it holds
/// through a transaction, and the game applies its answer to the air it had decided on, or
/// refuses when the listener changed that air and rolls the transactional edit back.
struct Breather {
    meddle: bool,
    seen_air: Arc<std::sync::Mutex<Vec<i32>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Breather {
    fn living_breathe(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingBreatheEvent,
    ) {
        use mod_api::HostTransaction;
        if event.entity.id() != 72 {
            return;
        }
        let entity = event.entity.entity();
        self.seen_air
            .lock()
            .unwrap()
            .push(entity.air_supply(host).unwrap());
        let place = mod_api::ItemPlace::Equipment {
            entity: 72,
            slot: mod_api::EquipmentSlot::MainHand,
        };
        host.transaction(|host, tx| place.edit(host, tx, |stack| stack.set_damage(5)))
            .unwrap();
        if self.meddle {
            assert!(entity.set_air_supply(host, 50));
        }
        event.can_breathe = false;
        event.consume = 7;
    }
}

/// One tick of a zombie with 100 air and an iron sword: the air it listened to, then the air
/// and the sword's damage the tick left.
fn breathing_tick(meddle: bool) -> (Vec<i32>, i32, i32) {
    crate::entry::install_client_hooks();
    let _ =
        sunrise_client::block_contact::HOOKS.set(crate::game::native_hooks::BLOCK_CONTACT_HOOKS);
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "sunrise-breathing-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    )));
    let seen_air = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut left = (0, 0);
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "breathing",
            Box::new(Breather {
                meddle,
                seen_air: seen_air.clone(),
            }),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.world_root = scratch.0.clone();
                game.enter_level(Dim::Overworld).unwrap();
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                assert!(game.entity_ticking_area()(1, 0));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(72, kind, Vec3::new(2.0, 80.0, 0.5), 0.0);
                mob.no_ai = true;
                mob.air = 100;
                mob.extra
                    .set_main_hand_stack(Some(sunrise_client::items::ItemStack::new(
                        "minecraft:iron_sword",
                        1,
                    )));
                game.mobs.push(mob);
                game.tick_creature_entities().unwrap();
                let mob = game.mobs.iter().find(|m| m.id == 72).unwrap();
                left = (mob.air, mob.extra.main_hand_stack().unwrap().damage);
            });
        },
    );
    let seen = seen_air.lock().unwrap().clone();
    (seen, left.0, i32::try_from(left.1).unwrap())
}

#[test]
#[ignore = "daily"]
fn a_mob_breathes_with_what_its_listener_answers_and_wrote() {
    // Vanilla would raise the 100 air by 4; the listener took 7 and damaged its sword.
    assert_eq!(breathing_tick(false), (vec![100], 93, 5));
}

#[test]
#[ignore = "daily"]
fn a_mob_whose_air_the_listener_changed_keeps_it_and_loses_the_transactional_writes() {
    // The decision read 100; the listener left 50, so the answer is refused rather than
    // recomputed (not 43, not 54), and the sword's edit is undone.
    assert_eq!(breathing_tick(true), (vec![100], 50, 0));
}

/// A listener that edits the mob's sword in a transaction and then panics in `living_breathe`
/// without touching `air`: under the unified failure rule, the panic is contained, the enclosing
/// transaction aborts (rolling back the sword edit and leaving air unchanged), and the listener is
/// disabled for `living_breathe` for the rest of the session so subsequent ticks run vanilla.
struct BreathPanic(Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for BreathPanic {
    fn living_breathe(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingBreatheEvent,
    ) {
        use mod_api::HostTransaction;
        if event.entity.id() != 72 {
            return;
        }
        self.0.fetch_add(1, Ordering::SeqCst);
        let place = mod_api::ItemPlace::Equipment {
            entity: 72,
            slot: mod_api::EquipmentSlot::MainHand,
        };
        host.transaction(|host, tx| place.edit(host, tx, |stack| stack.set_damage(5)))
            .unwrap();
        panic!("real breathing listener");
    }
}
#[test]
#[ignore = "daily"]
fn retained_breathing_tail() {
    crate::entry::install_client_hooks();
    let _ =
        sunrise_client::block_contact::HOOKS.set(crate::game::native_hooks::BLOCK_CONTACT_HOOKS);
    let calls = Arc::new(AtomicUsize::new(0));
    let scratch =
        Scratch(std::env::temp_dir().join(format!("sunrise-breath-panic-{}", std::process::id())));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "breath-panic",
            Box::new(BreathPanic(calls.clone())),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.world_root = scratch.0.clone();
                game.enter_level(Dim::Overworld).unwrap();
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(72, kind, Vec3::new(2.0, 80.0, 0.5), 0.0);
                mob.no_ai = true;
                mob.air = 100;
                mob.extra
                    .set_main_hand_stack(Some(sunrise_client::items::ItemStack::new(
                        "minecraft:iron_sword",
                        1,
                    )));
                game.mobs.push(mob);
                game.tick_creature_entities().unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                let mob = game.mobs.iter().find(|m| m.id == 72).unwrap();
                assert_eq!(mob.air, 100);
                assert_eq!(mob.extra.main_hand_stack().unwrap().damage, 0);
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "breath-panic",
                        hook: "living_breathe",
                    }]
                );
                // Next tick skips the poisoned listener and applies vanilla breathing (+4 air).
                game.tick_creature_entities().unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                let mob = game.mobs.iter().find(|m| m.id == 72).unwrap();
                assert_eq!(mob.air, 104);
                assert_eq!(mob.extra.main_hand_stack().unwrap().damage, 0);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn player_living_breathe_panic_rolls_back_transactional_writes() {
    struct PlayerBreathPanic {
        player: u128,
        calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PlayerBreathPanic {
        fn living_breathe(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::LivingBreatheEvent,
        ) {
            use mod_api::HostTransaction;
            let place = mod_api::ItemPlace::PlayerHand {
                player: self.player,
                hand: mod_api::client::first_person::Hand::Main,
            };
            host.transaction(|host, tx| place.edit(host, tx, |stack| stack.set_damage(5)))
                .unwrap();
            assert_eq!(place.with_stack(host, |stack| stack.damage()), Some(5));
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("player breathing listener");
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    crate::game::native_guest_turn_tests::with_guests(|game| {
        game.as_guest(12, |game| {
            game.player.as_mut().unwrap().air = 100;
            game.selected = 0;
            game.inventory[0] = Some(sunrise_client::items::ItemStack::new(
                "minecraft:iron_sword",
                1,
            ));
            let step = sunrise_client::breathing::AirStep {
                air: 100,
                max: 300,
                vanilla: sunrise_client::breathing::Breath {
                    can_breathe: true,
                    consume: 0,
                    refill: 4,
                },
                own: false,
            };
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "player-breath-panic",
                    Box::new(PlayerBreathPanic {
                        player: game.native_acting_player().unwrap().uuid(),
                        calls: calls.clone(),
                    }),
                )]
                .into(),
                || {
                    assert!(game.ask_player_breath(step).is_none());
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    assert_eq!(game.player.as_ref().unwrap().air, 100);
                    assert_eq!(game.inventory[0].as_ref().unwrap().damage, 0);
                    assert_eq!(
                        mod_api::__private::take_hook_panics(),
                        [mod_api::__private::HookPanicReport {
                            mod_id: "player-breath-panic",
                            hook: "living_breathe",
                        }]
                    );
                    // Second call skips the poisoned listener and accepts the vanilla breath.
                    assert_eq!(game.ask_player_breath(step), Some(step.vanilla));
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    assert_eq!(game.inventory[0].as_ref().unwrap().damage, 0);
                },
            );
        })
        .unwrap();
    });
}
