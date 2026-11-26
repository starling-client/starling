//! Collider sync: the boxes and pose of a moving collider (an entity's `set_collider`), sent by a
//! Sunrise server to the Sunrise clients that track the entity
//! (docs/modding/src/how-to/entities/collision-exclusions.md, "Moving colliders").
//!
//! A client that starts tracking the entity gets the collider whole, split into messages of
//! at most [`MAX_MESSAGE`] bytes; after that it gets only what changed: boxes replaced,
//! appended or cut off by index, or a new pose. A vanilla client never offers the channel
//! and is sent nothing.
//!
//! The channel is an engine declaration in the negotiated payload table: only a peer that
//! offers it at [`VERSION`] is sent anything.
use crate::{Error, Result, buf::Reader, packets::custom_payload::CLIENTBOUND_LIMIT};
use sunrise_core::geometry::oriented;
use sunrise_core::vanilla_bytes::Writer;

/// The custom payload channel, clientbound only.
pub const CHANNEL: &str = "sunrise:collider_sync";
/// The channel's exact declaration version in the negotiated table.
pub const VERSION: &str = "2";
/// Vanilla `ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` (1,048,576 bytes): the one size
/// limit on a collider's encoding.
pub const MAX_PAYLOAD_BYTES: usize = CLIENTBOUND_LIMIT;
/// The largest message the server writes, in bytes.
pub const MAX_MESSAGE: usize = 64 * 1024;
/// A box on the wire: six `f32`.
pub const BOX_BYTES: usize = 24;
/// Maximum boxes that fit in one [`MAX_PAYLOAD_BYTES`] `Boxes` message (53-byte header + 24 B/box).
pub const MAX_BOXES_IN_PAYLOAD: usize = (MAX_PAYLOAD_BYTES - 53) / BOX_BYTES;

fn varint_len(value: usize) -> usize {
    let mut v = value as u64;
    let mut len = 1;
    while v >= 0x80 {
        v >>= 7;
        len += 1;
    }
    len
}

/// Worst-case encoded byte size of a single `Message::Boxes` carrying `box_count` boxes.
/// Uses the maximum 5-byte entity varint so the check does not depend on entity id.
pub fn encoded_bytes(box_count: usize) -> usize {
    // kind(1) + entity(5) + rotation(16) + pivot(24) + total(varint) + first=0(1) + count(varint)
    let count_varint = varint_len(box_count);
    let header = 1 + 5 + 16 + 24 + count_varint + 1 + count_varint;
    header.saturating_add(box_count.saturating_mul(BOX_BYTES))
}

/// One box: the corners in the collider's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WireBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// What the server tells a client about one entity's collider.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A run of the whole collider: the boxes from index `first` of `total`, and the pose.
    /// A run with `first` 0 replaces what the client holds; the others extend it in order.
    Boxes {
        entity: u32,
        rotation: [f32; 4],
        pivot: [f64; 3],
        total: u32,
        first: u32,
        boxes: Vec<WireBox>,
    },
    /// A change to the collider the client holds: boxes cut off past `truncate` first (None:
    /// none), then `replaced` by index, then `appended`.
    Patch {
        entity: u32,
        truncate: Option<u32>,
        replaced: Vec<(u32, WireBox)>,
        appended: Vec<WireBox>,
    },
    /// The pose (rotation and pivot) alone.
    Pose {
        entity: u32,
        rotation: [f32; 4],
        pivot: [f64; 3],
    },
    /// The entity has no collider any more.
    Clear { entity: u32 },
}

fn write_box(w: &mut Writer, b: &WireBox) {
    b.min.iter().chain(&b.max).for_each(|&v| w.f32(v));
}

fn read_box(r: &mut Reader) -> Result<WireBox> {
    let mut v = [0.0f32; 6];
    for slot in &mut v {
        *slot = r.f32()?;
    }
    let wide = v.map(f64::from);
    if !oriented::check_box([wide[0], wide[1], wide[2]], [wide[3], wide[4], wide[5]]) {
        return Err(Error::new("collider sync: a box value is not finite"));
    }
    Ok(WireBox {
        min: [v[0], v[1], v[2]],
        max: [v[3], v[4], v[5]],
    })
}

fn write_rotation(w: &mut Writer, q: [f32; 4]) {
    q.iter().for_each(|&v| w.f32(v));
}

fn read_rotation(r: &mut Reader) -> Result<[f32; 4]> {
    let q = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
    oriented::normalize_rotation(q)
        .ok_or_else(|| Error::new("collider sync: the rotation is zero or not finite"))
}

fn write_pivot(w: &mut Writer, pivot: [f64; 3]) {
    pivot.iter().for_each(|&v| w.f64(v));
}

fn read_pivot(r: &mut Reader) -> Result<[f64; 3]> {
    let pivot = [r.f64()?, r.f64()?, r.f64()?];
    if !oriented::check_pivot(pivot) {
        return Err(Error::new("collider sync: a pivot value is not finite"));
    }
    Ok(pivot)
}

/// A message's payload.
pub fn encode(message: &Message) -> Vec<u8> {
    let mut w = Writer::new();
    match message {
        Message::Boxes {
            entity,
            rotation,
            pivot,
            total,
            first,
            boxes,
        } => {
            w.varint(0);
            w.varint(*entity as i32);
            write_rotation(&mut w, *rotation);
            write_pivot(&mut w, *pivot);
            w.varint(*total as i32);
            w.varint(*first as i32);
            w.varint(boxes.len() as i32);
            boxes.iter().for_each(|b| write_box(&mut w, b));
        }
        Message::Patch {
            entity,
            truncate,
            replaced,
            appended,
        } => {
            w.varint(1);
            w.varint(*entity as i32);
            w.bool(truncate.is_some());
            w.varint(truncate.unwrap_or(0) as i32);
            w.varint(replaced.len() as i32);
            for (index, b) in replaced {
                w.varint(*index as i32);
                write_box(&mut w, b);
            }
            w.varint(appended.len() as i32);
            appended.iter().for_each(|b| write_box(&mut w, b));
        }
        Message::Pose {
            entity,
            rotation,
            pivot,
        } => {
            w.varint(2);
            w.varint(*entity as i32);
            write_rotation(&mut w, *rotation);
            write_pivot(&mut w, *pivot);
        }
        Message::Clear { entity } => {
            w.varint(3);
            w.varint(*entity as i32);
        }
    }
    w.data
}

/// A payload's message; an error for one that is malformed or exceeds the payload limit.
pub fn decode(data: &[u8]) -> Result<Message> {
    if data.len() > MAX_PAYLOAD_BYTES {
        return Err(Error::new("collider sync: payload past the limit"));
    }
    let max_boxes = MAX_BOXES_IN_PAYLOAD as u32;
    let mut r = Reader::new(data);
    let kind = r.varint()?;
    let entity = r.varint()? as u32;
    let message = match kind {
        0 => {
            let rotation = read_rotation(&mut r)?;
            let pivot = read_pivot(&mut r)?;
            let total = r.varint()? as u32;
            let first = r.varint()? as u32;
            if total > max_boxes || first > total {
                return Err(Error::new("collider sync: box counts past the limit"));
            }
            let count = r.count(r.remaining() / BOX_BYTES)?;
            if u64::from(first) + count as u64 > u64::from(total) {
                return Err(Error::new("collider sync: a run past the total"));
            }
            let boxes = (0..count)
                .map(|_| read_box(&mut r))
                .collect::<Result<Vec<_>>>()?;
            Message::Boxes {
                entity,
                rotation,
                pivot,
                total,
                first,
                boxes,
            }
        }
        1 => {
            let truncate = r.bool()?;
            let at = r.varint()? as u32;
            if at > max_boxes {
                return Err(Error::new("collider sync: a cut past the limit"));
            }
            let replaced_count = r.count(r.remaining() / (BOX_BYTES + 1))?;
            if replaced_count > MAX_BOXES_IN_PAYLOAD {
                return Err(Error::new(
                    "collider sync: more replaced boxes than the limit",
                ));
            }
            let replaced = (0..replaced_count)
                .map(|_| {
                    let index = r.varint()? as u32;
                    if index >= max_boxes {
                        return Err(Error::new("collider sync: a replaced box past the limit"));
                    }
                    Ok((index, read_box(&mut r)?))
                })
                .collect::<Result<Vec<_>>>()?;
            let appended_count = r.count(r.remaining() / BOX_BYTES)?;
            if appended_count > MAX_BOXES_IN_PAYLOAD {
                return Err(Error::new(
                    "collider sync: more appended boxes than the limit",
                ));
            }
            let appended = (0..appended_count)
                .map(|_| read_box(&mut r))
                .collect::<Result<Vec<_>>>()?;
            Message::Patch {
                entity,
                truncate: truncate.then_some(at),
                replaced,
                appended,
            }
        }
        2 => Message::Pose {
            entity,
            rotation: read_rotation(&mut r)?,
            pivot: read_pivot(&mut r)?,
        },
        3 => Message::Clear { entity },
        kind => return Err(Error::new(format!("collider sync: kind {kind}"))),
    };
    if r.remaining() > 0 {
        return Err(Error::new("collider sync: trailing bytes"));
    }
    Ok(message)
}

/// How many boxes of a run fit in one message with its header.
fn per_message() -> usize {
    (MAX_MESSAGE - 96) / BOX_BYTES
}

/// The messages that give a client a whole collider, none larger than [`MAX_MESSAGE`].
pub fn whole(entity: u32, rotation: [f32; 4], pivot: [f64; 3], boxes: &[WireBox]) -> Vec<Message> {
    let total = boxes.len() as u32;
    if boxes.is_empty() {
        return vec![Message::Boxes {
            entity,
            rotation,
            pivot,
            total: 0,
            first: 0,
            boxes: Vec::new(),
        }];
    }
    boxes
        .chunks(per_message())
        .enumerate()
        .map(|(i, run)| Message::Boxes {
            entity,
            rotation,
            pivot,
            total,
            first: (i * per_message()) as u32,
            boxes: run.to_vec(),
        })
        .collect()
}

/// The messages that turn the collider `old` into `new`: only the boxes that differ, by index.
pub fn changes(entity: u32, old: &[WireBox], new: &[WireBox]) -> Vec<Message> {
    let truncate = (new.len() < old.len()).then_some(new.len() as u32);
    let replaced = old
        .iter()
        .zip(new)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (_, b))| (i as u32, *b))
        .collect::<Vec<_>>();
    let appended = new.get(old.len()..).unwrap_or_default();
    // A replaced entry costs its index too: keep a message under the cap.
    let per = (MAX_MESSAGE - 96) / (BOX_BYTES + 5);
    let mut out = Vec::new();
    let mut runs = replaced.chunks(per);
    let mut first = true;
    loop {
        let run = runs.next().unwrap_or_default();
        if !first && run.is_empty() {
            break;
        }
        out.push(Message::Patch {
            entity,
            truncate: if first { truncate } else { None },
            replaced: run.to_vec(),
            appended: Vec::new(),
        });
        first = false;
        if runs.len() == 0 {
            break;
        }
    }
    for run in appended.chunks(per_message()) {
        out.push(Message::Patch {
            entity,
            truncate: None,
            replaced: Vec::new(),
            appended: run.to_vec(),
        });
    }
    out.retain(|m| {
        !matches!(m, Message::Patch { truncate: None, replaced, appended, .. }
            if replaced.is_empty() && appended.is_empty())
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(n: usize) -> Vec<WireBox> {
        (0..n)
            .map(|i| WireBox {
                min: [(i % 400) as f32, (i / 400 % 100) as f32, 0.0],
                max: [(i % 400) as f32 + 1.0, (i / 400 % 100) as f32 + 1.0, 1.0],
            })
            .collect()
    }

    #[test]
    fn every_message_round_trips() {
        let b = boxes(3);
        let q = oriented::normalize_rotation([0.0, 0.5, 0.0, 0.866]).unwrap();
        for m in [
            Message::Boxes {
                entity: 9,
                rotation: q,
                pivot: [1.5, -2.0, 3.25],
                total: 3,
                first: 0,
                boxes: b.clone(),
            },
            Message::Patch {
                entity: 9,
                truncate: Some(2),
                replaced: vec![(1, b[0])],
                appended: vec![b[2]],
            },
            Message::Patch {
                entity: 9,
                truncate: None,
                replaced: vec![],
                appended: vec![],
            },
            Message::Pose {
                entity: 9,
                rotation: [0.0, 0.0, 0.0, 1.0],
                pivot: [0.5, 0.0, -0.5],
            },
            Message::Clear { entity: 9 },
        ] {
            assert_eq!(decode(&encode(&m)).unwrap(), m);
        }
    }

    /// A collider at the payload limit splits into messages under `MAX_MESSAGE` and reassembles.
    #[test]
    fn a_collider_at_the_payload_limit_is_split_and_reassembles() {
        assert!(encoded_bytes(MAX_BOXES_IN_PAYLOAD) <= MAX_PAYLOAD_BYTES);
        assert!(encoded_bytes(MAX_BOXES_IN_PAYLOAD + 1) > MAX_PAYLOAD_BYTES);
        let all = boxes(MAX_BOXES_IN_PAYLOAD);
        let messages = whole(5, [0.0, 0.0, 0.0, 1.0], [1.0, 2.0, 3.0], &all);
        assert!(messages.len() > 1);
        let mut joined = Vec::new();
        for m in &messages {
            let bytes = encode(m);
            assert!(bytes.len() <= MAX_MESSAGE, "{} bytes", bytes.len());
            let Message::Boxes {
                first,
                boxes,
                pivot,
                ..
            } = decode(&bytes).unwrap()
            else {
                panic!("not a run");
            };
            assert_eq!(pivot, [1.0, 2.0, 3.0]);
            assert_eq!(first as usize, joined.len());
            joined.extend(boxes);
        }
        assert_eq!(joined, all);
    }

    #[test]
    fn a_change_carries_only_the_boxes_that_differ() {
        let old = boxes(100);
        let mut new = old.clone();
        new[40].max[1] = 3.0;
        new.truncate(90);
        new.push(boxes(1)[0]);
        let messages = changes(1, &old, &new);
        let sent: usize = messages
            .iter()
            .map(|m| match m {
                Message::Patch {
                    replaced, appended, ..
                } => replaced.len() + appended.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(sent, 2);
        let mut held = old.clone();
        for m in &messages {
            let Message::Patch {
                truncate,
                replaced,
                appended,
                ..
            } = m
            else {
                panic!("not a patch");
            };
            if let Some(n) = truncate {
                held.truncate(*n as usize);
            }
            for (i, b) in replaced {
                held[*i as usize] = *b;
            }
            held.extend(appended.iter().copied());
        }
        assert_eq!(held, new);
        assert!(changes(1, &new, &new).is_empty());
    }

    #[test]
    fn hostile_payloads_are_refused_without_allocating_what_they_claim() {
        let mut w = Writer::new();
        w.varint(0);
        w.varint(1);
        write_rotation(&mut w, [0.0, 0.0, 0.0, 1.0]);
        write_pivot(&mut w, [0.0; 3]);
        w.varint(MAX_BOXES_IN_PAYLOAD as i32);
        w.varint(0);
        w.varint(1_000_000);
        assert!(decode(&w.data).is_err());

        let mut m = encode(&Message::Boxes {
            entity: 1,
            rotation: [0.0, 0.0, 0.0, 1.0],
            pivot: [0.0; 3],
            total: 1,
            first: 1,
            boxes: boxes(1),
        });
        assert!(decode(&m).is_err());
        m = encode(&Message::Pose {
            entity: 1,
            rotation: [f32::NAN, 0.0, 0.0, 1.0],
            pivot: [0.0; 3],
        });
        assert!(decode(&m).is_err());
        m = encode(&Message::Pose {
            entity: 1,
            rotation: [0.0, 0.0, 0.0, 0.0],
            pivot: [0.0; 3],
        });
        assert!(decode(&m).is_err());
        m = encode(&Message::Pose {
            entity: 1,
            rotation: [0.0, 0.0, 0.0, 1.0],
            pivot: [f64::NAN, 0.0, 0.0],
        });
        assert!(decode(&m).is_err());
        m = encode(&Message::Clear { entity: 1 });
        m.push(0);
        assert!(decode(&m).is_err());
        assert!(decode(&[9, 1]).is_err());
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn malformed_patches_are_refused() {
        let max_boxes = MAX_BOXES_IN_PAYLOAD as u32;
        let patch = |replaced: Vec<(u32, WireBox)>, appended: Vec<WireBox>| {
            decode(&encode(&Message::Patch {
                entity: 1,
                truncate: None,
                replaced,
                appended,
            }))
        };
        let one = boxes(1)[0];
        assert!(patch(vec![(max_boxes - 1, one)], vec![]).is_ok());
        assert!(patch(vec![(max_boxes, one)], vec![]).is_err());
        assert!(patch(vec![(u32::MAX >> 1, one)], vec![]).is_err());
        let cut = encode(&Message::Patch {
            entity: 1,
            truncate: Some(max_boxes + 1),
            replaced: vec![],
            appended: vec![],
        });
        assert!(decode(&cut).is_err());
    }
}
