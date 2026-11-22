//! The Rust side of tools/jmath_fuzz.py: regenerates `tools/jmath_fuzz/JmathFuzz.java`'s inputs (the same
//! SplitMix64 stream and input classes), computes each Rust variant of the function and compares its bits with
//! the JVM's results in FILE.
//!
//!     jmath_fuzz FUNCTION SEED COUNT FILE
//!
//! Prints one JSON line per variant: mismatches, the largest distance in ulps, and up to 8 sample inputs with
//! both results. NaN results compare equal whatever their payload.
use std::{env, fs};
use sunrise_core::jmath;

struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn edges() -> Vec<f64> {
    use std::f64::consts::PI;
    vec![
        0.0,
        -0.0,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1),
        1.0,
        -1.0,
        0.5,
        -0.5,
        2.0,
        -2.0,
        10.0,
        0.1,
        PI / 4.0,
        PI / 2.0,
        PI,
        3.0 * PI / 2.0,
        2.0 * PI,
        -PI / 2.0,
        -PI,
        f64::from((PI * 2.0) as f32),
        f64::from_bits(0x3e5 << 52),
        f64::from_bits(0x3e4 << 52),
        f64::from_bits(0x413 << 52),
        f64::from_bits(0x433 << 52),
        f64::from_bits(0x43e << 52),
        1e9,
        1e300,
        -1e300,
        709.782712893384,
        709.79,
        -745.1332191019411,
        -745.14,
        std::f64::consts::FRAC_PI_4,
        2.356194490192345,
        f64::MAX,
        -f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ]
}

fn range(f: &str) -> (f64, f64) {
    match f {
        "exp" => (-745.0, 710.0),
        "log" | "log10" | "cbrt" => (0.0, 1e6),
        "asin" | "acos" => (-1.0, 1.0),
        "pow" => (0.0, 16.0),
        _ => (-8.0 * std::f64::consts::PI, 8.0 * std::f64::consts::PI),
    }
}

fn input(s: &mut Stream, edges: &[f64], f: &str, i: u64, second: bool) -> f64 {
    // Mth's sine table: entry i, as Mth's static initializer computes it (no draws).
    if f == "mthtable" {
        return (i & 65535) as f64 / 10430.378350470453;
    }
    let (lo, hi) = range(f);
    match i & 3 {
        0 => {
            let u = (s.next() >> 11) as f64 * f64::from_bits(0x3ca << 52);
            if second && f == "pow" {
                -16.0 + u * 32.0
            } else {
                lo + u * (hi - lo)
            }
        }
        1 => {
            let bits = s.next();
            let exponent = 1023 - 60 + (bits >> 52) % 121;
            f64::from_bits((bits & 0x800F_FFFF_FFFF_FFFF) | (exponent << 52))
        }
        2 => f64::from_bits(s.next()),
        _ => {
            let edge = edges[(s.next() % edges.len() as u64) as usize];
            let nudge = (s.next() % 7) as i64 - 3;
            if edge.is_nan() || edge.is_infinite() {
                edge
            } else {
                f64::from_bits(edge.to_bits().wrapping_add(nudge as u64))
            }
        }
    }
}

type Variant = (&'static str, fn(f64, f64) -> f64);

fn variants(f: &str) -> Vec<Variant> {
    match f {
        "sin" => vec![
            ("jmath", |x, _| jmath::sin(x)),
            ("platform", |x, _| x.sin()),
            ("exact", |x, _| jmath::exact::sin(x)),
        ],
        "cos" => vec![
            ("jmath", |x, _| jmath::cos(x)),
            ("platform", |x, _| x.cos()),
            ("exact", |x, _| jmath::exact::cos(x)),
        ],
        "tan" => vec![
            ("jmath", |x, _| jmath::tan(x)),
            ("platform", |x, _| x.tan()),
        ],
        "exp" => vec![
            ("jmath", |x, _| jmath::exp(x)),
            ("platform", |x, _| x.exp()),
        ],
        "log" => vec![
            ("jmath", |x, _| jmath::log(x)),
            ("platform", |x, _| x.ln()),
            ("exact", |x, _| jmath::exact::log(x)),
            ("fdlibm", |x, _| jmath::fdlibm::log(x)),
        ],
        "log10" => vec![
            ("jmath", |x, _| jmath::log10(x)),
            ("platform", |x, _| x.log10()),
            ("fdlibm", |x, _| jmath::fdlibm::log10(x)),
        ],
        "pow" => vec![("jmath", jmath::pow), ("platform", |x, y| x.powf(y))],
        "cbrt" => vec![
            ("jmath", |x, _| jmath::cbrt(x)),
            ("platform", |x, _| x.cbrt()),
            ("fdlibm", |x, _| jmath::fdlibm::cbrt(x)),
        ],
        "atan2" => vec![("jmath", jmath::atan2), ("platform", |x, y| x.atan2(y))],
        "atan" => vec![
            ("jmath", |x, _| jmath::atan(x)),
            ("platform", |x, _| x.atan()),
        ],
        "asin" => vec![
            ("jmath", |x, _| jmath::asin(x)),
            ("platform", |x, _| x.asin()),
        ],
        "acos" => vec![
            ("jmath", |x, _| jmath::acos(x)),
            ("platform", |x, _| x.acos()),
        ],
        "hypot" => vec![
            ("jmath", jmath::hypot),
            ("platform", |x, y| x.hypot(y)),
            ("fdlibm", jmath::fdlibm::hypot),
        ],
        "mthtable" => vec![
            ("table", |x, _| {
                let i = (x * 10430.378350470453).round() as usize & 65535;
                f64::from(sunrise_core::geometry::trig::sine_table()[i])
            }),
            ("exact", |x, _| f64::from(jmath::exact::sin(x) as f32)),
        ],
        other => panic!("unknown function {other}"),
    }
}

/// Distance in ulps between two finite doubles of the same sign; u64::MAX otherwise.
fn ulps(a: f64, b: f64) -> u64 {
    if !a.is_finite() || !b.is_finite() || a.is_sign_negative() != b.is_sign_negative() {
        return u64::MAX;
    }
    a.to_bits().abs_diff(b.to_bits())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (f, seed, count, file) = (
        &args[1],
        args[2].parse::<u64>().unwrap(),
        args[3].parse::<u64>().unwrap(),
        &args[4],
    );
    let java = fs::read(file).expect("the JVM's results");
    assert_eq!(java.len() as u64, count * 8, "result count");
    let binary = matches!(f.as_str(), "pow" | "atan2" | "hypot");
    let (edges, variants) = (edges(), variants(f));
    let mut mismatches = vec![0u64; variants.len()];
    let mut worst = vec![0u64; variants.len()];
    let mut by_class = vec![[0u64; 4]; variants.len()];
    let mut samples: Vec<Vec<String>> = vec![Vec::new(); variants.len()];
    let mut s = Stream(seed);
    for i in 0..count {
        let x = input(&mut s, &edges, f, i, false);
        let y = if binary {
            input(&mut s, &edges, f, i, true)
        } else {
            0.0
        };
        let at = (i * 8) as usize;
        let expected = f64::from_bits(u64::from_le_bytes(java[at..at + 8].try_into().unwrap()));
        for (v, (_, compute)) in variants.iter().enumerate() {
            let got = compute(x, y);
            if got.to_bits() == expected.to_bits() || (got.is_nan() && expected.is_nan()) {
                continue;
            }
            mismatches[v] += 1;
            by_class[v][(i & 3) as usize] += 1;
            worst[v] = worst[v].max(ulps(got, expected));
            if samples[v].len() < 8 {
                samples[v].push(format!(
                    "{{\"x\":\"{:#018x}\",\"y\":\"{:#018x}\",\"jvm\":\"{:#018x}\",\"rust\":\"{:#018x}\"}}",
                    x.to_bits(), y.to_bits(), expected.to_bits(), got.to_bits()
                ));
            }
        }
    }
    for (v, (name, _)) in variants.iter().enumerate() {
        println!(
            "{{\"function\":\"{f}\",\"variant\":\"{name}\",\"seed\":{seed},\"count\":{count},\"mismatches\":{},\"by_class\":{:?},\"worst_ulps\":{},\"samples\":[{}]}}",
            mismatches[v],
            by_class[v],
            if worst[v] == u64::MAX {
                "null".to_string()
            } else {
                worst[v].to_string()
            },
            samples[v].join(",")
        );
    }
}
