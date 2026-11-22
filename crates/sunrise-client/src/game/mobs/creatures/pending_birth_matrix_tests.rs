//! Test pending children through real splitting and breeding with a native listener.

use super::pending_birth_tests::{SEED, YAWS, breed_pair, cube, find, near};
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{BabyEntitySpawnEvent, Flow, Hooks, MobSplitEvent, ServerHost};

#[path = "pending_birth_matrix_tests/fixture.rs"]
mod fixture;

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn mob_split(&self, host: &mut dyn ServerHost, event: &mut MobSplitEvent<'_>) -> Flow {
        fixture::mob_split(host, event)
    }
    fn baby_entity_spawn(
        &self,
        host: &mut dyn ServerHost,
        event: &mut BabyEntitySpawnEvent<'_>,
    ) -> Flow {
        fixture::baby_entity_spawn(host, event)
    }
}

/// A size-4 slime whose entity id ends in `mode`, split by the real route.
fn split(game: &mut Game<'_>, mode: u32) -> Vec<Mob> {
    game.mobs.clear();
    let mut dying = cube(game, "minecraft:slime", 4, SEED);
    dying.id = 9000 + mode;
    game.split_cube_children(&mut dying);
    game.mobs.to_vec()
}

/// Two cows bred by the real route; the parent's id ends in `mode`.
fn breed(game: &mut Game<'_>, mode: u32) -> (Vec<Mob>, Option<u32>) {
    game.mobs.clear();
    game.next_entity = 8999 + mode;
    let (_, _, child) = breed_pair(game, "minecraft:cow");
    (game.mobs.to_vec(), child)
}

fn matrix(mods: mod_api::Mods) {
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // No change: the vanilla three, in order.
            let kids = split(game, 0);
            assert_eq!(kids.len(), 3);
            assert!(kids.iter().all(|k| k.extra.size == 2));
            assert_eq!(kids[0].yaw, YAWS[0]);
            // Cancel: none.
            assert!(split(game, 1).is_empty());
            // Edit A, remove B, append D: [A, C, D].
            let kids = split(game, 2);
            assert_eq!(kids.len(), 3);
            assert_eq!(kids[0].yaw, 90.0);
            assert_eq!(kids[1].yaw, YAWS[2]);
            assert!(near(kids[1].position.x, 10.0 - 0.52));
            assert_eq!(
                (kids[2].position.x, kids[2].yaw, kids[2].extra.size),
                (3.0, 12.0, 2)
            );
            // Replace C with E; a second append of E reports Ownership (2) in A's yaw.
            let kids = split(game, 3);
            assert_eq!(kids.len(), 3);
            assert_eq!(kids[0].yaw, 2.0);
            assert_eq!(kids[2].kind.id, "minecraft:magma_cube");
            assert_eq!((kids[2].position.x, kids[2].yaw), (8.0, 33.0));
            // Invalid input: NaN, unknown kind, wrong dimension (4, 4, 4), duplicate token (2),
            // the 65th child (3) and a NaN move (4): digits 4,4,4,2,3,4 read from the right.
            let kids = split(game, 4);
            assert_eq!(kids.len(), 64);
            assert_eq!(kids[0].yaw, 432_444.0);
            // A removed child's borrow is stale (1).
            let kids = split(game, 5);
            assert_eq!(kids.len(), 2);
            assert_eq!(kids[0].yaw, 1.0);

            // Breeding: untouched baby.
            let (_, child) = breed(game, 0);
            assert_eq!(find(game, child.unwrap()).extra.age, -24000);
            // Veto, and removing the baby: no child, parents resting.
            for mode in [1, 3] {
                let (mobs, child) = breed(game, mode);
                assert_eq!(child, None, "mode {mode}");
                assert!(mobs.iter().all(|m| m.extra.age >= 0));
                assert_eq!(find(game, 9000 + mode).extra.age, 6000);
                assert_eq!(find(game, 9001 + mode).extra.in_love, 0);
            }
            // Replace with a pig baby; a second baby reports Capacity (3).
            let (_, child) = breed(game, 2);
            let pig = find(game, child.unwrap());
            assert_eq!(pig.kind.id, "minecraft:pig");
            assert!(pig.extra.age < 0);
            assert_eq!(pig.yaw, 3.0);
            // A grown replacement is Invalid (4); the cow baby stays.
            let (_, child) = breed(game, 4);
            let baby = find(game, child.unwrap());
            assert_eq!(baby.kind.id, "minecraft:cow");
            assert_eq!(baby.yaw, 4.0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn native_pending_children_shared_matrix() {
    matrix(
        vec![mod_api::ModEntry::new(
            "pending_children_fixture",
            Box::new(Native),
        )]
        .into(),
    );
}
