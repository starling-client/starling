//! Mob-sized movement collision over the test scene: nanoseconds per resolve,
//! and a checksum of every result to compare builds bit for bit.
#![forbid(unsafe_code)]
use sunrise_client::{assets, collisions, world};
use sunrise_core::geometry::{Aabb, Vec3};
use std::{path::PathBuf, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// xorshift64*: the same inputs on every run.
struct Inputs(u64);
impl Inputs {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + self.unit() * (high - low)
    }
}
struct Case {
    bounds: Aabb,
    movement: Vec3,
    on_ground: bool,
}
fn cases(count: usize, seed: u64, walking_only: bool) -> Vec<Case> {
    let mut r = Inputs(seed);
    let sizes = [
        (0.6, 1.95),
        (0.9, 1.4),
        (1.4, 0.9),
        (0.3, 0.3),
        (1.3965, 2.9),
    ];
    (0..count)
        .map(|_| {
            let (width, height) = sizes[(r.next() % sizes.len() as u64) as usize];
            let x = r.range(-20.0, 40.0);
            let z = r.range(-20.0, 40.0);
            // Standing on a block or a half block, or anywhere.
            let y = match r.next() % 4 {
                0 => r.range(3.0, 12.0),
                1 => (r.range(3.0, 12.0) * 2.0).floor() / 2.0,
                _ => r.range(3.0, 12.0).floor(),
            };
            let speed = if walking_only { 0.1 } else { 0.6 };
            let movement = Vec3::new(
                r.range(-speed, speed),
                if walking_only || r.next().is_multiple_of(2) {
                    -0.0784000015258789
                } else {
                    r.range(-1.0, 0.5)
                },
                r.range(-speed, speed),
            );
            let half = width / 2.0;
            Case {
                bounds: Aabb::new(
                    Vec3::new(x - half, y, z - half),
                    Vec3::new(x + half, y + height, z + half),
                ),
                movement,
                on_ground: walking_only || r.next().is_multiple_of(2),
            }
        })
        .collect()
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let resources = PathBuf::from(args.first().ok_or("usage: collision_bench RESOURCES")?);
    let assets = assets::Assets::load(&resources)?;
    let render = assets.reload(&resources)?;
    let world = world::World::test_scene(&assets, &render, 4)?;
    let binding = collisions::load(&resources, assets.registry)?;
    let mut scene = collisions::SceneCollisions {
        world: &world,
        binding: &binding,
        subject: None,
    };
    for (name, walking_only, seed) in [("walking", true, 7), ("mixed", false, 11)] {
        let cases = cases(200_000, seed, walking_only);
        let mut checksum = 0u64;
        let mut best = f64::MAX;
        for _ in 0..5 {
            checksum = 0;
            let started = Instant::now();
            for case in &cases {
                let moved = scene
                    .resolve(case.bounds, case.movement, 0.6, case.on_ground)
                    .unwrap_or(Vec3::new(f64::NAN, f64::NAN, f64::NAN));
                for part in [moved.x, moved.y, moved.z] {
                    checksum = checksum.rotate_left(7) ^ part.to_bits();
                }
            }
            best = best.min(started.elapsed().as_secs_f64() * 1e9 / cases.len() as f64);
        }
        println!("{name}: {best:.0} ns per resolve, checksum {checksum:016x}");
    }
    Ok(())
}
