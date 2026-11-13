//! The `block_entity_fluids` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0061

pub trait Hooks {
    /// Block `block`'s block entity has fluid tanks: set `fluids` when the
    /// block is yours, in the container's shape — each tank's capacity in
    /// millibuckets, the tanks each face reaches, and the tanks a fill may
    /// reach and a drain may take. Asked once for each mod block when the
    /// ticking table is built; at most 16 tanks, capacities clamped to
    /// 1..=1 000 000, and tank numbers past the count are left out. The tanks'
    /// contents are kept in the entity's saved compound, under `fluids`, and
    /// the fluid helpers below read and write them.
    #[hook(
        side = "server",
        contract = "replace",
        out = "fluids",
        host = "read",
        neoforge = "RegisterCapabilitiesEvent"
    )]
    fn block_entity_fluids();
}
