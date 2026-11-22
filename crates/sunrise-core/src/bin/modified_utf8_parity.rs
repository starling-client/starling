//! Rust side of `tools/test_modified_utf8_parity.py`: `sunrise_core::modified_utf8` (and the NBT writers' handling
//! of a string too long for it) on the inputs `VanillaModifiedUtf8Oracle.java` gives `DataOutputStream.writeUTF`
//! and `DataInputStream.readUTF`. Both sides generate the inputs from the same SplitMix64 seed and print one FNV-1a
//! digest per chunk; `dump` prints a chunk record by record, to find the first difference.
//!
//! Usage: `sunrise-modified-utf8-parity MODE FIRST COUNT THREADS` (one line `chunk records digest` per chunk), or
//! `sunrise-modified-utf8-parity dump MODE CHUNK` (one line per record), or `... callers` (the writers' edge cases).
use std::{
    io::{self, BufWriter, Write},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use sunrise_core::{
    modified_utf8::{MAX_LEN, decode, encode, encoded_len},
    nbt::{JavaString, Limits, NamedTag, Tag, write_named, write_network},
};

const SEED: u64 = 0x5375_6e72_6973_6555;
/// Strings (or byte strings) per chunk of the random modes.
const RANDOM_CHUNK: u64 = 1 << 20;

struct SplitMix(u64);
impl SplitMix {
    fn chunk(mode: u64, chunk: u64) -> Self {
        Self(
            SEED ^ mode.wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ chunk.wrapping_mul(0xd1b5_4a32_d192_ed03),
        )
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// What one chunk's records feed: an FNV-1a 64 digest, or with `dump` one text line per record.
struct Sink {
    hash: u64,
    records: u64,
    dump: Option<String>,
}
impl Sink {
    fn new(dump: bool) -> Self {
        Self {
            hash: 0xcbf2_9ce4_8422_2325,
            records: 0,
            dump: dump.then(String::new),
        }
    }
    fn bytes(&mut self, bytes: &[u8]) {
        let mut h = self.hash;
        for &b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.hash = h;
    }
    /// One record: its input (for the dump) and its result bytes.
    fn record(&mut self, input: impl FnOnce() -> String, result: &[u8]) {
        self.records += 1;
        self.bytes(result);
        if let Some(dump) = &mut self.dump {
            use std::fmt::Write;
            let _ = write!(dump, "{} {} ", self.records - 1, input());
            for b in result {
                let _ = write!(dump, "{b:02x}");
            }
            dump.push('\n');
        }
    }
}

fn hex_units(units: &[u16]) -> String {
    units
        .iter()
        .map(|u| format!("{u:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}
fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `writeUTF`'s result: `01`, its length and its bytes; or `00` where it throws (the string is too long).
fn encode_record(sink: &mut Sink, units: &[u16], out: &mut Vec<u8>) {
    out.clear();
    let len = encoded_len(units);
    if len > MAX_LEN {
        out.push(0);
    } else {
        out.push(1);
        out.extend((len as u16).to_be_bytes());
        encode(units, len, out);
    }
    sink.record(|| hex_units(units), out);
}

/// `readUTF`'s result: `01`, the unit count (4 bytes) and the units; or `00` where it throws.
fn decode_record(sink: &mut Sink, bytes: &[u8], out: &mut Vec<u8>) {
    out.clear();
    match decode(bytes) {
        Some(units) => {
            out.push(1);
            out.extend((units.len() as u32).to_be_bytes());
            out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
        }
        None => out.push(0),
    }
    sink.record(|| hex_bytes(bytes), out);
}

/// A random UTF-16 string, as the oracle's `randomUnits` draws it.
fn random_units(rng: &mut SplitMix, units: &mut Vec<u16>) {
    units.clear();
    let r = rng.next();
    let len = if r & 0xff == 0 {
        (r >> 8) % 2048
    } else {
        (r >> 8) % 48
    } as usize;
    let profile = (r >> 40) & 3;
    while units.len() < len {
        let r = rng.next();
        let (class, v) = match profile {
            0 => (0, r >> 3),
            1 if r & 15 != 0 => (8, r >> 4),
            3 => (4 + (r & 3), r >> 3),
            _ => (r & 7, r >> 3),
        };
        match class {
            0 | 1 => units.push((v % 128) as u16),
            2 => units.push((0x80 + v % 0x780) as u16),
            3 => {
                let u = 0x800 + v % (0x10000 - 0x800 - 0x800);
                units.push((if u >= 0xd800 { u + 0x800 } else { u }) as u16);
            }
            4 => units.push((0xd800 + v % 1024) as u16),
            5 => units.push((0xdc00 + v % 1024) as u16),
            6 => units.extend([
                (0xd800 + v % 1024) as u16,
                (0xdc00 + (v >> 10) % 1024) as u16,
            ]),
            7 => units.push(v as u16),
            _ => units.push((1 + v % 127) as u16),
        }
    }
}

/// A random byte string for `readUTF`, as the oracle's `randomBytes` draws it.
fn random_bytes(rng: &mut SplitMix, bytes: &mut Vec<u8>) {
    bytes.clear();
    let r = rng.next();
    let len = if r & 0xff == 0 {
        (r >> 8) % 1024
    } else {
        (r >> 8) % 40
    } as usize;
    let profile = (r >> 40) & 3;
    let cont = |v: u64| (0x80 | (v & 0x3f)) as u8;
    while bytes.len() < len {
        let r = rng.next();
        let v = r >> 4;
        let class = match profile {
            0 => 15,
            1 if r & 63 != 0 => 4 + v % 3,
            2 => 4 + v % 3,
            _ => r & 15,
        };
        match class {
            0..=4 => bytes.push((v % 128) as u8),
            5 => bytes.extend([(0xc0 | ((v >> 2) & 0x1f)) as u8, cont(v >> 7)]),
            6 => bytes.extend([
                (0xe0 | ((v >> 2) & 0x0f)) as u8,
                cont(v >> 6),
                cont(v >> 12),
            ]),
            7 => bytes.push(cont(v >> 2)),
            8 => bytes.push((0xc0 | ((v >> 2) & 0x1f)) as u8),
            9 => bytes.push((0xe0 | ((v >> 2) & 0x0f)) as u8),
            10 | 11 => bytes.push((0xf0 | ((v >> 2) & 0x0f)) as u8),
            12 => bytes.extend([(0xc0 | ((v >> 2) & 0x1f)) as u8, (v >> 7) as u8]),
            13 => bytes.extend([
                (0xe0 | ((v >> 2) & 0x0f)) as u8,
                cont(v >> 6),
                (v >> 12) as u8,
            ]),
            _ => bytes.push(v as u8),
        }
    }
    bytes.truncate(len);
}

/// One chunk of `mode`, fed to `sink`.
fn chunk(mode: &str, c: u64, sink: &mut Sink) -> Result<(), String> {
    let mut out = Vec::with_capacity(8192);
    let mut units = Vec::with_capacity(4096);
    let mut bytes = Vec::with_capacity(4096);
    match mode {
        // The empty string and every single unit.
        "enc-singles" => {
            encode_record(sink, &[], &mut out);
            for u in 0..=0xffff {
                encode_record(sink, &[u as u16], &mut out);
            }
        }
        // Chunk c: every pair whose high surrogate is U+D800 + c, and that surrogate alone, before and after
        // ASCII, doubled, and after each low surrogate (reversed).
        "enc-surrogates" => {
            let h = 0xd800 + c as u16;
            for l in 0xdc00..=0xdfff {
                encode_record(sink, &[h, l], &mut out);
                encode_record(sink, &[l, h], &mut out);
            }
            for s in [
                [h].as_slice(),
                &[h, b'a'.into()],
                &[b'a'.into(), h],
                &[h, h],
            ] {
                encode_record(sink, s, &mut out);
            }
            let l = 0xdc00 + c as u16;
            for s in [
                [l].as_slice(),
                &[l, b'a'.into()],
                &[b'a'.into(), l],
                &[l, l],
            ] {
                encode_record(sink, s, &mut out);
            }
        }
        // Chunk c: every ordered pair whose first unit is 64c to 64c + 63.
        "enc-pairs" => {
            for a in c * 64..c * 64 + 64 {
                for b in 0..=0xffff {
                    encode_record(sink, &[a as u16, b as u16], &mut out);
                }
            }
        }
        "enc-random" => {
            let mut rng = SplitMix::chunk(1, c);
            for _ in 0..RANDOM_CHUNK {
                random_units(&mut rng, &mut units);
                encode_record(sink, &units, &mut out);
            }
        }
        // The 65,535-byte edge: strings of 65,533 to 65,538 encoded bytes, in 1-, 2- and 3-byte units.
        "enc-edge" => {
            for target in 65_533..=65_538usize {
                for (unit, n) in [
                    (b'x' as u16, 1),
                    (0, 2),
                    (0x7ff, 2),
                    (0x800, 3),
                    (0xd800, 3),
                ] {
                    units.clear();
                    units.resize(target / n, unit);
                    units.resize(units.len() + target % n, b'y'.into());
                    encode_record(sink, &units, &mut out);
                }
            }
        }
        // Chunk 0: every 1- and 2-byte sequence; chunk 1 + b: every 3-byte sequence starting with byte b.
        "dec-exhaustive" => {
            if c == 0 {
                for a in 0..=0xff {
                    decode_record(sink, &[a], &mut out);
                }
                for a in 0..=0xffffu16 {
                    decode_record(sink, &a.to_be_bytes(), &mut out);
                }
            } else {
                let a = (c - 1) as u8;
                for b in 0..=0xffffu16 {
                    let [b1, b2] = b.to_be_bytes();
                    decode_record(sink, &[a, b1, b2], &mut out);
                }
            }
        }
        "dec-random" => {
            let mut rng = SplitMix::chunk(2, c);
            for _ in 0..RANDOM_CHUNK {
                random_bytes(&mut rng, &mut bytes);
                decode_record(sink, &bytes, &mut out);
            }
        }
        // Random strings' own encodings (at most 65,535 bytes), decoded.
        "dec-valid" => {
            let mut rng = SplitMix::chunk(3, c);
            for _ in 0..RANDOM_CHUNK {
                random_units(&mut rng, &mut units);
                bytes.clear();
                encode(&units, encoded_len(&units), &mut bytes);
                // The input joins the digest, so that two sides building different bytes can't agree on the units.
                sink.bytes(&bytes);
                decode_record(sink, &bytes, &mut out);
            }
        }
        _ => return Err(format!("unknown mode {mode}")),
    }
    Ok(())
}

/// The NBT writers on the edge strings, as the full records the oracle's `callers` prints for `writeUTF`'s bytes: one
/// line per string, `target unit disk=<hex> network=<hex or refused>`. The disk writer writes a string too long for
/// modified UTF-8 empty (#5064) and the network writer refuses it.
fn callers() -> String {
    let mut lines = String::new();
    for target in 65_533..=65_538usize {
        for (unit, n) in [(b'x' as u16, 1), (0, 2), (0x800, 3)] {
            let mut units = vec![unit; target / n];
            units.resize(units.len() + target % n, b'y'.into());
            let string = Tag::String(JavaString(units));
            let disk = write_named(
                &NamedTag {
                    name: JavaString::default(),
                    tag: string.clone(),
                },
                Limits::saved(),
            )
            .map_or_else(|_| "refused".to_owned(), |d| hex_bytes(&d));
            let network = write_network(&string, Limits::saved())
                .map_or_else(|_| "refused".to_owned(), |w| hex_bytes(&w));
            lines += &format!("{target} {unit:04x} disk={disk} network={network}\n");
        }
    }
    lines
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = io::stdout();
    let mut out = BufWriter::new(out.lock());
    let number = |i: usize| -> Result<u64, String> {
        args.get(i)
            .ok_or("missing argument")?
            .parse()
            .map_err(|_| "invalid number".to_owned())
    };
    match args.first().map(String::as_str) {
        Some("callers") => out
            .write_all(callers().as_bytes())
            .map_err(|e| e.to_string())?,
        Some("dump") => {
            let mut sink = Sink::new(true);
            chunk(&args[1], number(2)?, &mut sink)?;
            out.write_all(sink.dump.unwrap().as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Some(mode) => {
            let (first, count, threads) = (number(1)?, number(2)?, number(3)?.max(1));
            let next = AtomicU64::new(first);
            let results = Mutex::new(Vec::new());
            let error = Mutex::new(None);
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    scope.spawn(|| {
                        loop {
                            let c = next.fetch_add(1, Ordering::Relaxed);
                            if c >= first + count {
                                break;
                            }
                            let mut sink = Sink::new(false);
                            if let Err(e) = chunk(mode, c, &mut sink) {
                                *error.lock().unwrap() = Some(e);
                                break;
                            }
                            results.lock().unwrap().push((c, sink.records, sink.hash));
                        }
                    });
                }
            });
            if let Some(e) = error.into_inner().unwrap() {
                return Err(e);
            }
            let mut results = results.into_inner().unwrap();
            results.sort_unstable();
            for (c, records, hash) in results {
                writeln!(out, "{c} {records} {hash:016x}").map_err(|e| e.to_string())?;
            }
        }
        None => return Err("usage: MODE FIRST COUNT THREADS | dump MODE CHUNK | callers".into()),
    }
    out.flush().map_err(|e| e.to_string())
}
