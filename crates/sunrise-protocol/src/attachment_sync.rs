//! Attachment sync: the values of a native mod's synced attachments, sent by a
//! Sunrise server to the Sunrise clients that see their holder
//! (docs/modding/src/how-to/native-mods.md, "Server attachments").
//!
//! A payload is the holder (a varint kind and its fields), whether the
//! client's copy of the holder is replaced, then each change: the attachment's
//! namespace and name and its value as an NBT compound, or no tag for a
//! removal.
//!
//! The channel is an engine declaration in the negotiated payload table:
//! only a peer that offers it at [`VERSION`] is sent anything, and a message
//! is bounded by the connection's frame limits, not by a body cap of its own.
use {
    sunrise_core::{nbt, nbt::Tag},
    sunrise_mod_runtime::attachments::{AttachmentSync, Holder},
};

use crate::{Error, Result, buf::Reader};
use sunrise_core::vanilla_bytes::Writer;

mod staged;
pub use staged::StagedMessage;

/// The custom payload channel, clientbound only: a client sends nothing back.
pub const CHANNEL: &str = "sunrise:attachment_sync";
/// The channel's exact declaration version in the negotiated table.
pub const VERSION: &str = "1";
/// Characters in a namespace or a name at most.
pub const MAX_NAME: usize = 256;

/// The bounds of one value, the same when the server writes it and the client reads it,
/// so a value the server sends is never one the client discards: the engine's NBT defaults
/// (64 MiB, 1,000,000 nodes, depth 512, the depth vanilla's `NbtAccounter` keeps). They are
/// looser than the connection's frame limits, which bound the whole message; the network
/// quota of `Reader::nbt` (2 MiB), meant for vanilla's untrusted packets, does not apply.
pub fn value_limits() -> nbt::Limits {
    nbt::Limits::default()
}

/// A holder's changes as they travel.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub holder: Holder,
    /// The changes are the holder's whole synced state: drop what the client
    /// holds first.
    pub replace: bool,
    pub changes: Vec<AttachmentSync>,
}

/// A message's payload; an error for a holder it cannot write, a name past
/// [`MAX_NAME`], or a value that is not an encodable compound. Its size is the
/// sending connection's to bound (its frame limits).
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    encode_changes(message, |_, error| Err(error)).map(|(body, _)| body)
}

/// Report each refused change without dropping its holder's other changes. An empty replace
/// still goes out; a delta with no accepted changes has no message.
pub fn encode_retaining(
    message: &Message,
    mut refused: impl FnMut(&AttachmentSync, Error),
) -> Result<Option<Vec<u8>>> {
    let (body, count) = encode_changes(message, |change, error| {
        refused(change, error);
        Ok(())
    })?;
    Ok((message.replace || count != 0).then_some(body))
}

fn encode_changes(
    message: &Message,
    mut refused: impl FnMut(&AttachmentSync, Error) -> Result<()>,
) -> Result<(Vec<u8>, usize)> {
    let mut w = Writer::new();
    match message.holder {
        Holder::Entity(id) => {
            w.varint(0);
            w.varint(id as i32);
        }
        Holder::Player(uuid) => {
            w.varint(1);
            w.uuid(uuid);
        }
        Holder::BlockEntity(pos) => {
            w.varint(2);
            pos.iter().for_each(|&axis| w.i32(axis));
        }
        Holder::Chunk(pos) => {
            w.varint(3);
            pos.iter().for_each(|&axis| w.i32(axis));
        }
        Holder::Level => w.varint(4),
        _ => return Err(Error::new("attachment sync: unknown holder")),
    }
    w.bool(message.replace);
    let total = message.changes.len();
    let count_start = w.data.len();
    w.varint(total as i32);
    let count_end = w.data.len();
    let mut count = 0;
    for change in &message.changes {
        let start = w.data.len();
        match write_change(&mut w, change) {
            Ok(()) => count += 1,
            Err(error) => {
                w.data.truncate(start);
                refused(change, error)?;
            }
        }
    }
    if count != total {
        let mut accepted = Writer::new();
        accepted.varint(count as i32);
        drop(w.data.splice(count_start..count_end, accepted.data));
    }
    Ok((w.data, count))
}

fn write_change(w: &mut Writer, change: &AttachmentSync) -> Result<()> {
    for name in [&change.namespace, &change.name] {
        let name = name
            .to_string()
            .map_err(|_| Error::new("attachment sync: name is not UTF-16"))?;
        if name.encode_utf16().count() > MAX_NAME {
            return Err(Error::new(format!(
                "attachment sync: name {name} is too long"
            )));
        }
        w.string(&name);
    }
    match &change.value {
        Some(value) => {
            let tag = Tag::Compound(value.clone());
            let bytes = nbt::write_network(&tag, value_limits())
                .map_err(|e| Error::new(format!("attachment sync: value: {e}")))?;
            w.bytes(&bytes);
        }
        None => w.bytes(&[0]),
    }
    Ok(())
}

/// A payload's message, materialized immediately by callers that don't queue it.
pub fn decode(data: &[u8]) -> Result<Message> {
    let (holder, replace, _, changes) = decode_changes(data, |r, [namespace, name]| {
        let value = match r.nbt_within(value_limits())? {
            Tag::Compound(value) => Some(value),
            Tag::End => None,
            _ => return Err(Error::new("attachment sync: value is not a compound")),
        };
        Ok(AttachmentSync {
            namespace: namespace.text.as_str().into(),
            name: name.text.as_str().into(),
            value,
        })
    })?;
    Ok(Message {
        holder,
        replace,
        changes,
    })
}

/// Decode at packet ingress without allocating the holder's NBT maps there.
pub fn decode_staged(
    payload: crate::packets::configuration::CustomPayload,
) -> Result<StagedMessage> {
    staged::stage(payload)
}

struct AttachmentName {
    text: String,
    prefix: std::ops::Range<usize>,
    bytes: usize,
}

impl AttachmentName {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let start = r.position();
        let (text, bytes) = r.string_field(MAX_NAME)?;
        Ok(Self {
            text,
            prefix: start..r.position() - bytes,
            bytes,
        })
    }
}

fn decode_changes<T>(
    data: &[u8],
    mut change: impl FnMut(&mut Reader<'_>, [AttachmentName; 2]) -> Result<T>,
) -> Result<(Holder, bool, usize, Vec<T>)> {
    let mut r = Reader::new(data);
    let holder = match r.varint()? {
        0 => Holder::Entity(r.varint()? as u32),
        1 => Holder::Player(r.uuid()?),
        2 => Holder::BlockEntity([r.i32()?, r.i32()?, r.i32()?]),
        3 => Holder::Chunk([r.i32()?, r.i32()?]),
        4 => Holder::Level,
        kind => return Err(Error::new(format!("attachment sync: holder kind {kind}"))),
    };
    let replace = r.bool()?;
    let count = r.count(r.remaining())?;
    let start = r.position();
    let changes = (0..count)
        .map(|_| {
            let names = [AttachmentName::read(&mut r)?, AttachmentName::read(&mut r)?];
            change(&mut r, names)
        })
        .collect::<Result<_>>()?;
    if r.remaining() != 0 {
        return Err(Error::new("attachment sync: trailing bytes"));
    }
    Ok((holder, replace, start, changes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes() -> Vec<AttachmentSync> {
        vec![
            AttachmentSync {
                namespace: "mana".into(),
                name: "pool".into(),
                value: Some([("value".into(), Tag::Int(12))].into()),
            },
            AttachmentSync {
                namespace: "mana".into(),
                name: "gone".into(),
                value: None,
            },
        ]
    }

    #[test]
    fn every_holder_round_trips_with_either_replace_and_a_removal() {
        let expected: nbt::Compound = [
            ("value".into(), Tag::Int(12)),
            (
                "mixed".into(),
                Tag::List(vec![
                    Tag::Short(-5),
                    Tag::String(nbt::JavaString(vec![0, 0xd800, 0x20ac])),
                    Tag::compound([("\0", Tag::Byte(-3))]),
                    Tag::IntArray(vec![-11, 21]),
                    Tag::LongArray(vec![i64::MIN, i64::MAX]),
                    Tag::Long(-8),
                    Tag::Float(0x7fc0_0003),
                    Tag::Double(0x7ff8_0000_0000_0005),
                ]),
            ),
        ]
        .into();
        let holders = [
            Holder::Entity(4_000_000_000),
            Holder::Player(0x0123_4567_89ab_cdef_0011_2233_4455_6677),
            Holder::BlockEntity([-30_000_000, -64, 29_999_999]),
            Holder::Chunk([-5, 7]),
            Holder::Level,
        ];
        for holder in holders {
            for replace in [true, false] {
                let mut changes = changes();
                changes[0].value = Some(expected.clone());
                let message = Message {
                    holder,
                    replace,
                    changes,
                };
                assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
            }
        }
    }

    #[test]
    fn staged_and_eager_use_the_same_network_nbt_values_and_errors() {
        // The namespace uses Java's surrogate replacement and a nonminimal
        // five-byte length; the name is one supplementary character. The list
        // has two empty compounds and a mixed-list wrapper with a repeated key.
        let header = [
            4, 1, 1, 0x83, 0x80, 0x80, 0x80, 0x70, 0xed, 0xa0, 0x80, 4, 0xf0, 0x9f, 0x98, 0x80,
        ];
        #[rustfmt::skip]
        let value = [
            10, 9, 0, 1, b'l', 10, 0, 0, 0, 3, 0, 1, 0, 0, 1, 1, 0, 0, 2, 0, 0,
            5, 0, 1, b'f', 128, 0, 0, 0,
            8, 0, 1, b's', 0, 8, 0xc0, 0x80, 0xed, 0xa0, 0x80, 0xe0, 0x81, 0x81,
            6, 0, 1, b'd', 128, 0, 0, 0, 0, 0, 0, 0,
            4, 0, 1, b'v', 128, 0, 0, 0, 0, 0, 0, 7,
            7, 0, 1, b'b', 0, 0, 0, 2, 0xde, 0xad,
            11, 0, 1, b'i', 0, 0, 0, 1, 1, 2, 3, 4,
            12, 0, 1, b'a', 0, 0, 0, 1, 128, 0, 0, 0, 0, 0, 0, 7,
            0,
        ];
        let expected = Message {
            holder: Holder::Level,
            replace: true,
            changes: vec![AttachmentSync {
                namespace: "�".into(),
                name: "😀".into(),
                value: Some(
                    [
                        (
                            "l".into(),
                            Tag::List(vec![
                                Tag::Compound(nbt::Compound::new()),
                                Tag::Byte(2),
                                Tag::Compound(nbt::Compound::new()),
                            ]),
                        ),
                        ("f".into(), Tag::Float(0)),
                        (
                            "s".into(),
                            Tag::String(nbt::JavaString(vec![0, 0xd800, 65])),
                        ),
                        ("d".into(), Tag::Double(0)),
                        ("v".into(), Tag::Long(i64::MIN + 7)),
                        ("b".into(), Tag::ByteArray(vec![0xde, 0xad])),
                        ("i".into(), Tag::IntArray(vec![0x0102_0304])),
                        ("a".into(), Tag::LongArray(vec![i64::MIN + 7])),
                    ]
                    .into(),
                ),
            }],
        };
        let payload = [header.as_slice(), &value].concat();
        let stage = |data| {
            decode_staged(crate::packets::configuration::CustomPayload {
                channel: CHANNEL.into(),
                data,
            })
        };
        assert_eq!(decode(&payload).unwrap(), expected);
        let (decoded, original) = stage(payload.clone()).unwrap().into_message();
        assert_eq!(decoded, expected);
        assert_eq!(original.data, payload);

        for bad in [
            &value[..value.len() - 1],               // missing compound terminator
            &[10, 8, 0, 1, b's', 0, 1, 0xff, 0],     // bad modified UTF-8
            &[10, 9, 0, 1, b'l', 13, 0, 0, 0, 1, 0], // bad list type
            &[10, 11, 0, 1, b'a', 0xff, 0xff, 0xff, 0xff, 0], // negative array count
        ] {
            let payload = [header.as_slice(), bad].concat();
            let Err(staged) = stage(payload.clone()) else {
                panic!("staged decoder accepted malformed NBT: {bad:?}");
            };
            assert_eq!(staged.0, decode(&payload).unwrap_err().0);
        }
    }

    #[test]
    fn decode_refuses_trailing_bytes_and_an_unknown_holder_kind() {
        let message = Message {
            holder: Holder::Level,
            replace: false,
            changes: changes(),
        };
        let mut payload = encode(&message).unwrap();
        payload.push(0);
        assert!(decode(&payload).is_err(), "trailing bytes");
        assert!(decode(&[5, 0, 0]).is_err(), "unknown holder kind");
    }

    #[test]
    fn encode_refuses_a_change_the_client_would_not_read_as_sent() {
        let mut message = Message {
            holder: Holder::Level,
            replace: false,
            changes: changes(),
        };
        message.changes[0].name = "n".repeat(MAX_NAME).as_str().into();
        assert!(decode(&encode(&message).unwrap()).is_ok());
        message.changes[0].name = "n".repeat(MAX_NAME + 1).as_str().into();
        assert!(encode(&message).is_err(), "name");
        message.changes[0].name = "pool".into();
        let mut deep = Tag::Int(0);
        for _ in 0..600 {
            deep = Tag::Compound([("v".into(), deep)].into());
        }
        let Tag::Compound(deep) = deep else {
            unreachable!()
        };
        message.changes[0].value = Some(deep);
        assert!(encode(&message).is_err(), "unwritable value");
    }

    #[test]
    fn encoding_refusals_are_local_to_the_change_and_keep_replace_markers() {
        let bad = AttachmentSync {
            namespace: "fixture".into(),
            name: "bad".into(),
            value: Some([("s".into(), Tag::String("x".repeat(65_536).as_str().into()))].into()),
        };
        for replace in [true, false] {
            let healthy = changes();
            let mut message = Message {
                holder: Holder::Entity(17),
                replace,
                changes: vec![healthy[0].clone(), bad.clone(), healthy[1].clone()],
            };
            let mut refused = Vec::new();
            let body = encode_retaining(&message, |change, _| refused.push(change.name.clone()))
                .unwrap()
                .unwrap();
            assert_eq!(refused.as_slice(), std::slice::from_ref(&bad.name));
            message.changes = healthy;
            assert_eq!(decode(&body).unwrap(), message);

            message.changes = vec![bad.clone()];
            let body = encode_retaining(&message, |_, _| {}).unwrap();
            if replace {
                message.changes.clear();
                assert_eq!(decode(&body.unwrap()).unwrap(), message);
            } else {
                assert!(body.is_none());
            }
        }
    }

    #[test]
    fn the_format_is_unchanged_and_has_no_body_cap_of_its_own() {
        // Level holder (kind 4), not replaced, one removal of a:b.
        let removal = Message {
            holder: Holder::Level,
            replace: false,
            changes: vec![AttachmentSync {
                namespace: "a".into(),
                name: "b".into(),
                value: None,
            }],
        };
        assert_eq!(encode(&removal).unwrap(), [4, 0, 1, 1, b'a', 1, b'b', 0]);
        // Network NBT: an unnamed compound holding Int "v" = 7, then its end.
        let mut set = removal.clone();
        set.changes[0].value = Some([("v".into(), Tag::Int(7))].into());
        assert_eq!(
            encode(&set).unwrap(),
            [4, 0, 1, 1, b'a', 1, b'b', 10, 3, 0, 1, b'v', 0, 0, 0, 7, 0]
        );
        // Past the old 1,048,576-byte cap and the 2 MiB network NBT quota: what encodes,
        // decodes; the connection's frame limits bound the message.
        let big = Tag::ByteArray(vec![1; 3 << 20]);
        set.changes[0].value = Some([("big".into(), big)].into());
        let payload = encode(&set).unwrap();
        assert!(payload.len() > 3 << 20);
        assert_eq!(decode(&payload).unwrap(), set);
    }

    #[test]
    fn a_value_past_the_node_limit_is_refused_by_both_ends_alike() {
        // More entries than the 1,000,000 nodes of `value_limits`.
        let list = Tag::Compound([("l".into(), Tag::List(vec![Tag::Int(0); 1_000_001]))].into());
        let Tag::Compound(value) = list.clone() else {
            unreachable!()
        };
        let past = Message {
            holder: Holder::Level,
            replace: false,
            changes: vec![AttachmentSync {
                namespace: "a".into(),
                name: "b".into(),
                value: Some(value),
            }],
        };
        assert!(encode(&past).is_err(), "the writer refuses it");
        // A peer that wrote it anyway is refused by the reader, not truncated: Level
        // holder, not replaced, one change of a:b, then the value.
        let mut forged = vec![4, 0, 1, 1, b'a', 1, b'b'];
        let unbounded = nbt::Limits {
            max_nodes: usize::MAX,
            ..nbt::Limits::default()
        };
        forged.extend(nbt::write_network(&list, unbounded).unwrap());
        assert!(decode(&forged).is_err(), "the reader refuses it");
    }
}
