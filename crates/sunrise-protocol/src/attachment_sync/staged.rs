//! Queue the checked output without expanding its retained allocation.
use super::{Message, decode_changes, value_limits};
use crate::{
    Error, Result, buf::java_utf8, packets::configuration::CustomPayload, retained::OwnedHeap,
};
use sunrise_core::nbt::{JavaString, Tag, unpack_network};
use sunrise_mod_runtime::attachments::{AttachmentSync, Holder};

/// A structurally decoded attachment message awaiting the holder's turn. Its
/// payload buffer holds the packed values, not a second copy of their wire bytes.
pub struct StagedMessage {
    holder: Holder,
    replace: bool,
    start: usize,
    changes: usize,
    payload: CustomPayload,
}

pub(super) fn stage(mut payload: CustomPayload) -> Result<StagedMessage> {
    let mut copy = payload.data.clone().into_boxed_slice();
    let (holder, replace, start, changes) = decode_changes(&payload.data, |reader, names| {
        for name in names {
            let prefix = &mut copy[name.prefix];
            if prefix.len() > 1 {
                // A typed span's length (at most MAX_NAME * 3) fits in ten bits.
                // Store the original prefix width in the same two-byte header.
                // In a five-byte prefix keep the high unused bits of its last
                // group as well, so even noncanonical lengths restore exactly.
                prefix[0] = 0x80 | ((prefix.len() as u8 - 2) << 2) | (name.bytes >> 8) as u8;
                prefix[1] = name.bytes as u8;
            }
        }
        if !matches!(reader.stage_nbt(value_limits(), &mut copy)?, 0 | 10) {
            return Err(Error::new("attachment sync: value is not a compound"));
        }
        Ok(())
    })?;
    payload.data = copy.into_vec();
    Ok(StagedMessage {
        holder,
        replace,
        start,
        changes: changes.len(),
        payload,
    })
}

impl StagedMessage {
    /// Bytes of the one owned packet allocation, plus its channel name.
    pub fn owned_heap_bytes(&self) -> usize {
        self.payload.owned_heap_bytes()
    }

    /// Materialize the checked values and restore the original bytes for observers.
    pub fn into_message(mut self) -> (Message, CustomPayload) {
        let mut tape = &mut self.payload.data[self.start..];
        let changes = (0..self.changes)
            .map(|_| {
                let namespace = unpack_name(&mut tape);
                let name = unpack_name(&mut tape);
                let value = match unpack_network(&mut tape) {
                    Tag::End => None,
                    Tag::Compound(compound) => Some(compound),
                    _ => unreachable!("attachment roots are compounds or removals"),
                };
                AttachmentSync {
                    namespace,
                    name,
                    value,
                }
            })
            .collect();
        (
            Message {
                holder: self.holder,
                replace: self.replace,
                changes,
            },
            self.payload,
        )
    }
}

fn unpack_name(tape: &mut &mut [u8]) -> JavaString {
    let first = tape[0];
    let (width, len) = if first < 128 {
        (1, usize::from(first))
    } else {
        (
            usize::from((first & 0x7f) >> 2) + 2,
            usize::from(u16::from_be_bytes([first & 3, tape[1]])),
        )
    };
    let (prefix, tail) = std::mem::take(tape).split_at_mut(width);
    let (bytes, tail) = tail.split_at_mut(len);
    *tape = tail;
    let unused = if width == 5 { prefix[4] & 0x70 } else { 0 };
    for (i, group) in prefix.iter_mut().enumerate() {
        *group = ((len as u32 >> (7 * i)) & 0x7f) as u8;
        if i + 1 < width {
            *group |= 0x80;
        } else {
            *group |= unused;
        }
    }
    // Transcode the already bounded name span to the holder's JavaString.
    // The protocol's Java UTF-8 decoder is total, including replacement characters.
    java_utf8(bytes).as_str().into()
}
