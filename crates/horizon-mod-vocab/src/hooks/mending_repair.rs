//! The `mending_repair` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0213

pub trait Hooks {
    /// The selected damaged Mending `stack` and the proposed durability repair
    /// for `experience` points. The amount is clamped to the stack's current damage. Zero leaves the
    /// experience unspent when it overrides a positive proposal. If the selected
    /// stack changes, the repair is skipped.
    /// Server-side, including `PlayerRef::give_experience_points`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge only scales a repair with IItemExtension.getXpRepairRatio)"
    )]
    fn mending_repair();
}
