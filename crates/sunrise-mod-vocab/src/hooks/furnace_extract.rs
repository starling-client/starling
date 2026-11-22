//! The `furnace_extract` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0221

pub trait Hooks {
    /// A server player took an output from a furnace at `pos`; `item` is the output item id and
    /// `count` is how many were taken. The player's state is not swapped in.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_furnace_extract",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_extract(player: u128, name: &str, pos: [i32; 3], item: &str, count: u32);
}
