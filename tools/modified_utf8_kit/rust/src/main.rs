//! The Rust side of the modified UTF-8 speed kit (`tools/modified_utf8_kit/run.py` builds it in `.local/`).
//!
//!   kit gen DIR            the input sets, one file per set (`<set>.units`), which the JVM reads too
//!   kit bench OP SET DIR   every copy's loop for OP (`encode`, `decode`) on SET: one JSON line per copy
//!   kit check DIR          every copy gives copy 0's bytes and units back
//!
//! `copies.rs` and `copies/` are written by run.py: byte-identical copies of `sunrise_core::modified_utf8`, each with
//! its own loops, at different code offsets.
mod copies;

use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

/// Counts heap allocations, so that the bench can report the ones its timed loops make.
struct Counting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

/// Units per set: the strings of a set add up to about this many.
const SET_UNITS: usize = 16_384;
const MIXES: [&str; 4] = ["ascii", "latin", "cjk", "emoji"];
const LENGTHS: [usize; 3] = [8, 64, 1024];

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// One string of `mix`: `ascii` printable ASCII; `latin` three units in four from U+0080 to U+07FF (2 bytes), the
/// rest ASCII; `cjk` three in four from U+4E00 to U+9FFF (3 bytes); `emoji` half surrogate pairs (U+1F600 to
/// U+1F64F), half ASCII.
fn string(rng: &mut SplitMix, mix: &str, len: usize) -> Vec<u16> {
    let mut units = Vec::with_capacity(len + 1);
    while units.len() < len {
        let r = rng.next();
        let ascii = (0x20 + (r >> 8) % 95) as u16;
        match mix {
            "ascii" => units.push(ascii),
            "latin" if r & 3 != 0 => units.push((0x80 + (r >> 8) % 0x780) as u16),
            "cjk" if r & 3 != 0 => units.push((0x4e00 + (r >> 8) % 0x5200) as u16),
            "emoji" if r & 1 != 0 && units.len() + 1 < len => {
                units.extend([0xd83d, (0xde00 + (r >> 8) % 0x50) as u16])
            }
            _ => units.push(ascii),
        }
    }
    units
}

fn read_set(dir: &str, set: &str) -> Vec<Vec<u16>> {
    let bytes = std::fs::read(format!("{dir}/{set}.units")).expect("run `kit gen` first");
    let mut strings = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let n = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        strings.push(
            bytes[at..at + 2 * n]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect(),
        );
        at += 2 * n;
    }
    strings
}

/// `writeUTF` of every string, one after another (the bytes `decode` reads).
fn encoded(strings: &[Vec<u16>]) -> Vec<u8> {
    let mut out = Vec::new();
    copies::ENCODE[0].1(strings, &mut out);
    out
}

fn median_and_p99(mut t: Vec<f64>) -> (f64, f64) {
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (t[t.len() / 2], t[t.len() * 99 / 100])
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The fillers are referenced as the loops are, so that the compiler emits them in the same (name) order, each
    // right before its copy's loops.
    black_box(copies::FILLERS);
    match args.first().map(String::as_str) {
        Some("gen") => {
            for (m, mix) in MIXES.iter().enumerate() {
                for len in LENGTHS {
                    let mut rng = SplitMix(0x6d75_7466_385f_6b69 ^ ((m as u64) << 32) ^ len as u64);
                    let mut file = Vec::new();
                    for _ in 0..SET_UNITS / len {
                        let units = string(&mut rng, mix, len);
                        file.extend((units.len() as u32).to_le_bytes());
                        file.extend(units.iter().flat_map(|u| u.to_le_bytes()));
                    }
                    std::fs::write(format!("{}/{mix}-{len}.units", args[1]), file).unwrap();
                }
            }
        }
        Some("check") => {
            for mix in MIXES {
                for len in LENGTHS {
                    let strings = read_set(&args[1], &format!("{mix}-{len}"));
                    let bytes = encoded(&strings);
                    for (copy, encode) in copies::ENCODE {
                        let mut out = Vec::new();
                        encode(&strings, &mut out);
                        assert!(out == bytes, "copy {copy} encodes {mix}-{len} differently");
                    }
                    for (copy, decode) in copies::DECODE {
                        let mut out = vec![Vec::new(); strings.len()];
                        decode(&bytes, &mut out);
                        assert!(
                            out == strings,
                            "copy {copy} decodes {mix}-{len} differently"
                        );
                    }
                }
            }
            println!("every copy agrees");
        }
        Some("bench") => {
            let (op, set, dir) = (&args[1], &args[2], &args[3]);
            let strings = read_set(dir, set);
            let bytes = encoded(&strings);
            let mut out = Vec::with_capacity(bytes.len());
            let mut slots = vec![Vec::new(); strings.len()];
            let n = strings.len() as f64;
            let mut run = |copy: usize| -> (f64, f64, u64) {
                let mut once = || {
                    if op == "encode" {
                        black_box(copies::ENCODE[copy].1(
                            black_box(&strings),
                            black_box(&mut out),
                        ));
                    } else {
                        black_box(copies::DECODE[copy].1(
                            black_box(&bytes),
                            black_box(&mut slots),
                        ));
                    }
                };
                for _ in 0..200 {
                    once();
                }
                // The timings go into a vector made beforehand, so that only the loop's own allocations count.
                let mut t = vec![0.0f64; 400];
                let before = ALLOCATIONS.load(Ordering::Relaxed);
                for slot in t.iter_mut() {
                    let s = Instant::now();
                    once();
                    *slot = s.elapsed().as_nanos() as f64 / n;
                }
                let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
                let (median, p99) = median_and_p99(t);
                (median, p99, allocations)
            };
            for copy in 0..copies::ENCODE.len() {
                let (median, p99, allocations) = run(copy);
                println!(
                    "{{\"copy\":{copy},\"median_ns\":{median:.4},\"p99_ns\":{p99:.4},\"allocations\":{allocations}}}"
                );
            }
        }
        _ => eprintln!("usage: kit gen DIR | bench OP SET DIR | check DIR"),
    }
}
