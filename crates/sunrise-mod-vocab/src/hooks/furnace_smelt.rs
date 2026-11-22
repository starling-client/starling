//! The `furnace_smelt` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0222

pub trait Hooks {
    /// A furnace finished smelting `item` into `result` at `pos`. Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_furnace_smelt",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_smelt(pos: [i32; 3], item: &str, result: &str);
}
