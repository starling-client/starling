//! What the moving colliders cost a level's movers, through the path every move takes: the
//! level's scoped index (`moving_colliders::near`) and the step-resolved move against what it
//! found. `cargo run --release -p sunrise-client-render --example moving_colliders_bench`.
use std::{hint::black_box, sync::Arc, time::Instant};
use sunrise_client_render::moving_colliders::{self, Placed, Prepared};
use sunrise_core::geometry::{
    Aabb, Vec3, VoxelShape,
    oriented::{ColliderBoxes, Frame, MAX_BOXES, OrientedCollider},
    try_resolve_step_movement_oriented,
};
use sunrise_world::entity_obstruction::EntityIdentity;

const MOVERS: usize = 200;
const TICKS: usize = 200;

/// A collider at the box cap: a 32 x 32 x 32 lump of unit boxes, its top at y = 32.
fn capped(at: Vec3) -> Placed {
    let side = 32;
    assert_eq!(side * side * side, MAX_BOXES);
    let boxes = (0..MAX_BOXES)
        .map(|i| {
            let (x, y, z) = (
                (i % side) as f64,
                (i / side % side) as f64,
                (i / side / side) as f64,
            );
            Aabb::new(Vec3::new(x, y, z), Vec3::new(x + 1.0, y + 1.0, z + 1.0))
        })
        .collect();
    Placed {
        id: 1,
        collider: OrientedCollider::new(
            Frame::from_quaternion(at, [0.0, 0.1, 0.0, 0.995]),
            Arc::new(ColliderBoxes::new(boxes)),
        ),
        exempt: vec![1],
    }
}

fn small(at: Vec3) -> Placed {
    Placed {
        id: 1,
        collider: OrientedCollider::new(
            Frame::translation(at),
            Arc::new(ColliderBoxes::new(vec![Aabb::new(
                Vec3::ZERO,
                Vec3::new(2.0, 1.0, 2.0),
            )])),
        ),
        exempt: vec![1],
    }
}

/// Nanoseconds per move of the movers in `at`, each moving (0.1, -0.08, 0.1) from its place.
fn per_move(at: &[Vec3]) -> f64 {
    let no_blocks: Vec<&VoxelShape> = Vec::new();
    let mut out = Vec::new();
    let mut acc = 0.0;
    let start = Instant::now();
    for _ in 0..TICKS {
        for p in at {
            let mover = Aabb::new(
                Vec3::new(p.x - 0.3, p.y, p.z - 0.3),
                Vec3::new(p.x + 0.3, p.y + 1.8, p.z + 0.3),
            );
            let movement = Vec3::new(0.1, -0.08, 0.1);
            out.clear();
            moving_colliders::near(
                EntityIdentity(7),
                mover
                    .expand_towards(movement)
                    .expand_towards(Vec3::new(0.0, 0.6, 0.0)),
                &mut out,
            );
            let moved = try_resolve_step_movement_oriented::<&VoxelShape, ()>(
                movement,
                mover,
                0.6,
                true,
                &out,
                |_| Ok(no_blocks.clone()),
            )
            .unwrap();
            acc += black_box(moved).y;
        }
    }
    black_box(acc);
    start.elapsed().as_secs_f64() * 1.0e9 / (TICKS * at.len()) as f64
}

fn main() {
    // 200 movers in the open, and the same 200 standing on a collider at the cap.
    let open: Vec<Vec3> = (0..MOVERS)
        .map(|i| {
            Vec3::new(
                500.0 + (i % 20) as f64 * 3.0,
                64.0,
                500.0 + (i / 20) as f64 * 3.0,
            )
        })
        .collect();
    let on_lump: Vec<Vec3> = (0..MOVERS)
        .map(|i| {
            Vec3::new(
                2.0 + (i % 20) as f64 * 1.3,
                32.0 + 64.0,
                2.0 + (i / 20) as f64 * 3.0,
            )
        })
        .collect();
    let lump_origin = Vec3::new(0.0, 64.0, 0.0);
    println!("per move, 200 movers, {TICKS} ticks (ns):");
    let none = moving_colliders::in_level(Prepared::empty(), || per_move(&open));
    println!("  level with no collider, movers in the open:          {none:8.0}");
    let far =
        moving_colliders::in_level(Prepared::new(vec![capped(lump_origin)]), || per_move(&open));
    println!("  one capped collider, movers far from it:             {far:8.0}");
    let many: Vec<Placed> = (0..200)
        .map(|i| small(Vec3::new(i as f64 * 40.0 - 4000.0, 64.0, -300.0)))
        .collect();
    let crowd = moving_colliders::in_level(Prepared::new(many), || per_move(&open));
    println!("  200 small colliders, movers far from all:            {crowd:8.0}");
    let on = moving_colliders::in_level(Prepared::new(vec![capped(lump_origin)]), || {
        per_move(&on_lump)
    });
    println!("  one capped collider, movers standing on it:          {on:8.0}");
    println!(
        "  that is {:.2} ms per tick for {MOVERS} movers on a capped collider",
        on * MOVERS as f64 / 1.0e6
    );
}
