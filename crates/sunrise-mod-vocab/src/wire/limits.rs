//! The optional shared allocation budget of a typed payload, not a second codec.
use super::WireError;
use std::alloc::Layout;

/// The deepest a typed payload may nest in Wire.
pub const PAYLOAD_MAX_DEPTH: u16 = 64;
/// The largest UTF-8 string in a typed payload, in bytes.
pub const PAYLOAD_MAX_STRING_BYTES: usize = 4096;
/// The largest variable-length collection in a typed payload, in entries.
pub const PAYLOAD_MAX_LIST_ENTRIES: usize = 4096;
/// The allocation budget for a typed payload value, and its maximum encoded body size.
/// Owned collection storage is charged at 64-bit sizes, even on a 32-bit target.
pub const PAYLOAD_DECODE_BUDGET: usize = 8_388_608;

pub(super) struct Budget(u64);

impl Budget {
    pub(super) fn payload() -> Self {
        Self(PAYLOAD_DECODE_BUDGET as u64)
    }

    pub(super) fn charge<T>(&mut self, count: usize) -> Result<(), WireError> {
        // On a 32-bit target this conservatively scales every field by the pointer ratio.
        let width = (std::mem::size_of::<T>() as u64) * (8 / std::mem::size_of::<usize>() as u64);
        self.bytes((count as u64).checked_mul(width).ok_or(WireError::Budget)?)
    }

    pub(super) fn arc<T>(&mut self, count: usize) -> Result<(), WireError> {
        self.bytes(arc_bytes::<T>(count)?)
    }

    pub(super) fn arc_str_overhead(&mut self, len: usize) -> Result<(), WireError> {
        self.bytes(arc_bytes::<u8>(len)? - len as u64)
    }

    pub(super) fn btree<T>(&mut self, len: usize) -> Result<(), WireError> {
        if len == 0 {
            return Ok(());
        }
        // The root may have only one item, every other node at least five. Charge a
        // full node for each possible node: 11 items, 12 edges and its header.
        let nodes = 1 + (len - 1) / 5;
        self.charge::<(T, usize)>(nodes.saturating_mul(16))
    }

    pub(super) fn hash_table<T>(&mut self, len: usize) -> Result<(), WireError> {
        if len == 0 {
            return Ok(());
        }
        // Allow at most 7/8 occupancy of the power-of-two bucket array. Control
        // bytes include one byte per bucket, up to 15 bytes of alignment padding,
        // and the trailing 16-byte control group.
        let buckets = len
            .checked_mul(8)
            .and_then(|scaled| scaled.div_ceil(7).checked_next_power_of_two())
            .ok_or(WireError::Budget)?
            .max(4);
        self.charge::<T>(buckets)?;
        self.bytes(buckets as u64 + 31)
    }

    pub(super) fn compound(&mut self, len: usize) -> Result<(), WireError> {
        if len == 0 {
            return Ok(());
        }
        use sunrise_core::java_hash_map::JavaHashMap;
        use sunrise_core::nbt::{JavaString, Tag};
        self.charge::<JavaHashMap<JavaString, Tag>>(1)?;
        // Compound uses the Java map's entry slab, bin heads and collision-tree
        // links. Allow for its initial slab and for the first tripling, then doubling.
        // The extra slot covers the slab sentinel and per-slot padding/metadata.
        self.charge::<(JavaString, Tag, [usize; 8])>(
            len.saturating_mul(3).max(12).saturating_add(1),
        )
    }

    pub(super) fn bytes(&mut self, bytes: u64) -> Result<(), WireError> {
        self.0 = self.0.checked_sub(bytes).ok_or(WireError::Budget)?;
        Ok(())
    }
}

fn arc_bytes<T>(count: usize) -> Result<u64, WireError> {
    // An Arc allocation holds both counts and the aligned value, even for a ZST.
    let values = Layout::array::<T>(count).map_err(|_| WireError::Budget)?;
    let (layout, _) = Layout::new::<[usize; 2]>()
        .extend(values)
        .map_err(|_| WireError::Budget)?;
    (layout.pad_to_align().size() as u64)
        .checked_mul(8 / std::mem::size_of::<usize>() as u64)
        .ok_or(WireError::Budget)
}
