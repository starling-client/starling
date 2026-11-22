//! The age a Sunrise client carried its player on (docs/modding/src/how-to/native-mods.md,
//! "Moving colliders"): sent just before a position report while a moving collider carries the
//! player. A client moves its player with the collider as it last heard of it, so its position
//! fits the collider's transform from some ticks back; this says how many ticks stale the
//! client's copy was. The server uses it only as where its search of the recent transforms
//! starts, clamped into its own window, and still checks the position against them, so the
//! hint cannot move a player anywhere the server would not let it stand. A vanilla client
//! never sends it.
use crate::{Error, Result, buf::Reader};
use sunrise_core::vanilla_bytes::Writer;

/// The custom payload channel, serverbound only.
pub const CHANNEL: &str = "sunrise:carry_age";

/// What the client says: the collider carrying it and how many ticks old its copy was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hint {
    pub collider: u32,
    pub ticks: u8,
}

pub fn encode(hint: Hint) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(hint.collider as i32);
    w.u8(hint.ticks);
    w.data
}

/// The hint in a payload; an error for a cut one, a negative collider or trailing bytes.
pub fn decode(data: &[u8]) -> Result<Hint> {
    let mut r = Reader::new(data);
    let collider = u32::try_from(r.varint()?)
        .map_err(|_| Error::new("carry age: negative collider entity"))?;
    let ticks = r.u8()?;
    if r.remaining() > 0 {
        return Err(Error::new("carry age: trailing bytes"));
    }
    Ok(Hint { collider, ticks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_round_trips_and_a_bad_payload_is_an_error() {
        let hint = Hint {
            collider: 70_000,
            ticks: 9,
        };
        assert_eq!(decode(&encode(hint)).unwrap(), hint);
        assert!(decode(&[]).is_err());
        assert!(decode(&[1]).is_err());
        let mut long = encode(hint);
        long.push(0);
        assert!(decode(&long).is_err());
        assert!(decode(&[0xff, 0xff, 0xff, 0xff, 0x0f, 1]).is_err());
    }
}
