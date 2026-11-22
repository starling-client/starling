//! A size-preserving, lossless staging layout for the network decoder's output.
//!
//! The wire type bytes serve as the tape's opcodes, including homogeneous list
//! element types. The decoder writes native counts and values, and string
//! records contain decoded UTF-16 units and their original footprints. There
//! are no per-node slots or offsets. Arrays use the same slab as everything
//! else; there is also no retained copy of the original bytes. The inverse
//! layout writes the exact packet back as it materializes the checked values,
//! including accepted noncanonical strings and signed floating-point zero.
use super::{
    ArrayKind, Compound, DecodeOutput, Error, JavaString, Limits, Tag, double_tag, float_tag,
    network_reader, read_network_with_reader, unwrap_list_element,
};

/// Stage one network root into the corresponding range of a copy of `wire`.
/// The destination has the same layout length, including its type byte. On
/// success it contains a checked tape root and this returns its type and length.
/// Only the prefix read by the decoder is touched; discard it on failure.
pub fn pack_network(wire: &[u8], copy: &mut [u8], limits: Limits) -> Result<(u8, usize), Error> {
    read_network_with_reader(&mut network_reader(wire, limits), &mut Pack(copy))
}

struct Pack<'a>(&'a mut [u8]);

impl DecodeOutput for Pack<'_> {
    type Value = u8;
    type List = ();
    type Map = ();
    type Key = ();

    fn scalar(&mut self, offset: usize, wire: &[u8], tag: Tag) -> u8 {
        let id = tag.id();
        let out = &mut self.0[offset..offset + wire.len()];
        match tag {
            Tag::End | Tag::Byte(_) => {}
            Tag::Short(n) => out.copy_from_slice(&n.to_ne_bytes()),
            Tag::Int(n) => out.copy_from_slice(&n.to_ne_bytes()),
            Tag::Long(n) => out.copy_from_slice(&n.to_ne_bytes()),
            Tag::Float(bits) => out.copy_from_slice(&bits.to_ne_bytes()),
            Tag::Double(bits) => out.copy_from_slice(&bits.to_ne_bytes()),
            Tag::String(value) => pack_string(out, wire, value),
            _ => unreachable!("the network decoder emits collections directly"),
        }
        id
    }

    fn array(&mut self, kind: ArrayKind, offset: usize, bytes: &[u8]) -> u8 {
        let width = kind.width();
        self.0[offset - 4..offset].copy_from_slice(&((bytes.len() / width) as u32).to_ne_bytes());
        swap_words(&mut self.0[offset..offset + bytes.len()], width);
        match kind {
            ArrayKind::Byte => 7,
            ArrayKind::Int => 11,
            ArrayKind::Long => 12,
        }
    }

    fn list(&mut self, offset: usize, len: usize) {
        self.0[offset..offset + 4].copy_from_slice(&(len as u32).to_ne_bytes());
    }

    fn push(&mut self, _: &mut (), _: u8) {}
    fn end_list(&mut self, _: ()) -> u8 {
        9
    }
    fn compound(&mut self) {}
    fn contains(&self, _: &(), _: &JavaString) -> bool {
        unreachable!("the network decoder does not use disk duplicate checking")
    }
    fn key(&mut self, offset: usize, wire: &[u8], key: JavaString) {
        pack_string(&mut self.0[offset..offset + wire.len()], wire, key);
    }
    fn insert(&mut self, _: &mut (), _: (), _: u8) -> bool {
        false // The network tape does not use the disk heap quota.
    }
    fn end_compound(&mut self, _: ()) -> u8 {
        10
    }
}

fn pack_string(out: &mut [u8], wire: &[u8], value: JavaString) {
    let (len, mut out) = out.split_at_mut(2);
    len.copy_from_slice(&((wire.len() - 2) as u16).to_ne_bytes());
    let mut original = &wire[2..];
    for unit in value.0 {
        // The authoritative decoder already matched and decoded these spans.
        // The original width is sufficient to recover *any* accepted encoding.
        let width = match original[0] {
            0..=127 => 1,
            192..=223 => 2,
            _ => 3,
        };
        let slot = take(&mut out, width);
        match slot {
            [b] => *b = unit as u8,
            [high, low] => {
                *high = 0x80 | (unit >> 8) as u8;
                *low = unit as u8;
            }
            [marker, a, b] => {
                *marker = 0xc0;
                [*a, *b] = unit.to_ne_bytes();
            }
            _ => unreachable!("each decoded UTF-16 unit has a footprint of one to three bytes"),
        }
        original = &original[width..];
    }
}

/// Materialize one root from the checked tape made by [`pack_network`],
/// restoring its wire bytes in place and advancing past the root. Only a
/// successful `pack_network` output, beginning at its root, may be passed in.
pub fn unpack_network(tape: &mut &mut [u8]) -> Tag {
    let opcode = take(tape, 1)[0];
    materialize(tape, opcode)
}

fn materialize(tape: &mut &mut [u8], opcode: u8) -> Tag {
    match opcode {
        0 => Tag::End,
        1 => Tag::Byte(take(tape, 1)[0] as i8),
        2 => Tag::Short(i16::from_ne_bytes(restore_word(tape))),
        3 => Tag::Int(i32::from_ne_bytes(restore_word(tape))),
        4 => Tag::Long(i64::from_ne_bytes(restore_word(tape))),
        5 => float_tag(u32::from_ne_bytes(restore_word(tape))),
        6 => double_tag(u64::from_ne_bytes(restore_word(tape))),
        7 => materialize_array(tape, ArrayKind::Byte),
        8 => Tag::String(materialize_string(tape)),
        9 => {
            let element = take(tape, 1)[0];
            let len = u32::from_ne_bytes(restore_word(tape));
            Tag::List(
                (0..len)
                    .map(|_| unwrap_list_element(materialize(tape, element)))
                    .collect(),
            )
        }
        10 => {
            let mut compound = Compound::new();
            loop {
                let field = take(tape, 1)[0];
                if field == 0 {
                    break;
                }
                let key = materialize_string(tape);
                compound.insert(key, materialize(tape, field));
            }
            Tag::Compound(compound)
        }
        11 => materialize_array(tape, ArrayKind::Int),
        12 => materialize_array(tape, ArrayKind::Long),
        _ => unreachable!("the tape was emitted by the authoritative decoder"),
    }
}

fn materialize_string(tape: &mut &mut [u8]) -> JavaString {
    let len = u16::from_ne_bytes(restore_word(tape));
    let mut bytes = take(tape, usize::from(len));
    let mut units = Vec::with_capacity(bytes.len());
    while let Some(&first) = bytes.first() {
        let unit = match first {
            0..=0x7f => u16::from(take(&mut bytes, 1)[0]),
            0x80..=0xbf => {
                let span = take(&mut bytes, 2);
                let unit = u16::from_be_bytes([span[0] & 0x3f, span[1]]);
                span.copy_from_slice(&[0xc0 | (unit >> 6) as u8, 0x80 | (unit & 63) as u8]);
                unit
            }
            _ => {
                let span = take(&mut bytes, 3);
                let unit = u16::from_ne_bytes([span[1], span[2]]);
                span.copy_from_slice(&[
                    0xe0 | (unit >> 12) as u8,
                    0x80 | ((unit >> 6) & 63) as u8,
                    0x80 | (unit & 63) as u8,
                ]);
                unit
            }
        };
        units.push(unit);
    }
    JavaString(units)
}

fn materialize_array(tape: &mut &mut [u8], kind: ArrayKind) -> Tag {
    let count = u32::from_ne_bytes(restore_word(tape)) as usize;
    let bytes = take(tape, count * kind.width());
    let tag = kind.tag(bytes, true);
    swap_words(bytes, kind.width());
    tag
}

fn swap_words(bytes: &mut [u8], width: usize) {
    if cfg!(target_endian = "little") && width != 1 {
        for word in bytes.chunks_exact_mut(width) {
            word.reverse();
        }
    }
}

fn restore_word<const N: usize>(tape: &mut &mut [u8]) -> [u8; N] {
    let bytes = take(tape, N);
    let native = (&*bytes).try_into().expect("fixed-width tape word");
    if cfg!(target_endian = "little") {
        bytes.reverse();
    }
    native
}

fn take<'a>(tape: &mut &'a mut [u8], len: usize) -> &'a mut [u8] {
    let (head, rest) = std::mem::take(tape).split_at_mut(len);
    *tape = rest;
    head
}
