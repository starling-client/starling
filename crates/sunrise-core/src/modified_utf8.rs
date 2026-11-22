//! Java's modified UTF-8, as `DataOutput.writeUTF` writes it and `DataInput.readUTF` reads it: each UTF-16
//! unit on its own (a surrogate pair is two 3-byte sequences, a lone surrogate one), and U+0000 as `C0 80`.
//! NBT uses it on disk and on the wire. `tools/test_modified_utf8_parity.py` checks it against JDK 25 and
//! `tools/modified_utf8_kit/` times it against HotSpot.
//!
//! The module uses only `std`, so the speed kit can compile byte-identical copies of it.

/// `writeUTF`'s limit on the encoded bytes, the most its unsigned 16-bit length prefix holds; above it,
/// `writeUTF` throws `UTFDataFormatException` and writes nothing.
pub const MAX_LEN: usize = 65_535;

/// The bytes `unit` takes: 1 for U+0001 to U+007F, 2 for U+0000 and U+0080 to U+07FF, 3 above.
#[inline(always)]
pub const fn unit_len(unit: u16) -> usize {
    1 + (unit.wrapping_sub(1) >= 0x7f) as usize + (unit >= 0x800) as usize
}

/// Whether `unit` takes one byte (U+0001 to U+007F), written as itself.
#[inline(always)]
const fn one_byte(unit: u16) -> bool {
    unit.wrapping_sub(1) < 0x7f
}

/// The encoded byte length of `units`, without the length prefix.
#[inline]
pub fn encoded_len(units: &[u16]) -> usize {
    // Summed in 16-bit lanes a block at a time (at most 3 x 256 per block), so that it vectorizes.
    let (blocks, rest) = units.as_chunks::<256>();
    let mut len = 0;
    for block in blocks {
        len += usize::from(block.iter().map(|&u| unit_len(u) as u16).sum::<u16>());
    }
    len + rest.iter().map(|&u| unit_len(u)).sum::<usize>()
}

/// The bytes of the longest run of whole units at the start of `units` that fits in `room` bytes: where
/// writing them one at a time into `room` bytes stops.
pub fn fitting_len(units: &[u16], room: usize) -> usize {
    let mut len = 0;
    for &u in units {
        let next = len + unit_len(u);
        if next > room {
            break;
        }
        len = next;
    }
    len
}

/// Appends the encoding of `units` to `out`, without a length prefix. `len` is [`encoded_len`]`(units)`, which
/// the caller has computed for the prefix. Allocates only if `out` lacks the room.
#[inline]
pub fn encode(units: &[u16], len: usize, out: &mut Vec<u8>) {
    debug_assert_eq!(len, encoded_len(units));
    if len == units.len() {
        // Every unit is one byte (only those keep the length equal).
        out.extend(units.iter().map(|&u| u as u8));
        return;
    }
    let start = out.len();
    out.resize(start + len, 0);
    encode_into(units, &mut out[start..]);
}

/// Writes the encoding of `units` into `dst`, which is exactly [`encoded_len`]`(units)` bytes long.
fn encode_into(units: &[u16], dst: &mut [u8]) {
    let mut j = 0;
    let mut rest = units;
    // Eight units at a time while 24 bytes are left: one ASCII test for all eight, else each unit's three
    // bytes written whole and the next unit's overwriting the ones it doesn't use.
    while let Some((chunk, tail)) = rest.split_first_chunk::<8>() {
        let Some(window) = dst.get_mut(j..j + 24) else {
            break;
        };
        let window: &mut [u8; 24] = window.try_into().unwrap();
        if chunk.iter().fold(true, |all, &u| all & one_byte(u)) {
            for (d, &u) in window.iter_mut().zip(chunk) {
                *d = u as u8;
            }
            j += 8;
        } else {
            let mut p = 0;
            for &u in chunk {
                let n = unit_len(u);
                let bytes = if n == 1 {
                    [u as u8, 0, 0]
                } else if n == 2 {
                    [0xc0 | (u >> 6) as u8, 0x80 | (u & 63) as u8, 0]
                } else {
                    [
                        0xe0 | (u >> 12) as u8,
                        0x80 | ((u >> 6) & 63) as u8,
                        0x80 | (u & 63) as u8,
                    ]
                };
                window[p..p + 3].copy_from_slice(&bytes);
                p += n;
            }
            j += p;
        }
        rest = tail;
    }
    for &u in rest {
        match unit_len(u) {
            1 => {
                dst[j] = u as u8;
                j += 1;
            }
            2 => {
                dst[j..j + 2].copy_from_slice(&[0xc0 | (u >> 6) as u8, 0x80 | (u & 63) as u8]);
                j += 2;
            }
            _ => {
                dst[j..j + 3].copy_from_slice(&[
                    0xe0 | (u >> 12) as u8,
                    0x80 | ((u >> 6) & 63) as u8,
                    0x80 | (u & 63) as u8,
                ]);
                j += 3;
            }
        }
    }
    debug_assert_eq!(j, dst.len());
}

/// `readUTF`'s decoding of `bytes` (the length prefix already read), accepting what it accepts, overlong
/// forms included; `None` where it throws `UTFDataFormatException`: a byte `10xxxxxx` or `1111xxxx` where a
/// sequence starts, a continuation byte that isn't `10xxxxxx`, or a sequence cut off at the end.
#[inline]
pub fn decode(bytes: &[u8]) -> Option<Vec<u16>> {
    // ASCII, the common text, widens in one pass that also ORs the bytes together; the OR's top bit then says
    // whether any byte wasn't ASCII. Testing first and widening after reads the input twice, which the speed
    // kit's ASCII rows measured at about a tenth of the time. Other text keeps the vector's room for the
    // per-sequence loop.
    let mut high = 0;
    let mut units: Vec<u16> = bytes
        .iter()
        .map(|&b| {
            high |= b;
            u16::from(b)
        })
        .collect();
    if high < 0x80 {
        return Some(units);
    }
    units.clear();
    decode_units(units, bytes)
}

/// Every unit takes at least one byte, so `bytes` never holds more units than the room made for them: the room
/// is `units`' capacity (`decode` reserved `bytes.len()`), brought into use once, and the loops store into
/// it by index. A `push` per unit tests the capacity, stores the length back and keeps a growth call in the
/// loop: more instructions and one more branch in every pass, which the speed kit's 1024-unit rows of
/// non-ASCII text measured at 4% to 12% of the time. Inlined into `decode` (one per copy in the speed
/// kit): out of line, the copies' identical loops would be merged by the linker and land at the same
/// address, and the kit could no longer average over their placements.
#[inline(always)]
fn decode_units(mut units: Vec<u16>, bytes: &[u8]) -> Option<Vec<u16>> {
    units.resize(bytes.len(), 0);
    let out = &mut units[..];
    let mut o = 0;
    let mut i = 0;
    // One sequence at a time, specialised by its lead byte's class: each branch predicts the string's own mix
    // (a uniform stretch runs one branch to the end), which is where HotSpot's per-byte loop beats a block
    // machinery that handles every width at once. The unit's bytes load as one word, so the loads and their
    // bounds checks are one per unit, and the unit is gathered from the word's bytes with masks and shifts,
    // as in the encoder. Four bytes of lookahead while they are left.
    while let Some(w) = bytes[i..].first_chunk::<4>() {
        let v = u32::from_le_bytes(*w);
        let lead = v as u8;
        if lead < 0x80 {
            out[o] = u16::from(v as u8);
            o += 1;
            i += 1;
        } else if lead < 0xe0 {
            // A lead `110xxxxx`, or a stray continuation `10xxxxxx` where a sequence starts: `v & 0xc0e0`
            // is the lead's class and its continuation's top bits in one word.
            if v & 0xc0e0 != 0x80c0 {
                return None;
            }
            out[o] = (((v & 0x1f) << 6) | ((v >> 8) & 0x3f)) as u16;
            o += 1;
            i += 2;
        } else if lead < 0xf0 {
            // The lead is `1110xxxx` here; both following bytes must be `10xxxxxx`, which is one test of
            // the words' two top-bit pairs.
            if (v >> 8) & 0xc0c0 != 0x8080 {
                return None;
            }
            out[o] = (((v & 0xf) << 12) | (((v >> 8) & 0x3f) << 6) | ((v >> 16) & 0x3f)) as u16;
            o += 1;
            i += 3;
        } else {
            // A lead `1111xxxx`, which no sequence here has.
            return None;
        }
    }
    // Up to three bytes left: at most one whole sequence, or one cut off, or none.
    while i < bytes.len() {
        let at = |k: usize| bytes.get(i + k).copied().unwrap_or(0);
        let (unit, size) = sequence(bytes[i], at(1), at(2))?;
        out[o] = unit;
        o += 1;
        i += size;
    }
    units.truncate(o);
    Some(units)
}

/// The sequence size for each lead byte's top four bits: 1 for `0xxxxxxx`, 2 for `110xxxxx`, 3 for `1110xxxx`,
/// 0 (refused) for `10xxxxxx` and `1111xxxx`.
const SIZES: u64 = 0x0322_0000_1111_1111;

/// One sequence led by `b0`, with the two bytes after it (0 past the end, which no sequence accepts as a
/// continuation): its unit and size, or `None` where `readUTF` refuses it. Branch-free but for the refusal.
#[inline(always)]
fn sequence(b0: u8, b1: u8, b2: u8) -> Option<(u16, usize)> {
    let size = ((SIZES >> ((b0 >> 4) * 4)) & 15) as usize;
    let c1 = b1 & 0xc0 == 0x80;
    let c2 = b2 & 0xc0 == 0x80;
    let ok = (size != 0) & (size < 2 || c1) & (size < 3 || c2);
    ok.then(|| (unit(b0, b1, b2), size))
}

/// The unit of the sequence that `b0` leads, `b1` and `b2` following it, for a sequence already checked.
#[inline(always)]
fn unit(b0: u8, b1: u8, b2: u8) -> u16 {
    let two = (u16::from(b0 & 0x1f) << 6) | u16::from(b1 & 0x3f);
    let three = (u16::from(b0 & 0x0f) << 12) | (u16::from(b1 & 0x3f) << 6) | u16::from(b2 & 0x3f);
    let wide = std::hint::select_unpredictable(b0 < 0xe0, two, three);
    std::hint::select_unpredictable(b0 < 0x80, u16::from(b0), wide)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `writeUTF`'s bytes for one unit of each width and for the edges between them (JDK 25).
    #[test]
    fn units_encode_as_write_utf_writes_them() {
        let cases: [(u16, &[u8]); 9] = [
            (0x0000, &[0xc0, 0x80]),
            (0x0001, &[0x01]),
            (0x007f, &[0x7f]),
            (0x0080, &[0xc2, 0x80]),
            (0x07ff, &[0xdf, 0xbf]),
            (0x0800, &[0xe0, 0xa0, 0x80]),
            (0xd800, &[0xed, 0xa0, 0x80]),
            (0xdfff, &[0xed, 0xbf, 0xbf]),
            (0xffff, &[0xef, 0xbf, 0xbf]),
        ];
        for (unit, bytes) in cases {
            // Alone, and among enough units to take the eight-unit path.
            for units in [vec![unit], [vec![b'a'.into(); 7], vec![unit; 9]].concat()] {
                let mut expected =
                    vec![b'a'; units.len() - units.iter().filter(|&&u| u == unit).count()];
                for _ in 0..units.len() - expected.len() {
                    expected.extend(bytes);
                }
                let mut out = vec![9];
                encode(&units, encoded_len(&units), &mut out);
                assert_eq!(out[1..], expected, "{unit:04x}");
                assert_eq!(decode(&expected), Some(units), "{unit:04x}");
            }
        }
    }

    /// `readUTF` accepts overlong forms and refuses a stray continuation byte, a `1111xxxx` lead, a bad
    /// continuation and a cut-off sequence (JDK 25).
    #[test]
    fn decoding_accepts_and_refuses_what_read_utf_does() {
        assert_eq!(decode(&[0xc0, 0x80]), Some(vec![0]));
        assert_eq!(decode(&[0xe0, 0x81, 0x81]), Some(vec![0x41]));
        assert_eq!(decode(&[0xc1, 0xbf, b'x']), Some(vec![0x7f, 0x78]));
        for bad in [
            &[0x80][..],
            &[0xf0, 0x80, 0x80, 0x80],
            &[0xc2, 0x41],
            &[0xe0, 0x80, 0xc0],
            &[b'a', 0xc2],
            &[0xe0, 0x80],
        ] {
            assert_eq!(decode(bad), None, "{bad:02x?}");
        }
    }

    #[test]
    fn the_fitting_length_stops_before_the_unit_that_overflows() {
        let units = [b'a'.into(), 0x800, 0];
        assert_eq!(fitting_len(&units, 3), 1);
        assert_eq!(fitting_len(&units, 4), 4);
        assert_eq!(fitting_len(&units, 6), 6);
    }
}
