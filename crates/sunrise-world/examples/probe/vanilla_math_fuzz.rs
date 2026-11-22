//! The Rust side of tools/jmath_fuzz.py's game functions: regenerates `tools/jmath_fuzz/VanillaMathFuzz.java`'s
//! inputs (the same SplitMix64 stream and input classes), computes each Rust variant and compares its bits with
//! vanilla's results in FILE.
//!
//!     world_probe vanilla_math_fuzz FUNCTION SEED COUNT FILE
//!
//! Prints one JSON line per variant, as crates/sunrise-core/examples/jmath_fuzz.rs does.
use std::fs;
use sunrise_core::geometry::Vec3;

struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `VanillaMathFuzz.angle`: a float input in [lo, hi] for class 0.
    fn angle(&mut self, i: u64, lo: f32, hi: f32) -> f32 {
        const EDGES: [f32; 23] = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            45.0,
            -45.0,
            89.9,
            90.0,
            -90.0,
            180.0,
            -180.0,
            270.0,
            360.0,
            -360.0,
            720.0,
            22.5,
            0.5,
            f32::from_bits(1),
            f32::MAX,
            1e9,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        match i & 3 {
            0 => {
                let u = (self.next() >> 11) as f64 * f64::from_bits(0x3ca << 52);
                // `hi - lo` is a float subtraction in Java.
                (f64::from(lo) + u * f64::from(hi - lo)) as f32
            }
            1 => {
                let bits = self.next();
                let exponent = 127 - 30 + ((bits >> 40) % 61) as u32;
                f32::from_bits((bits as u32 & 0x807F_FFFF) | (exponent << 23))
            }
            2 => f32::from_bits(self.next() as u32),
            _ => {
                let edge = EDGES[(self.next() % EDGES.len() as u64) as usize];
                let nudge = (self.next() % 7) as i32 - 3;
                if edge.is_nan() || edge.is_infinite() {
                    edge
                } else {
                    f32::from_bits(edge.to_bits().wrapping_add(nudge as u32))
                }
            }
        }
    }

    /// `VanillaMathFuzz.coord`: a double in [lo, hi] for class 0, the other classes as for `angle`, widened.
    fn coord(&mut self, i: u64, lo: f64, hi: f64) -> f64 {
        if i & 3 == 0 {
            let u = (self.next() >> 11) as f64 * f64::from_bits(0x3ca << 52);
            return lo + u * (hi - lo);
        }
        f64::from(self.angle(i, 0.0, 0.0))
    }
}

/// The view vector Sunrise's gameplay used before it took vanilla's table: double radians and the platform's
/// `sin` and `cos`. Kept here only to measure against.
fn old_view_vector(pitch: f32, yaw: f32) -> Vec3 {
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    Vec3::new(-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos())
}

/// The passenger attachment turn Sunrise used before it took `Vec3::rotate_y`: double radians and the
/// platform's `sin` and `cos`. Kept here only to measure against.
fn old_rotated([x, y, z]: [f64; 3], yaw: f32) -> [f64; 3] {
    let a = -f64::from(yaw).to_radians();
    let (sin, cos) = (a.sin(), a.cos());
    [x * cos + z * sin, y, z * cos - x * sin]
}

/// `Mth.floor` and `Mth.ceil` as Sunrise's own copies had them before `sunrise_core::mth`: truncate, then step
/// with a wrapping `int` (`i32::MIN - 1` becomes `i32::MAX`, as a release build's `i - 1` does). Kept here only
/// to measure against.
fn old_rounding(a: &[f64]) -> Vec<f64> {
    // The step down (floor) or up (ceil) from the truncation `t` when the value lies past it.
    let step = |t: i32, below: bool, above: bool| {
        f64::from(
            t.wrapping_sub(i32::from(below))
                .wrapping_add(i32::from(above)),
        )
    };
    let (x, d) = (a[0] as f32, a[1]);
    let (tx, td) = (x as i32, d as i32);
    vec![
        step(tx, x < tx as f32, false),
        step(tx, false, x > tx as f32),
        step(td, d < f64::from(td), false),
        step(td, false, d > f64::from(td)),
    ]
}

type Variant = (&'static str, fn(&[f64]) -> Vec<f64>);

/// Each function's inputs drawn from the stream (in VanillaMathFuzz.apply's order) and its variants.
fn function(name: &str) -> (fn(&mut Stream, u64) -> Vec<f64>, Vec<Variant>) {
    let vec = |v: Vec3| vec![v.x, v.y, v.z];
    match name {
        "view_vector" => (
            |s, i| {
                let x_rot = s.angle(i, -90.0, 90.0);
                let y_rot = s.angle(i, -360.0, 360.0);
                vec![f64::from(x_rot), f64::from(y_rot)]
            },
            vec![
                ("sunrise", |a| {
                    let v = sunrise_world::view_direction::view_vector(a[0] as f32, a[1] as f32);
                    vec![v.x, v.y, v.z]
                }),
                ("old", |a| {
                    let v = old_view_vector(a[0] as f32, a[1] as f32);
                    vec![v.x, v.y, v.z]
                }),
            ],
        ),
        "vec3_yrot" => (
            |s, i| {
                let (x, y, z) = (
                    s.coord(i, -4.0, 4.0),
                    s.coord(i, -4.0, 4.0),
                    s.coord(i, -4.0, 4.0),
                );
                vec![x, y, z, f64::from(s.angle(i, -360.0, 360.0))]
            },
            vec![
                ("sunrise", |a| {
                    let yaw = a[3] as f32;
                    let v = Vec3::new(a[0], a[1], a[2])
                        .rotate_y(-yaw * sunrise_core::geometry::trig::DEG_TO_RAD);
                    vec![v.x, v.y, v.z]
                }),
                ("old", |a| {
                    old_rotated([a[0], a[1], a[2]], a[3] as f32).to_vec()
                }),
            ],
        ),
        "projectile_rot" => (
            |s, i| {
                vec![
                    s.coord(i, -3.0, 3.0),
                    s.coord(i, -3.0, 3.0),
                    s.coord(i, -3.0, 3.0),
                ]
            },
            vec![
                ("sunrise", |a| {
                    use sunrise_core::geometry::trig::atan2_degrees;
                    let m = Vec3::new(a[0], a[1], a[2]);
                    vec![
                        f64::from(atan2_degrees(m.x, m.z)),
                        f64::from(atan2_degrees(m.y, m.horizontal_distance())),
                    ]
                }),
                ("old", |a| {
                    vec![
                        f64::from(a[0].atan2(a[2]).to_degrees() as f32),
                        f64::from(a[1].atan2(a[0].hypot(a[2])).to_degrees() as f32),
                    ]
                }),
            ],
        ),
        "input_vector" => (
            |s, i| {
                let (x, y, z) = (
                    s.coord(i, -2.0, 2.0),
                    s.coord(i, -2.0, 2.0),
                    s.coord(i, -2.0, 2.0),
                );
                let speed = s.angle(i, 0.0, 1.0);
                vec![
                    x,
                    y,
                    z,
                    f64::from(speed),
                    f64::from(s.angle(i, -360.0, 360.0)),
                ]
            },
            vec![("sunrise", |a| {
                let v = sunrise_world::living_air::input_vector(
                    Vec3::new(a[0], a[1], a[2]),
                    a[3] as f32,
                    a[4] as f32,
                );
                vec![v.x, v.y, v.z]
            })],
        ),
        "blocking_angle" => (
            |s, i| {
                let mut v: Vec<f64> = (0..6).map(|_| s.coord(i, -8.0, 8.0)).collect();
                v.push(f64::from(s.angle(i, -360.0, 360.0)));
                v
            },
            vec![
                ("sunrise", |a| {
                    let (source, position) =
                        (Vec3::new(a[0], a[1], a[2]), Vec3::new(a[3], a[4], a[5]));
                    vec![sunrise_world::view_direction::blocking_angle(
                        source,
                        position,
                        a[6] as f32,
                    )]
                }),
                ("old_shield", |a| {
                    // The shield's own copy before: double radians and platform trig, a 1e-4 threshold, glibc acos.
                    let yaw = (-(a[6] as f32)).to_radians();
                    let view = [yaw.sin() as f64, yaw.cos() as f64];
                    let (dx, dz) = (a[0] - a[3], a[2] - a[5]);
                    let len = (dx * dx + dz * dz).sqrt();
                    let to = if len < 1.0e-4 {
                        [0.0, 0.0]
                    } else {
                        [dx / len, dz / len]
                    };
                    vec![(to[0] * view[0] + to[1] * view[1]).acos()]
                }),
            ],
        ),
        "mth_rounding" => (
            |s, i| {
                let x = s.angle(i, -3e9, 3e9);
                vec![f64::from(x), s.coord(i, -3e9, 3e9)]
            },
            vec![
                ("sunrise", |a| {
                    use sunrise_core::mth::{ceil, ceil_f32, floor, floor_f32};
                    let x = a[0] as f32;
                    [floor_f32(x), ceil_f32(x), floor(a[1]), ceil(a[1])]
                        .map(f64::from)
                        .to_vec()
                }),
                ("old", old_rounding),
            ],
        ),
        other => {
            let _ = vec;
            panic!("unknown function {other}")
        }
    }
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = crate::args().collect();
    let (name, seed, count, file) = (
        &args[1],
        args[2].parse::<u64>()?,
        args[3].parse::<u64>()?,
        &args[4],
    );
    let vanilla = fs::read(file)?;
    let (inputs, variants) = function(name);
    let mut s = Stream(seed);
    let mut mismatches = vec![0u64; variants.len()];
    let mut by_class = vec![[0u64; 4]; variants.len()];
    let mut samples: Vec<Vec<String>> = vec![Vec::new(); variants.len()];
    let mut at = 0usize;
    for i in 0..count {
        let input = inputs(&mut s, i);
        let mut expected = Vec::new();
        for (v, (_, compute)) in variants.iter().enumerate() {
            let got = compute(&input);
            if expected.is_empty() {
                for k in 0..got.len() {
                    let o = at + 8 * k;
                    expected.push(f64::from_bits(u64::from_le_bytes(
                        vanilla[o..o + 8].try_into()?,
                    )));
                }
            }
            if got.iter().zip(&expected).all(|(&g, &e)| same(g, e)) {
                continue;
            }
            mismatches[v] += 1;
            by_class[v][(i & 3) as usize] += 1;
            if samples[v].len() < 8 {
                let hex = |xs: &[f64]| {
                    xs.iter()
                        .map(|x| format!("\"{:#018x}\"", x.to_bits()))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                samples[v].push(format!(
                    "{{\"in\":[{}],\"vanilla\":[{}],\"rust\":[{}]}}",
                    hex(&input),
                    hex(&expected),
                    hex(&got)
                ));
            }
        }
        at += 8 * expected.len();
    }
    assert_eq!(at, vanilla.len(), "result count");
    for (v, (variant, _)) in variants.iter().enumerate() {
        println!(
            "{{\"function\":\"{name}\",\"variant\":\"{variant}\",\"seed\":{seed},\"count\":{count},\"mismatches\":{},\"by_class\":{:?},\"worst_ulps\":null,\"samples\":[{}]}}",
            mismatches[v],
            by_class[v],
            samples[v].join(",")
        );
    }
    Ok(())
}
