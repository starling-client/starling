//! Cost of colliding movers with a collider at the box cap: 32,768 boxes (2048 blocks of 16
//! boxes) in a train 171 long, or a lump, 200 movers walking on it. Prints microseconds per mover per
//! tick for the sweep of the three movement axes and the step-up heights, for the grid
//! broad phase and for testing every box.
//!
//! `cargo run --release -p sunrise-core --example collider_bench`
use std::time::Instant;
use sunrise_core::geometry::{
    Aabb, Axis, Vec3,
    oriented::{ColliderBoxes, Frame, OrientedCollider},
};

const BOXES: usize = 32_768;
const MOVERS: usize = 200;
const TICKS: usize = 200;

fn blocks_boxes(x: f64, y: f64, z: f64, out: &mut Vec<Aabb>) {
    // A block of 16 boxes: a cauldron's worst case.
    for k in 0..16 {
        let (dx, dy, dz) = (
            f64::from(k % 2) * 0.5,
            f64::from((k / 2) % 2) * 0.5,
            f64::from(k / 4) * 0.25,
        );
        out.push(Aabb::new(
            Vec3::new(x + dx, y + dy, z + dz),
            Vec3::new(x + dx + 0.5, y + dy + 0.5, z + dz + 0.25),
        ));
    }
}

/// 2048 blocks: a train 3 wide, 4 tall and 171 long, or a cube of 13 x 13 x 12.
fn train(dense: bool) -> Vec<Aabb> {
    let mut boxes = Vec::with_capacity(BOXES);
    for block in 0..2048usize {
        let (x, y, z) = if dense {
            (
                (block % 13) as f64,
                ((block / 13) % 13) as f64,
                (block / 169) as f64,
            )
        } else {
            (
                (block % 3) as f64 - 1.5,
                ((block / 3) % 4) as f64 - 1.0,
                (block / 12) as f64 - 85.0,
            )
        };
        blocks_boxes(x, y, z, &mut boxes);
    }
    boxes
}

fn rng(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 11) as f64 / (1u64 << 53) as f64
}

fn main() {
    let half = 20.0_f64.to_radians() / 2.0;
    for (label, dense) in [("spread train", false), ("dense lump", true)] {
        let boxes = train(dense);
        let build = Instant::now();
        let grid = ColliderBoxes::new(boxes);
        let built = build.elapsed();
        let frame = Frame::from_quaternion(
            Vec3::new(0.0, 64.0, 0.0),
            [half.sin() * 0.3, half.sin(), 0.0, half.cos()],
        );
        let collider = OrientedCollider::new(frame, std::sync::Arc::new(grid));
        let mut seed = 0x1234_5678_9abc_def0u64;
        let movers: Vec<Aabb> = (0..MOVERS)
            .map(|_| {
                // Standing on the top: the train's is at y = 3, the lump's at y = 13.
                let at = if dense {
                    frame.to_world(Vec3::new(
                        1.0 + rng(&mut seed) * 11.0,
                        13.0,
                        1.0 + rng(&mut seed) * 10.0,
                    ))
                } else {
                    frame.to_world(Vec3::new(
                        rng(&mut seed) * 2.0 - 1.0,
                        3.0,
                        (rng(&mut seed) - 0.5) * 160.0,
                    ))
                };
                Aabb::new(
                    Vec3::new(at.x - 0.3, at.y, at.z - 0.3),
                    Vec3::new(at.x + 0.3, at.y + 1.8, at.z + 0.3),
                )
            })
            .collect();
        let start = Instant::now();
        let mut acc = 0.0;
        for _ in 0..TICKS {
            for mover in &movers {
                let step = Vec3::new(0.1, -0.08, 0.1);
                let mut at = *mover;
                for axis in [Axis::Y, Axis::X, Axis::Z] {
                    let moved = collider.collide(at, axis, step.component(axis));
                    acc += moved;
                    at = at.move_by(Vec3::ZERO.with_component(axis, moved));
                }
            }
        }
        let sweeps = start.elapsed().as_secs_f64() * 1.0e6 / (TICKS * MOVERS) as f64;
        let start = Instant::now();
        for _ in 0..TICKS {
            for mover in &movers {
                let mut heights = Vec::new();
                collider.step_heights(*mover, Vec3::new(0.1, 0.0, 0.1), 0.6, &mut heights);
                acc += heights.len() as f64;
            }
        }
        let steps = start.elapsed().as_secs_f64() * 1.0e6 / (TICKS * MOVERS) as f64;
        let per = sweeps + steps;
        println!("  sweeps {sweeps:.2} us, step heights {steps:.2} us");
        std::hint::black_box(acc);
        println!(
            "{label}: build {:.1} ms, {per:.2} us per mover per tick, {:.2} ms per tick for {MOVERS} movers",
            built.as_secs_f64() * 1.0e3,
            per * MOVERS as f64 / 1.0e3
        );
    }
}
