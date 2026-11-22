//! Cost of the carried-player check on a collider at the box cap, in an optimised build: a
//! 32 x 32 x 32 lump of 32768 unit boxes with its top at its origin. Prints nanoseconds per
//! call for the pieces of the report check: one footing query, one frame lookup, the whole age
//! search that fits nothing, and the search of a player walking beside the lump.
//!
//! `cargo run --profile fast -p sunrise-core --example carried_bench`
use std::time::Instant;
use sunrise_core::geometry::{
    Aabb, Vec3,
    carried::{Ages, Body, FrameLog, footing, judge},
    oriented::{ColliderBoxes, Frame},
};

const SIDE: usize = 32;
const BODY: Body = Body {
    half_width: 0.3,
    height: 1.8,
};

fn time(name: &str, runs: u32, mut f: impl FnMut() -> u64) {
    let began = Instant::now();
    let mut sink = 0;
    for _ in 0..runs {
        sink += f();
    }
    let each = began.elapsed().as_secs_f64() * 1.0e9 / f64::from(runs);
    println!("{name}: {each:.0} ns each (sink {sink})");
}

fn main() {
    let lump = ColliderBoxes::new(
        (0..SIDE * SIDE * SIDE)
            .map(|i| {
                let (x, y, z) = (i % SIDE, i / SIDE % SIDE, i / SIDE / SIDE);
                Aabb::new(
                    Vec3::new(x as f64 - 16.0, y as f64 - 32.0, z as f64 - 16.0),
                    Vec3::new(x as f64 - 15.0, y as f64 - 31.0, z as f64 - 15.0),
                )
            })
            .collect(),
    );
    let mut log = FrameLog::default();
    for t in 0..=40u64 {
        log.record(t, Frame::translation(Vec3::new(0.0, 64.0, 2.0 * t as f64)));
    }
    let ages = Ages::new(100, None);
    let runs = 20_000;
    for (name, local) in [
        ("on the lump", Vec3::new(0.3, 0.0, 0.3)),
        ("25 blocks along it", Vec3::new(0.0, 0.0, 25.0)),
        ("beside it", Vec3::new(16.8, 0.0, 0.0)),
        ("far above it", Vec3::new(0.0, 30.0, 0.0)),
    ] {
        time(&format!("footing, {name}"), runs * 10, || {
            u64::from(footing(&lump, std::hint::black_box(local), BODY).is_some())
        });
        let to = log.frame_at(40, 4.0).unwrap().to_world(local);
        time(&format!("judge (attached), {name}"), runs, || {
            judge(&log, 40, &lump, std::hint::black_box(to), BODY, ages, false)
                .map_or(0, |fit| fit.age as u64 + 1)
        });
        time(&format!("judge (standing only), {name}"), runs, || {
            judge(&log, 40, &lump, std::hint::black_box(to), BODY, ages, true)
                .map_or(0, |fit| fit.age as u64 + 1)
        });
    }
    time("frame_at + to_local", runs * 100, || {
        let frame = log.frame_at(40, std::hint::black_box(3.3)).unwrap();
        frame.to_local(Vec3::new(1.0, 2.0, 3.0)).x as u64
    });
}
