//! Moving colliders on a real game: set, read, refused, saved and loaded back, and solid to a
//! falling item.
use super::{Dim, Game, native_colliders::SAVE_KEY};
use mod_api::{Collider, ColliderBox, ColliderError, EntityRef};
use std::sync::Arc;
use sunrise_client::chunk::Chunk;
use sunrise_client::{entities::ItemEntity, items::ItemStack};
use sunrise_core::{
    geometry::Vec3,
    nbt::{Compound, Tag},
};

fn deck() -> Collider {
    Collider {
        boxes: vec![ColliderBox {
            min: [-1.5, -1.0, -1.5],
            max: [1.5, 0.0, 1.5],
        }],
        rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// A pig with no AI and no gravity at `at`: it stays where it is put.
fn stand(game: &mut Game<'_>, at: [f64; 3]) -> EntityRef {
    stand_with_id(game, at).1
}

fn stand_with_id(game: &mut Game<'_>, at: [f64; 3]) -> (u32, EntityRef) {
    let nbt: Compound = [
        ("NoGravity".into(), Tag::Byte(1)),
        ("NoAI".into(), Tag::Byte(1)),
        ("Silent".into(), Tag::Byte(1)),
    ]
    .into_iter()
    .collect();
    let dimension = game.dim.id();
    let id = game
        .summon(dimension, "minecraft:pig", at, nbt)
        .expect("a pig");
    (id, game.entity(id).expect("its handle"))
}

#[test]
#[ignore = "daily"]
fn a_collider_is_set_read_refused_saved_and_loaded() {
    with_world(|game| {
        let entity = stand(game, [1.5, 70.0, 1.5]);
        assert_eq!(entity.collider(game), None);
        assert_eq!(entity.set_collider(game, Some(deck())), Ok(()));
        assert_eq!(entity.collider(game), Some(deck()));

        // Every refusal is by name, and the collider that was set stays.
        let mut bad = deck();
        bad.boxes[0].max[1] = f64::NAN;
        assert_eq!(
            entity.set_collider(game, Some(bad)),
            Err(ColliderError::NonFinite { index: Some(0) })
        );
        let mut far = deck();
        far.boxes[0].max[0] = Collider::MAX_REACH + 1.0;
        assert_eq!(
            entity.set_collider(game, Some(far)),
            Err(ColliderError::OutOfRange { index: 0 })
        );
        let mut many = deck();
        many.boxes = vec![many.boxes[0]; Collider::MAX_BOXES + 1];
        assert_eq!(
            entity.set_collider(game, Some(many)),
            Err(ColliderError::TooManyBoxes {
                count: Collider::MAX_BOXES as u64 + 1,
                limit: Collider::MAX_BOXES as u64
            })
        );
        let mut turned = deck();
        turned.rotation = [0.0, 0.0, 0.0, 3.0];
        assert_eq!(
            entity.set_collider(game, Some(turned)),
            Err(ColliderError::NonFinite { index: None })
        );
        assert_eq!(entity.collider(game), Some(deck()));
        let epoch = entity.epoch();
        assert_eq!(
            EntityRef::new(epoch, 999_999).set_collider(game, Some(deck())),
            Err(ColliderError::StaleEntity)
        );

        // The rotation alone.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(
            entity.set_collider_transform(game, [0.0, half, 0.0, half]),
            Ok(())
        );
        assert_eq!(
            entity.collider(game).unwrap().rotation,
            [0.0, half, 0.0, half]
        );
        assert_eq!(
            entity.set_collider_transform(game, [f32::NAN, 0.0, 0.0, 1.0]),
            Err(ColliderError::NonFinite { index: None })
        );
        let other = stand(game, [10.5, 70.0, 10.5]);
        assert_eq!(
            other.set_collider_transform(game, [0.0, 0.0, 0.0, 1.0]),
            Err(ColliderError::NoCollider)
        );

        // Saved with the entity, and read back by the entity that loads it.
        let id = entity.id();
        let mut tag = Tag::Compound(Compound::new());
        super::persist::write_attachments(&mut tag, id, &game.entity_attachments);
        assert!(tag.get(SAVE_KEY).is_some(), "the save holds the collider");
        let want = entity.collider(game);
        assert_eq!(entity.set_collider(game, None), Ok(()));
        assert_eq!(entity.collider(game), None);
        let mut cleared = Tag::Compound(Compound::new());
        super::persist::write_attachments(&mut cleared, id, &game.entity_attachments);
        assert!(cleared.get(SAVE_KEY).is_none(), "none is not saved");
        game.load_saved_collider(id, &tag);
        assert_eq!(entity.collider(game), want);
        // A malformed save is dropped, and leaves none.
        assert_eq!(entity.set_collider(game, None), Ok(()));
        let broken = Tag::compound([(SAVE_KEY, Tag::compound([("rotation", Tag::Int(1))]))]);
        game.load_saved_collider(id, &broken);
        assert_eq!(entity.collider(game), None);
    });
}

/// A stone dropped at `at`, two blocks above it, in the level in place: its id.
fn drop_item(game: &mut Game<'_>, at: [f64; 3]) -> u32 {
    game.next_entity += 1;
    let id = game.next_entity;
    game.dropped.push(ItemEntity::new(
        id,
        ItemStack::new("minecraft:stone", 1),
        Vec3::new(at[0], at[1] + 2.0, at[2]),
        Vec3::ZERO,
        0.0,
    ));
    id
}

fn item_y(game: &Game<'_>, id: u32) -> Option<f64> {
    game.dropped
        .iter()
        .find(|i| i.id == id)
        .map(|i| i.position.y)
}

/// An item dropped over a collider rests on its top: the one collision path every mover takes
/// finds the collider, placed at the start of the tick.
#[test]
#[ignore = "daily"]
fn a_dropped_item_rests_on_a_collider() {
    with_world(|game| {
        let entity = stand(game, [1.5, 70.0, 1.5]);
        entity.set_collider(game, Some(deck())).unwrap();
        let held = drop_item(game, [1.5, 70.0, 1.5]);
        for _ in 0..60 {
            tick_level(game);
        }
        let y = item_y(game, held).unwrap();
        assert!(
            (70.0..70.2).contains(&y),
            "the item rests on the top: y = {y}"
        );
        // Without the collider the same drop falls past that height.
        entity.set_collider(game, None).unwrap();
        let falling = drop_item(game, [1.5, 70.0, 1.5]);
        for _ in 0..60 {
            tick_level(game);
        }
        let y = item_y(game, falling).unwrap();
        assert!(y < 69.9, "nothing holds it up: y = {y}");
    });
}

/// Each level ticks with its own colliders: an item over the overworld's collider rests on it,
/// and an item at the same place in the nether, which has none there, falls past it.
#[test]
#[ignore = "daily"]
fn each_level_collides_with_its_own_colliders() {
    with_world(|game| {
        let spot = [1.5, 70.0, 1.5];
        let other = [10.5, 70.0, 10.5];
        let overworld = stand(game, spot);
        overworld.set_collider(game, Some(deck())).unwrap();
        game.enter_level(Dim::Nether).unwrap();
        game.native_level_opened(Dim::Nether);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.streaming.forced.insert((0, 0));
        let nether = stand(game, other);
        nether.set_collider(game, Some(deck())).unwrap();
        // The nether's items: one over its own collider, one over the overworld's place.
        let (n_own, n_foreign) = (drop_item(game, other), drop_item(game, spot));
        for _ in 0..60 {
            tick_level(game);
        }
        game.enter_level(Dim::Overworld).unwrap();
        // The overworld's: one over its own collider, one over the nether's place.
        let (o_own, o_foreign) = (drop_item(game, spot), drop_item(game, other));
        for _ in 0..60 {
            tick_level(game);
        }
        let rests = |y: Option<f64>| y.is_some_and(|y| (70.0..70.2).contains(&y));
        assert!(rests(item_y(game, o_own)), "{:?}", item_y(game, o_own));
        assert!(
            item_y(game, o_foreign).is_none_or(|y| y < 69.9),
            "the nether's collider holds no overworld item: {:?}",
            item_y(game, o_foreign)
        );
        game.enter_level(Dim::Nether).unwrap();
        assert!(rests(item_y(game, n_own)), "{:?}", item_y(game, n_own));
        assert!(
            item_y(game, n_foreign).is_none_or(|y| y < 69.9),
            "the overworld's collider holds no nether item: {:?}",
            item_y(game, n_foreign)
        );
        game.enter_level(Dim::Overworld).unwrap();
    });
}

/// A call into the level in place, made while another level's colliders are in scope (a hook of
/// one level reaching into another), reads the level in place's colliders, and the caller's
/// are back afterwards.
#[test]
#[ignore = "daily"]
fn a_call_into_another_level_reads_that_levels_colliders_and_restores_the_callers() {
    use sunrise_core::geometry::Aabb;
    use sunrise_world::entity_obstruction::EntityIdentity;
    with_world(|game| {
        let (spot, other) = ([1.5, 70.0, 1.5], [10.5, 70.0, 10.5]);
        stand(game, spot).set_collider(game, Some(deck())).unwrap();
        game.enter_level(Dim::Nether).unwrap();
        game.native_level_opened(Dim::Nether);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.streaming.forced.insert((0, 0));
        stand(game, other).set_collider(game, Some(deck())).unwrap();
        // Each level ticks once, so each has recorded its colliders.
        tick_level(game);
        game.enter_level(Dim::Overworld).unwrap();
        tick_level(game);
        let meets = |at: [f64; 3]| {
            let mut found = Vec::new();
            let bounds = Aabb::new(
                Vec3::new(at[0] - 0.5, 69.2, at[2] - 0.5),
                Vec3::new(at[0] + 0.5, 69.8, at[2] + 0.5),
            );
            sunrise_client_render::moving_colliders::near(EntityIdentity(0), bounds, &mut found);
            found.len()
        };
        // The overworld's colliders are the caller's scope.
        let placed = game.placed_moving_colliders();
        let _scope = sunrise_client_render::moving_colliders::Scope::enter_level(
            game.collider_level(),
            placed,
        );
        assert_eq!((meets(spot), meets(other)), (1, 0));
        game.enter_level(Dim::Nether).unwrap();
        game.in_this_levels_colliders(|_| {
            assert_eq!(
                (meets(spot), meets(other)),
                (0, 1),
                "the level in place's colliders, not the caller's"
            );
        });
        assert_eq!(
            (meets(spot), meets(other)),
            (1, 0),
            "the caller's colliders are back"
        );
        game.enter_level(Dim::Overworld).unwrap();
    });
}

/// A game that simulates its overworld, with the chunk the tests build on loaded.
fn with_world(test: impl FnOnce(&mut Game<'_>)) {
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.streaming.forced.insert((0, 0));
        game.native_joins_skip();
        test(game);
    });
}

/// One tick of the level in place, as `tick_server` runs it: with the level's colliders placed.
fn tick_level(game: &mut Game<'_>) {
    tick_level_moving(game, |_| {});
}

/// [`tick_level`] with a mod moving its colliders during the tick, after the colliders were
/// placed and the movers ran, which is when a mod's own tick moves them.
fn tick_level_moving(game: &mut Game<'_>, moving: impl FnOnce(&mut Game<'_>)) {
    let placed = game.placed_moving_colliders();
    sunrise_client_render::moving_colliders::in_level(placed.clone(), || {
        game.tick_world(true).unwrap()
    });
    moving(game);
    // Then the bodies on a collider that moved are carried with it, as the level's tick does.
    game.carry_pass(&placed);
}

/// A collider goes with its entity through a dimension change: the same collider reads back
/// from the arrived entity, and it still holds an item up in the destination.
#[test]
#[ignore = "daily"]
fn a_collider_travels_with_its_entity() {
    with_world(|game| {
        let source = game.dim;
        let (id, entity) = stand_with_id(game, [1.5, 70.0, 1.5]);
        entity.set_collider(game, Some(deck())).unwrap();
        let want = entity.collider(game);
        game.enter_level(Dim::Nether).unwrap();
        game.native_level_opened(Dim::Nether);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.streaming.forced.insert((0, 0));
        game.arrive_vehicle(source, id, Vec3::new(1.5, 70.0, 1.5), 0.0);
        for _ in 0..3 {
            tick_level(game);
        }
        let arrived = game.entity(id).expect("it arrived");
        assert_eq!(arrived.collider(game), want, "the collider came along");
        let held = drop_item(game, [1.5, 70.0, 1.5]);
        for _ in 0..60 {
            tick_level(game);
        }
        let y = item_y(game, held).unwrap();
        assert!(
            (70.0..70.2).contains(&y),
            "it still holds an item up: y = {y}"
        );
    });
}

/// The collider's own entity is never stopped by it, and neither is anything in its vehicle
/// stack, ancestors and siblings included; a stranger over the same boxes is.
#[test]
#[ignore = "daily"]
fn a_collider_is_not_solid_to_its_own_vehicle_stack() {
    with_world(|game| {
        let (a, a_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (b, b_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (c, c_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (stranger, _) = stand_with_id(game, [10.5, 70.0, 10.5]);
        // a rides b rides c.
        game.entity_start_riding(a_ref, b_ref, true).unwrap();
        game.entity_start_riding(b_ref, c_ref, true).unwrap();
        // The collider is on the middle of the stack.
        b_ref.set_collider(game, Some(deck())).unwrap();
        for held in [a, b, c] {
            let mut stack = game.vehicle_stack(held);
            stack.sort_unstable();
            assert_eq!(stack, vec![a, b, c], "the stack seen from {held}");
        }
        assert_eq!(game.vehicle_stack(stranger), vec![stranger]);
        // What a mover of each kind meets where the collider stands.
        let placed = game.placed_moving_colliders();
        assert_eq!(placed.len(), 1);
        let mut exempt = placed.exempt(0).unwrap().to_vec();
        exempt.sort_unstable();
        assert_eq!(exempt, vec![a, b, c]);
        let mut stands_on = |id: u32| {
            use sunrise_client_render::moving_colliders::{in_level, near};
            let mut found = Vec::new();
            in_level(game.placed_moving_colliders(), || {
                near(
                    sunrise_world::entity_obstruction::EntityIdentity(u64::from(id)),
                    sunrise_core::geometry::Aabb::new(
                        Vec3::new(1.0, 69.5, 1.0),
                        Vec3::new(2.0, 71.0, 2.0),
                    ),
                    &mut found,
                );
            });
            !found.is_empty()
        };
        assert!(!stands_on(a) && !stands_on(b) && !stands_on(c));
        assert!(stands_on(stranger), "a stranger meets it");
    });
}

/// An item is a mover that carries its own collider: it moves through the gated collision as
/// a self move, so the boxes under its own feet are not solid to it and it falls through them,
/// where a stranger over the same boxes lands.
#[test]
#[ignore = "daily"]
fn an_item_falls_through_its_own_collider_and_a_stranger_lands_on_it() {
    let deck_under = || Collider {
        boxes: vec![ColliderBox {
            min: [-1.0, -2.5, -1.0],
            max: [1.0, -2.0, 1.0],
        }],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };
    with_world(|game| {
        // A holder that stays where it is, with the deck's top at y = 70, and a stranger over it.
        let fixed = drop_item(game, [1.5, 70.0, 1.5]);
        game.dropped
            .iter_mut()
            .find(|i| i.id == fixed)
            .unwrap()
            .no_gravity = true;
        game.entity(fixed)
            .unwrap()
            .set_collider(game, Some(deck_under()))
            .unwrap();
        let stranger = drop_item(game, [1.2, 71.0, 1.2]);
        // A holder that falls, with the same deck under its own feet.
        let own = drop_item(game, [10.5, 70.0, 10.5]);
        game.entity(own)
            .unwrap()
            .set_collider(game, Some(deck_under()))
            .unwrap();
        for _ in 0..60 {
            tick_level(game);
        }
        let other = item_y(game, stranger).unwrap();
        assert!(
            (70.0..70.3).contains(&other),
            "the stranger stands on the fixed holder's deck: y = {other}"
        );
        // The falling holder took its deck with it and was never stopped by it: with the deck
        // a block and a half under its feet, a stopped holder would rest at y = 72 - 2 + ... 70.
        let fell = item_y(game, own).unwrap_or(f64::NEG_INFINITY);
        assert!(
            fell < 60.0,
            "the holder fell through its own deck: y = {fell}"
        );
    });
}

/// Siblings in one vehicle stack are exempt from each other's collider, and a mover outside
/// the stack is not.
#[test]
#[ignore = "daily"]
fn siblings_riding_one_vehicle_share_a_stack() {
    with_world(|game| {
        let (x, x_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (y, y_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (z, z_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (other, _) = stand_with_id(game, [10.5, 70.0, 10.5]);
        game.entity_start_riding(x_ref, z_ref, true).unwrap();
        game.entity_start_riding(y_ref, z_ref, true).unwrap();
        x_ref.set_collider(game, Some(deck())).unwrap();
        for member in [x, y, z] {
            let mut stack = game.vehicle_stack(member);
            stack.sort_unstable();
            let mut want = vec![x, y, z];
            want.sort_unstable();
            assert_eq!(stack, want, "seen from {member}");
        }
        assert_eq!(game.vehicle_stack(other), vec![other]);
        // What the exemption does: the collision query leaves the siblings and the vehicle
        // out and meets a stranger.
        let placed = game.placed_moving_colliders();
        let meets = |id: u32| {
            let mut found = Vec::new();
            sunrise_client_render::moving_colliders::in_level(placed.clone(), || {
                sunrise_client_render::moving_colliders::near(
                    sunrise_world::entity_obstruction::EntityIdentity(u64::from(id)),
                    sunrise_core::geometry::Aabb::new(
                        Vec3::new(1.0, 69.5, 1.0),
                        Vec3::new(2.0, 71.0, 2.0),
                    ),
                    &mut found,
                );
            });
            !found.is_empty()
        };
        assert!(!meets(x) && !meets(y) && !meets(z), "the stack is exempt");
        assert!(meets(other), "a stranger meets it");
    });
}

/// What a level's tick costs with 200 items over a collider at the box cap: with no collider,
/// with one that stays, with one rebuilt every tick but moved before it begins, and with one
/// that moves during the tick and carries all 200 (Create's normal case), each timed over the
/// same ticks. The numbers go to stderr (`--nocapture`); the test holds the outcome: the items
/// rest on the capped collider, moving or not, and go with it when it moves.
#[test]
#[ignore = "daily"]
fn two_hundred_items_over_a_capped_collider_rest_and_the_tick_is_timed() {
    use std::time::Instant;
    const SIDE: usize = 32;
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        let items = |game: &mut Game<'_>| -> Vec<u32> {
            (0..200)
                .map(|i| {
                    let at = [0.5 + (i % 15) as f64, 70.0, 0.5 + (i / 15) as f64];
                    drop_item(game, at)
                })
                .collect()
        };
        let time =
            |game: &mut Game<'_>, ticks: usize, mut each: Option<&mut dyn FnMut(&mut Game<'_>)>| {
                let start = Instant::now();
                for _ in 0..ticks {
                    if let Some(each) = each.as_mut() {
                        each(game);
                    }
                    tick_level(game);
                }
                start.elapsed().as_secs_f64() * 1000.0 / ticks as f64
            };
        // No collider: the items fall in the open.
        let before = items(game);
        let open = time(game, 40, None);
        for id in &before {
            game.dropped.retain(|i| i.id != *id);
        }
        // The cap: a 32 x 32 x 32 lump, its top at the holder's feet.
        let boxes = (0..SIDE * SIDE * SIDE)
            .map(|i| {
                let (x, y, z) = (i % SIDE, i / SIDE % SIDE, i / SIDE / SIDE);
                ColliderBox {
                    min: [x as f64 - 16.0, y as f64 - 32.0, z as f64 - 16.0],
                    max: [x as f64 - 15.0, y as f64 - 31.0, z as f64 - 15.0],
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(boxes.len(), Collider::MAX_BOXES);
        holder_ref
            .set_collider(
                game,
                Some(Collider {
                    boxes,
                    rotation: [0.0, 0.0, 0.0, 1.0],
                }),
            )
            .unwrap();
        let on = items(game);
        for _ in 0..20 {
            tick_level(game);
        }
        let rests = |game: &Game<'_>| {
            on.iter()
                .filter(|id| item_y(game, **id).is_some_and(|y| (70.0..70.3).contains(&y)))
                .count()
        };
        assert_eq!(rests(game), 200, "every item rests on the capped collider");
        let still = time(game, 40, None);
        assert_eq!(rests(game), 200);
        // The collider moves every tick: its placement is rebuilt each time.
        let index = game.mobs.iter().position(|m| m.id == holder).unwrap();
        let mut step = |game: &mut Game<'_>| {
            game.mobs.update(index, |m| m.position.x += 0.001);
        };
        let moving = time(game, 40, Some(&mut step));
        assert_eq!(rests(game), 200, "and they still rest on it while it moves");
        // The collider moves during the tick, after the bodies were placed, so the carry pass
        // carries all 200 items (the phase above moves it before the tick: its pass sees none).
        let from: Vec<f64> = on.iter().map(|id| item_at(game, *id).x).collect();
        let began = Instant::now();
        for _ in 0..40 {
            tick_level_moving(game, |g| {
                g.mobs.update(index, |m| m.position.x += 0.05);
            });
        }
        let carried = began.elapsed().as_secs_f64() * 1000.0 / 40.0;
        assert_eq!(rests(game), 200, "and they are carried with it");
        for (id, x) in on.iter().zip(&from) {
            let carried_by = item_at(game, *id).x - x;
            assert!(
                (carried_by - 2.0).abs() < 0.05,
                "item {id} moved {carried_by} with a collider that moved 2.0"
            );
        }
        eprintln!(
            "tick of the level, 200 items, ms per tick: no collider {open:.3}, capped collider still {still:.3}, capped collider moved before the tick {moving:.3}, carried {carried:.3}"
        );
    });
}

// Carrying: the bodies standing on a collider move with it.

/// Where an item is.
fn item_at(game: &Game<'_>, id: u32) -> Vec3 {
    game.dropped
        .iter()
        .find(|i| i.id == id)
        .map(|i| i.position)
        .expect("an item")
}

/// A different stack from `drop_item`'s, so two items never merge.
fn drop_dirt(game: &mut Game<'_>, at: [f64; 3]) -> u32 {
    game.next_entity += 1;
    let id = game.next_entity;
    game.dropped.push(ItemEntity::new(
        id,
        ItemStack::new("minecraft:dirt", 1),
        Vec3::new(at[0], at[1] + 2.0, at[2]),
        Vec3::ZERO,
        0.0,
    ));
    id
}

/// Moves the pig that holds a collider, as a mod's own tick would.
fn shift_stand(game: &mut Game<'_>, id: u32, by: Vec3) {
    let index = game.mobs.iter().position(|m| m.id == id).expect("a pig");
    game.mobs.update(index, |m| {
        m.position = Vec3::new(
            m.position.x + by.x,
            m.position.y + by.y,
            m.position.z + by.z,
        );
    });
}

fn settle(game: &mut Game<'_>) {
    for _ in 0..25 {
        tick_level(game);
    }
}

fn near(a: Vec3, b: Vec3, within: f64) -> bool {
    (a.x - b.x).abs() <= within && (a.y - b.y).abs() <= within && (a.z - b.z).abs() <= within
}

/// A body resting on a collider that moves is carried by what the contact point moves; one off
/// the deck is not, and the collider is never stopped by the body.
#[test]
#[ignore = "daily"]
fn a_body_on_a_moving_collider_is_carried() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        // Beside the deck, on nothing: it keeps falling and is not carried with the deck.
        let beside = drop_dirt(game, [12.0, 70.0, 8.0]);
        settle(game);
        let from = item_at(game, rider);
        assert!(
            near(from, Vec3::new(8.5, 70.0, 8.0), 0.2),
            "rests on the deck: {from:?}"
        );
        let beside_x = item_at(game, beside).x;
        for _ in 0..20 {
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.1, 0.0, 0.05)));
        }
        let to = item_at(game, rider);
        assert!(
            near(to, Vec3::new(from.x + 2.0, from.y, from.z + 1.0), 0.05),
            "carried by the deck's 2 x 1: {from:?} -> {to:?}"
        );
        assert!(
            (item_at(game, beside).x - beside_x).abs() < 0.05,
            "an item off the deck stays"
        );
        // The deck stops: so does the rider.
        for _ in 0..5 {
            tick_level(game);
        }
        assert!(
            near(item_at(game, rider), to, 0.02),
            "a still deck carries nothing"
        );
    });
}

/// A collider that turns carries the body at a distance from its origin along the arc, the
/// contact point's own displacement: yaw, and roll (a deck that tilts under the body).
#[test]
#[ignore = "daily"]
fn a_turning_collider_carries_the_body_along_the_arc() {
    with_world(|game| {
        let (_, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [9.0, 70.0, 8.0]);
        settle(game);
        let yaw = |degrees: f32| {
            let half = degrees.to_radians() / 2.0;
            [0.0, half.sin(), 0.0, half.cos()]
        };
        // Nine turns of 10 degrees: a quarter turn. Local +x (1 block out) goes to -z.
        for step in 1..=9 {
            tick_level_moving(game, |g| {
                holder_ref
                    .set_collider_transform(g, yaw(10.0 * step as f32))
                    .unwrap()
            });
        }
        let at = item_at(game, rider);
        assert!(
            near(at, Vec3::new(8.0, 70.0, 7.0), 0.06),
            "a quarter turn about the deck's centre: {at:?}"
        );
    });
}

/// A deck that rises carries the body up with it, at 0.3 blocks a tick.
#[test]
#[ignore = "daily"]
fn a_rising_collider_carries_the_body_up() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        settle(game);
        let from = item_at(game, rider);
        for _ in 0..10 {
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.0, 0.3, 0.0)));
        }
        let to = item_at(game, rider);
        assert!(
            near(to, Vec3::new(from.x, from.y + 3.0, from.z), 0.05),
            "up 3 blocks: {from:?} -> {to:?}"
        );
    });
}

/// A deck tilting under a body (roll about z) lifts it by the contact point's rise.
#[test]
#[ignore = "daily"]
fn a_tilting_collider_lifts_the_body_by_its_contact_point() {
    with_world(|game| {
        let (_, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [9.0, 70.0, 8.0]);
        settle(game);
        let roll = 15.0_f32.to_radians() / 2.0;
        tick_level_moving(game, |g| {
            holder_ref
                .set_collider_transform(g, [0.0, 0.0, roll.sin(), roll.cos()])
                .unwrap()
        });
        // The contact point 1 block out along local x rises by sin 15 and moves out by cos 15.
        let want = Vec3::new(
            8.0 + 15.0_f64.to_radians().cos(),
            70.0 + 15.0_f64.to_radians().sin(),
            8.0,
        );
        let at = item_at(game, rider);
        assert!(near(at, want, 0.1), "{at:?} vs {want:?}");
    });
}

/// A body is carried into a wall never through it, and the collider is not stopped.
#[test]
#[ignore = "daily"]
fn a_carried_body_stops_at_a_wall_and_the_collider_goes_on() {
    with_world(|game| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for y in 70..=72 {
            for z in 6..=10 {
                game.world.set(10, y, z, stone).unwrap();
            }
        }
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        settle(game);
        for _ in 0..15 {
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.1, 0.0, 0.0)));
        }
        let at = item_at(game, rider);
        assert!(at.x < 10.0, "never through the wall: {at:?}");
        assert!(at.x > 9.8, "carried up to it: {at:?}");
        let moved = game
            .mobs
            .iter()
            .find(|m| m.id == holder)
            .unwrap()
            .position
            .x;
        assert!(
            (moved - 9.5).abs() < 1.0e-6,
            "the collider is not stopped: {moved}"
        );
    });
}

/// A collider standing on a collider is carried first, and its riders take its whole
/// displacement.
#[test]
#[ignore = "daily"]
fn a_collider_on_a_collider_passes_the_displacement_on() {
    crate::game::in_own_process(
        "game::native_colliders_tests::a_collider_on_a_collider_passes_the_displacement_on",
        || {
            with_world(|game| {
                let (c1, c1_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
                c1_ref.set_collider(game, Some(deck())).unwrap();
                // The second collider's holder rests on the first deck, with a small deck above it.
                // Its descent clears the pig that holds the first deck, so this tests carry only.
                let c2 = drop_item(game, [9.0, 70.0, 8.0]);
                game.entity(c2)
                    .unwrap()
                    .set_collider(
                        game,
                        Some(Collider {
                            boxes: vec![ColliderBox {
                                min: [-0.5, 0.0, -0.5],
                                max: [0.5, 0.2, 0.5],
                            }],
                            rotation: [0.0, 0.0, 0.0, 1.0],
                        }),
                    )
                    .unwrap();
                // And a body on that.
                let rider = drop_dirt(game, [9.0, 70.2, 8.0]);
                settle(game);
                let (from_c2, from) = (item_at(game, c2), item_at(game, rider));
                assert!(
                    from.y > 70.1,
                    "the rider stands on the small deck: {from:?}"
                );
                for _ in 0..10 {
                    tick_level_moving(game, |g| shift_stand(g, c1, Vec3::new(0.1, 0.0, 0.0)));
                }
                let (to_c2, to) = (item_at(game, c2), item_at(game, rider));
                assert!(
                    near(
                        to_c2,
                        Vec3::new(from_c2.x + 1.0, from_c2.y, from_c2.z),
                        0.05
                    ),
                    "{to_c2:?}"
                );
                assert!(
                    near(to, Vec3::new(from.x + 1.0, from.y, from.z), 0.05),
                    "{from:?} -> {to:?}"
                );
            })
        },
    );
}

/// A carry is an `entity_move` with the cause `Carried`: a listener hears it with the contact
/// point's displacement, and cancelling it leaves the body where it stood.
#[test]
#[ignore = "daily"]
fn a_listener_hears_a_carry_and_may_cancel_it() {
    use mod_api::{EntityMoveEvent, Flow, Hooks, LoopContext, MoveCause};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Listener(Arc<AtomicBool>, Arc<Mutex<Vec<(u32, MoveCause, [f64; 3])>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
            if event.cause == MoveCause::Carried {
                self.1
                    .lock()
                    .unwrap()
                    .push((event.entity, event.cause, *event.delta));
                if self.0.load(Ordering::Relaxed) {
                    return Flow::Cancel;
                }
            }
            Flow::Continue
        }
    }
    let cancel = Arc::new(AtomicBool::new(true));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "carry_listener",
        Box::new(Listener(cancel.clone(), heard.clone())),
    )]
    .into();
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        settle(game);
        let from = item_at(game, rider);
        mod_platform::testing::with_mods(mods, || {
            // Cancelled: heard, and the body does not move.
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.5, 0.0, 0.0)));
            assert!(
                near(item_at(game, rider), from, 0.01),
                "a cancelled carry moves nothing"
            );
            let first = heard.lock().unwrap().clone();
            assert_eq!(first.len(), 1, "{first:?}");
            assert_eq!(first[0].0, rider);
            assert!(
                (first[0].2[0] - 0.5).abs() < 1.0e-6,
                "the contact point's 0.5: {first:?}"
            );
            // Allowed: the same carry goes through.
            cancel.store(false, Ordering::Relaxed);
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.5, 0.0, 0.0)));
            assert!(
                near(
                    item_at(game, rider),
                    Vec3::new(from.x + 0.5, from.y, from.z),
                    0.02
                ),
                "{:?}",
                item_at(game, rider)
            );
        });
    });
}

/// A carried vehicle takes its passenger and that passenger's own passenger with it: all three
/// move by the same displacement, the riders seated by the one rider positioning the tick uses.
#[test]
#[ignore = "daily"]
fn a_carried_vehicle_takes_its_passengers_and_theirs_with_it() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let (vehicle, _) = stand_with_id(game, [8.5, 70.0, 8.0]);
        let (middle, _) = stand_with_id(game, [8.5, 71.0, 8.0]);
        let (top, _) = stand_with_id(game, [8.5, 72.0, 8.0]);
        let update = |game: &mut Game<'_>, id: u32, f: &dyn Fn(&mut sunrise_client::mobs::Mob)| {
            let at = game.mobs.iter().position(|m| m.id == id).expect("a pig");
            game.mobs.update(at, |m| f(m));
        };
        update(game, vehicle, &|m| m.ground.on_ground = true);
        update(game, middle, &|m| m.extra.mount = Some(vehicle));
        update(game, top, &|m| m.extra.mount = Some(middle));
        game.position_riders();
        let position = |game: &Game<'_>, id: u32| {
            game.mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.position)
                .expect("a pig")
        };
        let before = [vehicle, middle, top].map(|id| position(game, id));
        assert!(
            before[1].y > before[0].y && before[2].y > before[1].y,
            "{before:?}"
        );
        tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.5, 0.0, 0.0)));
        for (id, was) in [vehicle, middle, top].into_iter().zip(before) {
            let now = position(game, id);
            assert!(
                near(now, Vec3::new(was.x + 0.5, was.y, was.z), 0.02),
                "{id}: {was:?} -> {now:?}"
            );
        }
    });
}

/// The level records its colliders as each of its ticks begins, so a report that arrives in a
/// tick is judged against the transforms of the ticks before it: age 0 is where the collider
/// was when this tick began, age n where it was n ticks earlier; and a level with no collider
/// keeps nothing.
#[test]
#[ignore = "daily"]
fn the_level_records_its_colliders_as_each_tick_begins() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let x_of = |game: &Game<'_>| {
            game.mobs
                .iter()
                .find(|m| m.id == holder)
                .map(|m| m.position.x)
                .unwrap()
        };
        let mut began = Vec::new();
        for _ in 0..8 {
            began.push(x_of(game));
            // The level's own tick records the colliders as it begins.
            game.tick_server(false).unwrap();
            shift_stand(game, holder, Vec3::new(0.5, 0.0, 0.0));
        }
        let history = game
            .level_collider_history()
            .expect("a server records a level that has a collider");
        let history = history.borrow();
        let (log, _) = history.get(holder).expect("the collider's frames");
        for age in 0..8usize {
            let frame = log.frame_at(history.now(), age as f64).unwrap();
            assert!(
                (frame.origin.x - began[7 - age]).abs() < 1.0e-9,
                "age {age}: {} vs {}",
                frame.origin.x,
                began[7 - age]
            );
        }
        // Between two ticks it is the line between them.
        let mid = log.frame_at(history.now(), 1.5).unwrap().origin.x;
        assert!((mid - (began[6] + began[5]) / 2.0).abs() < 1.0e-9, "{mid}");
        drop(history);
        // The collider goes: the level keeps no history.
        holder_ref.set_collider(game, None).unwrap();
        game.tick_server(false).unwrap();
        assert!(game.level_collider_history().is_none());
    });
}
