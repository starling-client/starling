//! The `is_valid_book_shelf` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the position `offset` from the enchanting table at `table_pos` counts as a
    /// bookshelf (`EnchantingTableBlock.isValidBookShelf`), asked for each position the table
    /// scans: vanilla's 32 and the offsets mods added with
    /// `add_enchanting_table_offsets`. `valid` starts at vanilla's answer: the block there
    /// gives a nonzero `enchant_power_bonus` and the block halfway to the table (each of x and
    /// z halved, rounding toward zero) is in `#minecraft:enchantment_power_transmitter`.
    /// Set it to `true` for a position the default refuses, a blocked midpoint for example,
    /// or to `false` for one it accepts.
    ///
    /// The offer sum (`EnchantmentMenu.slotsChanged`) adds the `enchant_power_bonus` of the
    /// block at each valid position, so `valid = true` on a block with no bonus adds 0. The
    /// client's enchant particles (`EnchantingTableBlock.animateTick`) ask it too, only after
    /// the position's `nextInt(16)` draw passes, and spawn a particle for each valid position.
    /// Both sides ask it, so a mod that changes it must be on both: a client without it shows
    /// vanilla's particles and the server's offers stay authoritative. `level` is read-only.
    /// Only called when a mod listens; with none, the scan costs what vanilla's does.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "valid",
        host = "read",
        neoforge = "none (EnchantingTableBlock.isValidBookShelf; NeoForge's extension is the per-block IBlockExtension.getEnchantPowerBonus and it has no hook here)"
    )]
    fn is_valid_book_shelf(
        level: LevelRef,
        table_pos: BlockPos,
        offset: BlockPos,
        valid: &mut bool,
    );
}
