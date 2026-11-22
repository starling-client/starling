//! Menu extra data: what a Sunrise server tells a Sunrise client about a
//! native mod's menu type its block opened, just before the
//! `ClientboundOpenScreenPacket` of the same window (docs/native-mods.md,
//! "Menu extra data"). NeoForge's counterpart is `openMenu` with an extra-data
//! writer, read by the menu type's `IContainerFactory`.
//!
//! A payload is the window id, the block's position if there is one (a
//! boolean, then the three axes), then the bytes the mod wrote (empty when it
//! wrote none). Clientbound only.
//!
//! **Revision** (configuration, both directions): before play, the server
//! sends a client whose `minecraft:brand` is Sunrise's [`query`]
//! (`[QUERY, REVISION]`). A client answers with [`answer`] (`[ANSWER, revision]`)
//! for a supported revision (`1..=REVISION`); unsupported revisions (including 0)
//! are rejected. Revision 2 adds blockless menu data updates (`DATA_CHANNEL`)
//! without changing the opening payload. A vanilla client is never asked.
use crate::{Error, Result, buf::Reader};
use sunrise_core::vanilla_bytes::Writer;

/// The custom payload channel.
pub const CHANNEL: &str = "sunrise:menu_extra";
/// Play payload for the current blockless window's live, full-width data slots.
pub const DATA_CHANNEL: &str = "sunrise:menu_data";
/// First revision that supports the live data payload and slotless menus.
pub const LIVE_DATA_REVISION: u8 = 2;
/// Latest supported menu revision. Revision 1 introduced optional opening positions.
pub const REVISION: u8 = LIVE_DATA_REVISION;
/// Bytes a mod may write at most: fits within vanilla's 1 MiB clientbound custom
/// payload limit (`CLIENTBOUND_LIMIT`, 1,048,576 bytes) after the worst-case
/// envelope header (5-byte VarInt window + 1-byte optional-position flag +
/// 12-byte position + 3-byte VarInt byte-array length = 21 bytes).
pub const MAX_DATA: usize = crate::packets::custom_payload::CLIENTBOUND_LIMIT - 21;

const QUERY: u8 = 0;
const ANSWER: u8 = 1;

/// The server's query, configuration clientbound: the custom payload's data.
pub fn query() -> Vec<u8> {
    vec![QUERY, REVISION]
}

/// Reads the server's query and picks the highest mutually supported revision.
pub fn read_query(data: &[u8]) -> Result<u8> {
    let mut r = Reader::new(data);
    if r.u8()? != QUERY {
        return Err(Error::new("menu extra revision: not a query"));
    }
    let offered = r.u8()?;
    if offered == 0 {
        return Err(Error::new("menu extra revision: unsupported revision"));
    }
    if offered <= REVISION && r.remaining() > 0 {
        return Err(Error::new("menu extra revision: trailing bytes"));
    }
    Ok(offered.min(REVISION))
}

/// The client's answer, configuration serverbound: it reads this revision's layout.
pub fn answer(revision: u8) -> Vec<u8> {
    vec![ANSWER, revision]
}

/// Reads a client's answer: the supported revision whose capabilities it accepts.
pub fn read_answer(data: &[u8]) -> Result<u8> {
    let mut r = Reader::new(data);
    if r.u8()? != ANSWER {
        return Err(Error::new("menu extra revision: not an answer"));
    }
    let revision = r.u8()?;
    if revision == 0 || revision > REVISION {
        return Err(Error::new("menu extra revision: unsupported revision"));
    }
    if r.remaining() > 0 {
        return Err(Error::new("menu extra revision: trailing bytes"));
    }
    Ok(revision)
}

/// A value for an already open blockless mod menu. Play packets are ordered:
/// updates follow the open and precede any close or reuse of the window id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataSlot {
    pub window: i32,
    pub index: u16,
    pub value: i32,
}

impl DataSlot {
    pub fn encode(self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(self.window);
        w.u16(self.index);
        w.i32(self.value);
        w.data
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let slot = Self {
            window: r.varint()?,
            index: r.u16()?,
            value: r.i32()?,
        };
        if r.remaining() > 0 {
            return Err(Error::new("menu data: trailing bytes"));
        }
        Ok(slot)
    }
}

/// A menu's extra data as it travels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The window id the `OpenScreen` that follows carries.
    pub window: i32,
    /// The block that opened the menu; None for a menu opened without one.
    pub pos: Option<[i32; 3]>,
    pub data: Vec<u8>,
}

/// A message's payload; an error for data past [`MAX_DATA`].
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    if message.data.len() > MAX_DATA {
        return Err(Error::new("menu extra: data too big"));
    }
    let mut w = Writer::new();
    w.varint(message.window);
    w.bool(message.pos.is_some());
    message.pos.iter().flatten().for_each(|&axis| w.i32(axis));
    w.byte_array(&message.data);
    Ok(w.data)
}

/// A payload's message; an error for one that breaks a cap.
pub fn decode(data: &[u8]) -> Result<Message> {
    let mut r = Reader::new(data);
    let window = r.varint()?;
    let pos = if r.bool()? {
        Some([r.i32()?, r.i32()?, r.i32()?])
    } else {
        None
    };
    let data = r.byte_array(MAX_DATA)?;
    if r.remaining() > 0 {
        return Err(Error::new("menu extra: trailing bytes"));
    }
    Ok(Message { window, pos, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_data_keeps_all_bits_and_rejects_trailing_bytes() {
        let bytes = [7, 0, 1, 0x80, 0, 0, 0];
        let slot = DataSlot {
            window: 7,
            index: 1,
            value: i32::MIN,
        };
        assert_eq!(slot.encode(), bytes);
        assert_eq!(DataSlot::decode(&bytes).unwrap(), slot);
        assert!(DataSlot::decode(&bytes[..6]).is_err());
        assert!(DataSlot::decode(&[&bytes[..], &[0]].concat()).is_err());
    }

    #[test]
    fn a_message_round_trips() {
        for (window, pos, data) in [
            (100, Some([-30_000_000, -64, 29_999_999]), vec![]),
            (100, Some([0, 0, 0]), vec![0, 255, 7]),
            (-1, Some([-30_000_000, -64, 29_999_999]), vec![9; MAX_DATA]),
            (100, None, vec![1, 2, 3]),
        ] {
            let m = Message { window, pos, data };
            let encoded = encode(&m).unwrap();
            assert!(encoded.len() <= crate::packets::custom_payload::CLIENTBOUND_LIMIT);
            assert_eq!(decode(&encoded).unwrap(), m);
        }
    }

    #[test]
    fn caps_are_enforced_both_ways() {
        let big = Message {
            window: 1,
            pos: Some([0; 3]),
            data: vec![0; MAX_DATA + 1],
        };
        assert!(encode(&big).is_err());
        // Built by hand, as a server that skipped the check would.
        let mut w = Writer::new();
        w.varint(1);
        w.bool(false);
        w.byte_array(&vec![0; MAX_DATA + 1]);
        assert!(decode(&w.data).is_err());
        let mut ok = encode(&Message {
            window: 1,
            pos: Some([0; 3]),
            data: vec![1],
        })
        .unwrap();
        ok.push(0);
        assert!(decode(&ok).is_err(), "trailing bytes");
        assert!(decode(&[]).is_err());
        assert!(decode(&[1, 1, 0]).is_err(), "a cut position");
    }

    #[test]
    fn the_revision_exchange_round_trips() {
        assert_eq!(read_query(&query()).unwrap(), REVISION);
        assert_eq!(read_answer(&answer(REVISION)).unwrap(), REVISION);
        assert_eq!(read_query(&[QUERY, 1]).unwrap(), 1);
        assert_eq!(read_query(&[QUERY, 2]).unwrap(), 2);
        assert_eq!(read_answer(&answer(1)).unwrap(), 1);
        // A newer server's query is clamped to the highest revision this client supports.
        let other = [QUERY, REVISION + 1, 9];
        assert_eq!(read_query(&other).unwrap(), REVISION);
        // Neither side can opt into the removed revision-0 layout.
        assert!(read_query(&[QUERY, 0]).is_err());
        assert!(read_answer(&[ANSWER, 0]).is_err());
        assert!(read_answer(&[ANSWER, REVISION + 1]).is_err());
        assert!(read_answer(&[ANSWER, REVISION + 1, 9]).is_err());
        assert!(read_query(&[ANSWER, REVISION]).is_err());
        assert!(read_answer(&[QUERY, REVISION]).is_err());
        assert!(read_query(&[QUERY, REVISION, 0]).is_err(), "trailing bytes");
        assert!(
            read_answer(&[ANSWER, REVISION, 0]).is_err(),
            "trailing bytes"
        );
        assert!(read_query(&[]).is_err());
        assert!(read_answer(&[]).is_err());
    }
}
