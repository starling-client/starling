#![cfg(test)]
use super::Game;
use super::item_abilities::ability;
use mod_api::{
    CanPerformActionEvent, Hooks, Identifier, ItemAbilityContext, ItemPart, Registrar,
    RegistrationContext,
};
use std::sync::Arc;
use sunrise_client::items::ItemStack;
use sunrise_client::mobs::Mob;
use sunrise_core::geometry::{Direction, Vec3};
use sunrise_core::use_animation::UseAnimation;

/// A part of `item` that grants the abilities in `grants` and denies the ones in `denies`.
#[derive(Clone)]
struct Abilities {
    item: &'static str,
    grants: &'static [&'static str],
    denies: &'static [&'static str],
}

#[mod_api::sunrise_part]
impl ItemPart for Abilities {
    fn can_perform_action(
        &self,
        context: &ItemAbilityContext<'_>,
        event: &mut CanPerformActionEvent<'_>,
    ) {
        let expected_key = event.stack.item();
        assert_eq!(
            mod_api::__private::item_key_id(expected_key),
            Some(self.item)
        );
        let swords = context
            .tags()
            .key(&Identifier::parse("minecraft:swords").unwrap())
            .expect("swords tag resolves");
        assert_eq!(
            context.tags().is_in_tag(expected_key, swords.typed()),
            Some(self.item == "minecraft:diamond_sword")
        );
        if self.item == "minecraft:shears" && SHEARING.contains(&event.ability) {
            assert!(event.can_perform);
        }
        if self.item == "minecraft:shield" && event.ability == ability::SHIELD_BLOCK {
            assert!(event.can_perform);
        }
        if self.grants.contains(&event.ability) {
            assert!(!event.can_perform);
            event.can_perform = true;
        } else if self.denies.contains(&event.ability) {
            event.can_perform = false;
        }
    }
}

struct Mod(Vec<Abilities>);

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for part in &self.0 {
            let key = r.item_key(part.item);
            r.item_part(key, part.clone());
        }
    }
}

/// A serving level with one chunk of air and a pumpkin at (4, 64, 4), its world locked as an open
/// world's is, so its merged tags are readable. The folder holds the lock; keep it for the test.
fn field(game: &mut Game<'_>) -> tempfile::TempDir {
    game.options.dedicated = Some(Vec::new());
    let folder = tempfile::tempdir().unwrap();
    game.session_lock = Some(sunrise_client::save::SessionLock::acquire(folder.path()).unwrap());
    game.native_lifecycle
        .begin_open(Some(mod_api::Authority::IntegratedServer));
    game.world = sunrise_client::world::World::new(game.res.facts().air, 1);
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
    let pumpkin = game
        .pack
        .assets
        .registry
        .default_state("minecraft:pumpkin")
        .unwrap();
    game.world.set(4, 64, 4, pumpkin).unwrap();
    folder
}

fn block_name(game: &Game<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

fn add_sheep(game: &mut Game<'_>, x: f64) -> usize {
    let kind = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
    game.next_entity += 1;
    game.mobs.push(Mob::new(
        game.next_entity,
        kind,
        Vec3::new(x, 64.0, 0.5),
        0.0,
    ));
    game.mobs.len() - 1
}

/// Right click on the sheep at `index` holding `stack`.
fn click_sheep(game: &mut Game<'_>, index: usize, stack: &ItemStack) -> bool {
    game.inventory[game.selected] = Some(stack.clone());
    let id = game.mobs[index].id as i32;
    game.interact_mob(Some(crate::game::InteractTarget {
        id,
        location: [0.0; 3],
    }))
    .unwrap()
}

const SHEARING: &[&str] = &[ability::SHEARS_HARVEST, ability::SHEARS_CARVE];

/// Parts that grant a sword `shield_block`, a stick the shears' two abilities, and take
/// both away from the shears and `shield_block` from the shield.
#[test]
#[ignore = "daily"]
fn parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item() {
    crate::game::in_own_process(
        "game::item_abilities_tests::parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item",
        parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item_isolated,
    );
}

fn parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item_isolated() {
    use crate::game::native_entity_refs::tests::with_game;
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "abilities",
        Box::new(Mod(vec![
            Abilities {
                item: "minecraft:diamond_sword",
                grants: &[ability::SHIELD_BLOCK],
                denies: &[],
            },
            Abilities {
                item: "minecraft:stick",
                grants: &[ability::SHEARS_CARVE],
                denies: &[],
            },
            Abilities {
                item: "minecraft:arrow",
                grants: &[ability::SHEARS_HARVEST],
                denies: &[ability::SHEARS_CARVE],
            },
            Abilities {
                item: "minecraft:shears",
                grants: &[],
                denies: SHEARING,
            },
            Abilities {
                item: "minecraft:shield",
                grants: &[],
                denies: &[ability::SHIELD_BLOCK],
            },
        ])),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            let _world = field(game);
            let sword = ItemStack::new("minecraft:diamond_sword", 1);
            let axe = ItemStack::new("minecraft:diamond_axe", 1);
            let shield = ItemStack::new("minecraft:shield", 1);

            // A sword the part lets block blocks as a bare `blocks_attacks` component does (no
            // block delay, one reduction of everything within 90 degrees), is held up for as
            // long as a shield, and shows the blocking pose.
            let granted = game
                .blocks_attacks(&sword)
                .expect("the part grants blocking");
            let again = game.blocks_attacks(&sword).unwrap();
            assert!(std::ptr::eq(granted.as_ref(), again.as_ref()));
            assert_eq!(granted.delay_seconds, 0.0);
            assert_eq!(granted.reductions.len(), 1);
            assert_eq!(granted.reductions[0].horizontal_angle, 90.0);
            drop(granted);
            assert_eq!(game.use_duration(&sword), 72_000);
            assert_eq!(game.item_use_animation(&sword), UseAnimation::Block);
            // An axe no part is attached to blocks nothing.
            assert!(game.blocks_attacks(&axe).is_none());
            assert_eq!(game.use_duration(&axe), 0);
            assert_eq!(game.item_use_animation(&axe), UseAnimation::None);
            // A shield whose part denies the ability no longer blocks nor is raised.
            assert!(game.blocks_attacks(&shield).is_none());
            assert_eq!(game.use_duration(&shield), 0);
            game.inventory[game.selected] = Some(shield.clone());
            assert!(!game.raise_shield(game.selected));
            game.inventory[game.selected] = Some(sword.clone());
            assert!(game.raise_shield(game.selected));

            // Shears carve a pumpkin only when the ability is not denied: a stick the part lets
            // carve does, and an item no part is attached to never does.
            let carve = |game: &mut Game<'_>, item: &str| {
                let held = ItemStack::new(item, 1);
                let carved = game
                    .use_pumpkin([4, 64, 4], [0, 1, 0], Some(&held))
                    .unwrap();
                (carved, block_name(game, [4, 64, 4]))
            };
            let pumpkin = "minecraft:pumpkin".to_string();
            assert_eq!(carve(game, "minecraft:arrow"), (false, pumpkin.clone()));
            assert_eq!(carve(game, "minecraft:shears"), (false, pumpkin.clone()));
            assert_eq!(carve(game, "minecraft:blaze_rod"), (false, pumpkin));
            assert_eq!(
                carve(game, "minecraft:stick"),
                (true, "minecraft:carved_pumpkin".to_string())
            );

            // A sheep, through a right click: the same.
            let stick = ItemStack::new("minecraft:arrow", 1);
            let shears = ItemStack::new("minecraft:shears", 1);
            let rod = ItemStack::new("minecraft:blaze_rod", 1);
            let sheep = add_sheep(game, 8.5);
            assert!(!click_sheep(game, sheep, &shears));
            assert!(!game.mobs[sheep].extra.sheared);
            assert!(!click_sheep(game, sheep, &rod));
            assert!(!game.mobs[sheep].extra.sheared);
            assert!(click_sheep(game, sheep, &stick));
            assert!(game.mobs[sheep].extra.sheared);

            // A dispenser at (7, 64, 0) facing a sheep at (8, 64, 0): the denied shears and the
            // rod are dropped instead of shearing it, the stick shears it.
            let sheep = add_sheep(game, 8.5);
            for held in [&shears, &rod] {
                let _ = game
                    .dispense_item((7, 64, 0), Direction::East, held.clone())
                    .unwrap();
                assert!(!game.mobs[sheep].extra.sheared);
            }
            let _ = game
                .dispense_item((7, 64, 0), Direction::East, stick.clone())
                .unwrap();
            assert!(game.mobs[sheep].extra.sheared);
        });
    });
}

/// With no part attached to any item, vanilla's answers stand: a shield blocks after its
/// quarter second, shears carve, shear and are dispensed as shears, nothing else does.
#[test]
#[ignore = "daily"]
fn without_parts_the_vanilla_item_checks_stand() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let _world = field(game);
        let shield = ItemStack::new("minecraft:shield", 1);
        let granted = game.blocks_attacks(&shield).expect("a shield blocks");
        assert_eq!(granted.delay_seconds, 0.25);
        drop(granted);
        assert_eq!(game.use_duration(&shield), 72_000);
        let sword = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(game.blocks_attacks(&sword).is_none());
        assert_eq!(game.use_duration(&sword), 0);

        let stick = ItemStack::new("minecraft:stick", 1);
        let shears = ItemStack::new("minecraft:shears", 1);
        assert!(
            game.use_pumpkin([4, 64, 4], [0, 1, 0], Some(&stick))
                .is_ok_and(|carved| !carved)
        );
        assert_eq!(block_name(game, [4, 64, 4]), "minecraft:pumpkin");
        assert!(
            game.use_pumpkin([4, 64, 4], [0, 1, 0], Some(&shears))
                .unwrap()
        );
        assert_eq!(block_name(game, [4, 64, 4]), "minecraft:carved_pumpkin");

        let sheep = add_sheep(game, 8.5);
        assert!(!click_sheep(game, sheep, &stick));
        assert!(!game.mobs[sheep].extra.sheared);
        assert!(click_sheep(game, sheep, &shears));
        assert!(game.mobs[sheep].extra.sheared);
        let sheep = add_sheep(game, 8.5);
        let _ = game
            .dispense_item((7, 64, 0), Direction::East, stick)
            .unwrap();
        assert!(!game.mobs[sheep].extra.sheared);
        let _ = game
            .dispense_item((7, 64, 0), Direction::East, shears)
            .unwrap();
        assert!(game.mobs[sheep].extra.sheared);
    });
}

/// The abilities [`Lights`] heard `block_tool_modification` ask, and whether a player acted.
static HEARD: std::sync::Mutex<Vec<(mod_api::ToolAbility, bool)>> =
    std::sync::Mutex::new(Vec::new());
/// What `host.item_abilities()` answered [`Lights`].
static LISTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// Where [`Lights`] takes every tool's answer away.
const REFUSED: [i32; 3] = [6, 64, 8];

/// Parts as [`Mod`] attaches them, and a `block_tool_modification` listener that records
/// the ability and keeps the block at [`REFUSED`] as it is.
struct Lights(Vec<Abilities>);

#[mod_api::__subscriptions]
impl Hooks for Lights {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for part in &self.0 {
            let key = r.item_key(part.item);
            r.item_part(key, part.clone());
        }
    }
    fn block_tool_modification(
        &self,
        host: &dyn mod_api::Host,
        event: &mut mod_api::BlockToolModificationEvent<'_>,
    ) -> mod_api::Flow {
        HEARD
            .lock()
            .unwrap()
            .push((event.tool.ability, event.tool.player.is_some()));
        *LISTED.lock().unwrap() = host.item_abilities();
        if event.tool.pos == REFUSED {
            *event.final_state = None;
        }
        mod_api::Flow::Continue
    }
}

/// A grounded player with a full charge at (0.5, 64, 0.5), its speed the base
/// `movement_speed` its first tick would set, so standing still passes the sweep's speed bound.
fn grounded(game: &mut Game<'_>) {
    let start = Vec3::new(0.5, 64.0, 0.5);
    let mut player = sunrise_client::motion::PlayerMotion::new(game.motion, start).unwrap();
    player.ground.on_ground = true;
    player.speed = 0.1;
    game.player = Some(player);
}

/// A part granting a blaze rod `sword_sweep` and `shears_trim` sweeps and trims with it, one
/// denying a diamond sword `sword_sweep` stops its sweep on the server and the client, a fire
/// starter lighting a campfire by hand or from a dispenser asks `block_tool_modification` with
/// `FirestarterLight`, a fire charge denied `firestarter_light` lights nothing, and the host
/// lists the fourteen abilities.
#[test]
#[ignore = "daily"]
fn parts_answer_the_abilities_at_every_site() {
    crate::game::in_own_process(
        "game::item_abilities_tests::parts_answer_the_abilities_at_every_site",
        parts_answer_the_abilities_at_every_site_isolated,
    );
}

fn parts_answer_the_abilities_at_every_site_isolated() {
    use crate::game::native_entity_refs::tests::{with_client_game, with_game};
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "abilities",
        Box::new(Lights(vec![
            Abilities {
                item: "minecraft:blaze_rod",
                grants: &[ability::SWORD_SWEEP, ability::SHEARS_TRIM],
                denies: &[],
            },
            Abilities {
                item: "minecraft:diamond_sword",
                grants: &[],
                denies: &[ability::SWORD_SWEEP],
            },
            Abilities {
                item: "minecraft:fire_charge",
                grants: &[],
                denies: &[ability::FIRESTARTER_LIGHT],
            },
        ])),
    )]
    .into();
    let sweeps = |game: &mut Game<'_>| {
        grounded(game);
        [
            "minecraft:blaze_rod",
            "minecraft:diamond_sword",
            "minecraft:iron_sword",
        ]
        .map(|item| {
            game.inventory[game.selected] = Some(ItemStack::new(item, 1));
            game.sweep_decision(1.0, false, "minecraft:zombie")
        })
    };
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_client_game(|client| {
            assert_eq!(
                sweeps(client),
                [true, false, true],
                "the client predicts the sweep"
            );
        });
        with_game(|game| {
            field(game);
            assert_eq!(sweeps(game), [true, false, true], "the server sweeps");

            // A blaze rod crops a cave vine's tip to its top age; a stick does nothing.
            let registry = game.pack.assets.registry;
            let vine = registry.default_state("minecraft:cave_vines").unwrap();
            game.world.set(2, 64, 2, vine).unwrap();
            let stick = ItemStack::new("minecraft:stick", 1);
            assert!(!game.shears_crop([2, 64, 2], &stick).unwrap());
            let rod = ItemStack::new("minecraft:blaze_rod", 1);
            assert!(game.shears_crop([2, 64, 2], &rod).unwrap());
            let cropped = registry.state(game.world.block(2, 64, 2)).unwrap();
            assert_eq!(cropped.properties["age"], "25");

            // Flint and steel lights an unlit campfire through `block_tool_modification`; the
            // listener takes the answer away for the campfire at `REFUSED`.
            let mut campfire = registry
                .state(registry.default_state("minecraft:campfire").unwrap())
                .unwrap()
                .clone();
            campfire.properties.insert("lit".into(), "false".into());
            let unlit = registry.resolve(&campfire).unwrap();
            let hit = |pos: [i32; 3]| crate::game::Hit {
                pos,
                face: [0, 1, 0],
                point: [6.5, 65.0, 6.5],
                inside: false,
            };
            for pos in [[6, 64, 6], REFUSED] {
                game.world.set(pos[0], pos[1], pos[2], unlit).unwrap();
            }
            HEARD.lock().unwrap().clear();
            let flint = ItemStack::new("minecraft:flint_and_steel", 1);
            let lit = game
                .fire_starter_lit(&hit([6, 64, 6]), &flint, true)
                .unwrap()
                .unwrap();
            assert_eq!(registry.state(lit).unwrap().properties["lit"], "true");
            assert_eq!(
                game.fire_starter_lit(&hit(REFUSED), &flint, true).unwrap(),
                None
            );
            // Denied `firestarter_light`, a fire charge lights nothing; the listener still hears.
            let charge = ItemStack::new("minecraft:fire_charge", 1);
            assert_eq!(
                game.fire_starter_lit(&hit([6, 64, 6]), &charge, true)
                    .unwrap(),
                None
            );
            // A dispenser's flint and steel lights through the same hook, with no player.
            game.world.set(10, 64, 6, unlit).unwrap();
            let _ = game
                .dispense_item((9, 64, 6), Direction::East, flint)
                .unwrap();
            let dispensed = registry.state(game.world.block(10, 64, 6)).unwrap();
            assert_eq!(dispensed.properties["lit"], "true");
            let light = mod_api::ToolAbility::FirestarterLight;
            assert_eq!(
                *HEARD.lock().unwrap(),
                [(light, true), (light, true), (light, true), (light, false)]
            );

            assert_eq!(
                *LISTED.lock().unwrap(),
                [
                    "brush_brush",
                    "firestarter_light",
                    "fishing_rod_cast",
                    "shears_carve",
                    "shears_dig",
                    "shears_disarm",
                    "shears_harvest",
                    "shears_remove_armor",
                    "shears_trim",
                    "shield_block",
                    "shovel_douse",
                    "spyglass_scope",
                    "sword_sweep",
                    "trident_throw",
                ]
            );
        });
    });
}
