//! General entity spawn: how a Sunrise server tells a Sunrise client that it now tracks one row
//! of a mod's general entity type.
//!
//! A general type's key is not in the synced `minecraft:entity_type` registry, so the vanilla
//! `AddEntity` cannot name it. Pairing sends this message in its place; after it, the vanilla
//! packets keyed by the network id (moves, rotations, position syncs, removal) apply as they
//! do to any tracked entity, and the row's declared fields arrive as its synced attachments
//! (`sunrise:attachment_sync`).
//!
//! The channel is an engine declaration in the negotiated payload table: only a peer that
//! offers it at [`VERSION`] is sent anything. A vanilla peer never offers it, and is never
//! paired with a general row.
use crate::{Error, Result, buf::Reader};
use sunrise_core::identifier::Identifier;
use sunrise_core::vanilla_bytes::Writer;

/// The custom payload channel, clientbound only.
pub const CHANNEL: &str = "sunrise:general_entity";
/// The channel's exact declaration version in the negotiated table.
pub const VERSION: &str = "1";
/// The longest type key, in Java UTF-16 code units: vanilla's `Identifier.STREAM_CODEC` /
/// `ByteBufCodecs.STRING_UTF8` (`FriendlyByteBuf.DEFAULT_MAX_STRING_LENGTH`).
pub const MAX_KEY: usize = 32_767;
/// The largest clientbound custom payload message, in bytes: vanilla's
/// `ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` (1 MiB).
pub const MAX_MESSAGE: usize = crate::packets::custom_payload::CLIENTBOUND_LIMIT;

/// What the server tells a client about a general row.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// The client starts tracking a row: its network id, UUID, type key and where it is.
    Spawn {
        id: i32,
        uuid: u128,
        type_key: String,
        position: [f64; 3],
        /// Yaw, then pitch, in degrees.
        rotation: [f32; 2],
        /// Blocks per tick.
        velocity: [f64; 3],
    },
}

/// A message's payload; an error for a key that is not a namespaced identifier of at most
/// [`MAX_KEY`] UTF-16 code units, or a value that is not finite.
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    match message {
        Message::Spawn {
            id,
            uuid,
            type_key,
            position,
            rotation,
            velocity,
        } => {
            check_key(type_key)?;
            check_finite(position, rotation, velocity)?;
            w.varint(0);
            w.varint(*id);
            w.uuid(*uuid);
            w.string(type_key);
            position.iter().for_each(|&v| w.f64(v));
            rotation.iter().for_each(|&v| w.f32(v));
            velocity.iter().for_each(|&v| w.f64(v));
        }
    }
    if w.data.len() > MAX_MESSAGE {
        return Err(Error::new("general entity: a message past the cap"));
    }
    Ok(w.data)
}

/// A payload's message; an error for one that is malformed or breaks a bound.
pub fn decode(data: &[u8]) -> Result<Message> {
    if data.len() > MAX_MESSAGE {
        return Err(Error::new("general entity: a message past the cap"));
    }
    let mut r = Reader::new(data);
    let message = match r.varint()? {
        0 => {
            let id = r.varint()?;
            let uuid = r.uuid()?;
            let type_key = r.string(MAX_KEY)?;
            check_key(&type_key)?;
            let position = [r.f64()?, r.f64()?, r.f64()?];
            let rotation = [r.f32()?, r.f32()?];
            let velocity = [r.f64()?, r.f64()?, r.f64()?];
            check_finite(&position, &rotation, &velocity)?;
            Message::Spawn {
                id,
                uuid,
                type_key,
                position,
                rotation,
                velocity,
            }
        }
        kind => return Err(Error::new(format!("general entity: kind {kind}"))),
    };
    if r.remaining() > 0 {
        return Err(Error::new("general entity: trailing bytes"));
    }
    Ok(message)
}

fn check_key(key: &str) -> Result<()> {
    if key.encode_utf16().count() > MAX_KEY
        || Identifier::parse_namespaced(key).is_none()
        || !key.contains(':')
    {
        return Err(Error::new(
            "general entity: the type key is not an identifier",
        ));
    }
    Ok(())
}

fn check_finite(position: &[f64; 3], rotation: &[f32; 2], velocity: &[f64; 3]) -> Result<()> {
    let finite = position.iter().chain(velocity).all(|v| v.is_finite())
        && rotation.iter().all(|v| v.is_finite());
    if !finite {
        return Err(Error::new("general entity: a value is not finite"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn() -> Message {
        Message::Spawn {
            id: 4321,
            uuid: 0x0123_4567_89ab_cdef_0011_2233_4455_6677,
            type_key: "test:missile".into(),
            position: [1.5, 70.0, -3.25],
            rotation: [90.0, -12.5],
            velocity: [0.25, 0.0, -0.5],
        }
    }

    #[test]
    fn a_spawn_round_trips() {
        let m = spawn();
        assert_eq!(decode(&encode(&m).unwrap()).unwrap(), m);
    }

    #[test]
    fn a_value_that_is_not_finite_is_refused_both_ways() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let Message::Spawn {
                id,
                uuid,
                type_key,
                rotation,
                velocity,
                ..
            } = spawn();
            let m = Message::Spawn {
                id,
                uuid,
                type_key,
                position: [0.0, bad, 0.0],
                rotation,
                velocity,
            };
            assert!(encode(&m).is_err());
        }
        // Written by hand, past the encoder: a NaN yaw.
        let mut bytes = encode(&spawn()).unwrap();
        let yaw = bytes.len() - 3 * 8 - 2 * 4;
        bytes[yaw..yaw + 4].copy_from_slice(&f32::NAN.to_be_bytes());
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn a_key_that_is_not_an_identifier_or_too_long_is_refused() {
        for key in ["missile", "Test:Missile", "test:", ":missile", "test:a b"] {
            let mut m = spawn();
            let Message::Spawn { type_key, .. } = &mut m;
            *type_key = key.into();
            assert!(encode(&m).is_err(), "{key}");
        }
        // Keys longer than the removed 256-byte cap still round-trip up to vanilla's string bound.
        let mut long_valid = spawn();
        let Message::Spawn { type_key, .. } = &mut long_valid;
        *type_key = format!("test:{}", "a".repeat(300));
        assert_eq!(decode(&encode(&long_valid).unwrap()).unwrap(), long_valid);

        let mut m = spawn();
        let Message::Spawn { type_key, .. } = &mut m;
        *type_key = format!("test:{}", "a".repeat(MAX_KEY));
        assert!(encode(&m).is_err());
        // Past the encoder: a long key on the wire.
        let mut w = Writer::new();
        w.varint(0);
        w.varint(1);
        w.uuid(1);
        w.string(&format!("test:{}", "a".repeat(MAX_KEY)));
        (0..3).for_each(|_| w.f64(0.0));
        (0..2).for_each(|_| w.f32(0.0));
        (0..3).for_each(|_| w.f64(0.0));
        assert!(decode(&w.data).is_err());
    }

    #[test]
    fn a_truncated_message_trailing_bytes_or_an_unknown_kind_are_refused() {
        let bytes = encode(&spawn()).unwrap();
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_err());
        let mut other = bytes;
        other[0] = 1;
        assert!(decode(&other).is_err());
        assert!(decode(&vec![0; MAX_MESSAGE + 1]).is_err());
    }
}
