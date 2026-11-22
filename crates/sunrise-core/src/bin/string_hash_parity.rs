//! `String.hashCode` probe for tools/test_string_hash_parity.py. It generates the same strings as
//! `VanillaStringHashOracle` from the same seeds and prints a streaming digest of the hashes per
//! chunk, so that a difference localizes to a chunk; `dump` writes one chunk's hashes in full. Each
//! string that is valid UTF-16 is also hashed as a Rust `str` map key (`JavaKey::java_hash`), which
//! must agree with `java_string_hash` on its UTF-16 units.
//!
//! ```text
//! sunrise-string-hash-parity exhaustive THREADS
//! sunrise-string-hash-parity random SEED FIRST_CHUNK CHUNKS PER_CHUNK THREADS
//! sunrise-string-hash-parity dump exhaustive CHUNK FILE
//! sunrise-string-hash-parity dump random SEED CHUNK PER_CHUNK FILE
//! ```
//!
//! Exhaustive chunk 0 is the empty string then every one-unit string; chunk `1 + h` is every
//! two-unit string starting with unit `h`.
use std::io::{BufWriter, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_core::{java_hash_map::JavaKey, random::java_string_hash};

const EXHAUSTIVE_CHUNKS: u64 = 1 + 65_536;
const MAX_LEN: usize = 600;

struct SplitMix(u64);
impl SplitMix {
    fn new(seed: u64, chunk: u64) -> Self {
        Self(seed ^ chunk.wrapping_mul(0xD1B5_4A32_D192_ED03))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// One random string into `out`, its length returned: 0 to 600 units, all ASCII, all Latin-1, any
/// 16-bit units, or a mix of ASCII, Latin-1, BMP, surrogate pairs and lone surrogates.
fn random_string(rng: &mut SplitMix, out: &mut [u16; MAX_LEN]) -> usize {
    let len = (rng.next() % (MAX_LEN as u64 + 1)) as usize;
    let mode = rng.next() & 3;
    let mut i = 0;
    while i < len {
        let r = rng.next();
        let b = (r >> 8) as u16;
        let unit = match mode {
            0 => b & 0x7F,
            1 => b & 0xFF,
            2 => b,
            _ => match r & 7 {
                0..=2 => b & 0x7F,
                3 => b & 0xFF,
                4 => b,
                5 if i + 1 < len => {
                    out[i] = 0xD800 | (b & 0x3FF);
                    i += 1;
                    0xDC00 | ((r >> 24) as u16 & 0x3FF)
                }
                5 => b & 0x7F,
                6 => 0xD800 | (b & 0x3FF),
                _ => 0xDC00 | (b & 0x3FF),
            },
        };
        out[i] = unit;
        i += 1;
    }
    len
}

#[derive(Default)]
struct Chunk {
    count: u64,
    digest: u64,
    str_checked: u64,
    str_differed: u64,
}
impl Chunk {
    fn new() -> Self {
        Self {
            digest: 0xCBF2_9CE4_8422_2325,
            ..Self::default()
        }
    }
    fn add(&mut self, units: &[u16], hash: i32) {
        self.count += 1;
        self.digest = (self.digest ^ u64::from(hash as u32)).wrapping_mul(0x0100_0000_01B3);
        // The &str entry on every string that is valid UTF-16, decoded on the stack.
        let mut bytes = [0u8; 4 * MAX_LEN];
        let mut n = 0;
        for c in char::decode_utf16(units.iter().copied()) {
            let Ok(c) = c else { return };
            n += c.encode_utf8(&mut bytes[n..]).len();
        }
        let text = std::str::from_utf8(&bytes[..n]).expect("encoded from chars");
        self.str_checked += 1;
        if text.java_hash() != hash {
            self.str_differed += 1;
        }
    }
}

/// Calls `each(units, hash)` for every string of `chunk`.
fn exhaustive_chunk(chunk: u64, mut each: impl FnMut(&[u16], i32)) {
    if chunk == 0 {
        each(&[], java_string_hash([]));
        for a in 0..=u16::MAX {
            each(&[a], java_string_hash([a]));
        }
    } else {
        let a = (chunk - 1) as u16;
        for b in 0..=u16::MAX {
            let s = [a, b];
            each(&s, java_string_hash(s));
        }
    }
}

fn random_chunk(seed: u64, chunk: u64, per_chunk: u64, mut each: impl FnMut(&[u16], i32)) {
    let mut rng = SplitMix::new(seed, chunk);
    let mut buffer = [0u16; MAX_LEN];
    for _ in 0..per_chunk {
        let len = random_string(&mut rng, &mut buffer);
        let s = &buffer[..len];
        each(s, java_string_hash(s.iter().copied()));
    }
}

/// Runs `chunks` on `threads` threads, printing `TAG chunk count digest` in chunk order and then
/// `S checked differed` for the &str entry.
fn digests(
    tag: &str,
    first: u64,
    chunks: u64,
    threads: usize,
    run: impl Fn(u64, &mut Chunk) + Sync,
) {
    let next = AtomicU64::new(first);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let chunk = next.fetch_add(1, Ordering::Relaxed);
                    if chunk >= first + chunks {
                        break;
                    }
                    let mut c = Chunk::new();
                    run(chunk, &mut c);
                    results.lock().unwrap().push((chunk, c));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r.0);
    let mut out = BufWriter::new(std::io::stdout().lock());
    let (mut checked, mut differed) = (0, 0);
    for (chunk, c) in &results {
        writeln!(out, "{tag} {chunk} {} {:016x}", c.count, c.digest).unwrap();
        checked += c.str_checked;
        differed += c.str_differed;
    }
    writeln!(out, "S {checked} {differed}").unwrap();
}

fn dump(file: &str, fill: impl FnOnce(&mut dyn FnMut(&[u16], i32))) {
    let mut out = BufWriter::new(std::fs::File::create(file).expect("dump file"));
    let mut index = 0u64;
    fill(&mut |units: &[u16], hash: i32| {
        let hex: Vec<String> = units.iter().map(|u| format!("{u:04x}")).collect();
        writeln!(out, "{index} {} {hash}", hex.join(",")).unwrap();
        index += 1;
    });
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n = |i: usize| -> u64 { args[i].parse().expect("a number") };
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["exhaustive", _] => digests("E", 0, EXHAUSTIVE_CHUNKS, n(1) as usize, |chunk, c| {
            exhaustive_chunk(chunk, |s, h| c.add(s, h))
        }),
        ["random", _, _, _, _, _] => {
            let (seed, per_chunk) = (n(1), n(4));
            digests("R", n(2), n(3), n(5) as usize, |chunk, c| {
                random_chunk(seed, chunk, per_chunk, |s, h| c.add(s, h))
            })
        }
        ["dump", "exhaustive", _, file] => dump(file, |each| exhaustive_chunk(n(2), each)),
        ["dump", "random", _, _, _, file] => {
            dump(file, |each| random_chunk(n(2), n(3), n(4), each))
        }
        _ => {
            eprintln!("usage: see the module documentation");
            std::process::exit(2);
        }
    }
}
