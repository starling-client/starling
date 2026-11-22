use super::*;
use mod_api::{BlockPart, FallOnEvent, Hooks, LevelRandomHost, ModEntry, Registrar};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};

/// What the part on stone heard of a landing: entity, kind, block, fall and damage. It
/// asks for the block to be destroyed.
#[derive(Clone, Default)]
struct Lands(Arc<Mutex<Vec<(i32, String, BlockPos, f64, f32)>>>);
#[mod_api::sunrise_part]
impl BlockPart for Lands {
    fn fall_on(&self, _: &mut dyn LevelRandomHost, event: &mut FallOnEvent<'_>) {
        self.0.lock().unwrap().push((
            event.entity,
            event.kind.to_owned(),
            event.pos,
            event.distance,
            event.damage_multiplier,
        ));
        event.request(mod_api::LandingWrite::DestroyBlock { drop: false });
    }
}
struct Mod(Lands);
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let block_key = r.block_key("minecraft:stone");
        r.block_part(block_key, self.0.clone());
    }
}

static LOAN_VALUE: mod_api::AttachmentType<i32> = mod_api::AttachmentType {
    namespace: "loan_test",
    name: "progress",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

use sunrise_core::random::RandomSource as _;

struct LoanPart(std::sync::Arc<()>);
#[mod_api::sunrise_part]
impl BlockPart for LoanPart {
    fn fall_on(&self, host: &mut dyn LevelRandomHost, event: &mut FallOnEvent<'_>) {
        let draw = host
            .with_random(|random| random.next_int())
            .expect("authoritative landing random");
        assert_eq!(draw, 1553932502);
        LOAN_VALUE
            .set(host.attachments(), mod_api::Holder::Level, 73)
            .unwrap();
        event.request(mod_api::LandingWrite::DestroyBlock { drop: false });
        std::panic::panic_any(self.0.clone());
    }
}
struct LoanMod(std::sync::Arc<()>);
#[mod_api::__subscriptions]
impl Hooks for LoanMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.register_attachment_type("loan_test:progress", false)
            .unwrap();
        let key = r.block_key("minecraft:stone");
        r.block_part(key, LoanPart(self.0.clone()));
    }
}

/// Exercise the actual dimension loan and the dispatcher in both dimensions. Each
/// fixture installs a fresh session so the first panic does not skip the second case.
#[test]
#[ignore = "daily"]
fn item_fall_on_returns_progressed_level_loan_after_part_panic() {
    let payload = std::sync::Arc::new(());
    let mods: mod_api::Mods = vec![ModEntry::new(
        "loan_test",
        Box::new(LoanMod(payload.clone())),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    let _ = sunrise_client::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
    let mut loaded = 0;
    for dimension in [crate::game::Dim::Overworld, crate::game::Dim::Nether] {
        let scratch =
            std::env::temp_dir().join(format!("item-loan-{}-{dimension:?}", std::process::id()));
        with_mods(mods.clone(), || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.world_root = scratch.clone();
                game.enter_level(dimension).unwrap();
                loaded += 1;
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let pos = BlockPos::new(1, 64, 1);
                game.world.set(pos.x, pos.y, pos.z, stone).unwrap();
                *game.world.random = sunrise_core::random::LegacyRandom::new(12345);
                LOAN_VALUE
                    .set(&mut game.world.attachments, mod_api::Holder::Level, 11)
                    .unwrap();
                game.world.attachment_syncs.insert(mod_api::Holder::Level);
                let land = |game: &mut crate::game::Game<'_>| {
                    sunrise_client::block_contact::in_level(
                        sunrise_client::parallel::TickThread::mint(),
                        Some(game.dim.id()),
                        || game.item_fall_on(42, pos, 5.0),
                    );
                };
                land(game);
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "loan_test",
                        hook: "fall_on",
                    }],
                );
                land(game);
                assert!(mod_api::__private::take_hook_panics().is_empty());
                assert_eq!(
                    game.world.random.next_int(),
                    -2090749135,
                    "the progressed stream is returned and the disabled Part draws no more"
                );
                assert_eq!(
                    LOAN_VALUE
                        .get(&mut game.world.attachments, mod_api::Holder::Level)
                        .unwrap(),
                    73
                );
                assert!(
                    game.world
                        .attachment_syncs
                        .contains(&mod_api::Holder::Level)
                );
                game.drain_entity_inside();
                assert_eq!(
                    game.world.block(pos.x, pos.y, pos.z),
                    stone,
                    "aborted landing writes were not queued"
                );
            });
        });
        let _ = std::fs::remove_dir_all(scratch);
    }
    assert_eq!(loaded, 2, "both resource fixtures ran");
}

/// A dropped item that falls onto a block with a `fall_on` part is told to it once, as
/// `minecraft:item`, with the whole fall and no damage; the write the part asks for lands
/// after the items' batch; and an item resting there lands no more.
#[test]
#[ignore = "daily"]
fn a_dropped_item_lands_on_a_fall_on_part() {
    let heard = Lands::default();
    let mods: mod_api::Mods = vec![ModEntry::new("lands", Box::new(Mod(heard.clone())))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    let _ = sunrise_client::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
    with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let (stone, air) = (state("minecraft:stone"), state("minecraft:air"));
            for x in -1..=1 {
                for z in -1..=1 {
                    let mut chunk = sunrise_client::chunk::Chunk::empty(x, z);
                    chunk.lit = true;
                    game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
                }
            }
            game.world.set(1, 64, 1, stone).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let stack = ItemStack::new("minecraft:apple", 1);
            let item = game
                .level_spawn_item(level, 1.5, 70.0, 1.5, stack, 10)
                .unwrap();
            if let Some(i) = game.dropped.iter_mut().find(|i| i.id == item.id()) {
                i.velocity = sunrise_core::geometry::Vec3::ZERO;
            }
            // The part's write destroys the stone after the batch; it is put back at once,
            // so the item rests on it again and must not be heard landing twice.
            let mut destructions = 0;
            for _ in 0..80 {
                game.ticked_entities_through = u32::MAX;
                sunrise_client::block_contact::in_level(
                    sunrise_client::parallel::TickThread::mint(),
                    Some(game.dim.id()),
                    || {
                        game.tick_items();
                    },
                );
                if game.world.block(1, 64, 1) == air {
                    destructions += 1;
                    game.world.set(1, 64, 1, stone).unwrap();
                    let heard = heard.0.lock().unwrap();
                    assert_eq!(heard.len(), 1, "the write landed after the batch");
                }
            }
            assert_eq!(destructions, 1, "one observed landing-write destruction");
            let heard = heard.0.lock().unwrap().clone();
            assert_eq!(heard.len(), 1, "one landing: {heard:?}");
            let (entity, kind, pos, distance, damage) = &heard[0];
            assert_eq!(*entity, item.id() as i32);
            assert_eq!(kind, "minecraft:item");
            assert_eq!(*pos, BlockPos::new(1, 64, 1));
            // From rest at y 70 onto the stone's top at 65.
            assert!((distance - 5.0).abs() < 1e-3, "fell {distance}");
            assert_eq!(*damage, 0.0);
        });
    });
}
