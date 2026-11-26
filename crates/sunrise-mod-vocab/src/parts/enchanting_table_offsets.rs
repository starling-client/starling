//! Extra bookshelf scan offsets of the enchanting table.
use super::*;

/// Enchanting table scan offsets declared during registration.
pub trait EnchantingTableOffsetsRegistrar {
    /// Add `offsets` to `EnchantingTableBlock.BOOKSHELF_OFFSETS`, the positions around a table
    /// (relative to it) that the offer sum and the client's enchant particles scan. Vanilla's
    /// 32 come first, then every mod's in load order; an offset already in the list is dropped.
    /// The list is fixed at startup: changing a mod's offsets needs a restart, not `/reload`.
    /// The block halfway between the table and a scanned position (each of x and z halved,
    /// rounding toward zero) must still transmit, so a ring farther than two blocks out is
    /// checked against the block at half its distance. Each added offset adds one
    /// `nextInt(16)` draw to the client's particle tick of every table in range, never to the
    /// server's offer.
    fn add_enchanting_table_offsets(&mut self, offsets: &[[i32; 3]]);
}

/// The offsets declared during registration, in declaration order and before de-duplication.
// hidden: engine scan list; mods declare offsets through Registrar.
#[doc(hidden)]
pub fn enchanting_table_offset_declarations() -> &'static [[i32; 3]] {
    installed().map_or(&[], |parts| &parts.enchanting_table_offsets)
}
