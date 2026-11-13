//! The `block_entity_energy` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0149

pub trait Hooks {
    /// Block `block`'s block entity has an energy buffer
    /// (`Capabilities.Energy.BLOCK`): set `energy` when the block is yours,
    /// as `SimpleEnergyHandler` is built: its capacity, the most one insert
    /// and one extract moves, and the faces a fill may come through and a
    /// drain may leave by. Asked once for each mod block when the ticking
    /// table is built; a capacity under 1 leaves the buffer out, and the
    /// limits are clamped to 0..=capacity. The amount is kept in the entity's
    /// saved compound, as the int `energy`, and the energy helpers below read
    /// and write it.
    #[hook(
        side = "server",
        contract = "replace",
        out = "energy",
        host = "read",
        neoforge = "Capabilities.Energy.BLOCK"
    )]
    fn block_entity_energy();
}
