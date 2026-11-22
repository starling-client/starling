//! The `block_cook` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0172

pub trait Hooks {
    /// A block finished cooking `item` into `result` at `pos`.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_block_cook",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_cook(pos: [i32; 3], item: &str, result: &str);
}
