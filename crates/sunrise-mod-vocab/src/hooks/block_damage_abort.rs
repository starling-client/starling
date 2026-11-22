//! The `block_damage_abort` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0173

pub trait Hooks {
    /// A server player stopped digging `pos` before the block broke, after
    /// the server cleared the dig and sent the block back to the client.
    /// The player's state is not swapped in.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_block_damage_abort",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockDamageAbortEvent)"
    )]
    fn block_damage_abort(player: u128, name: &str, pos: [i32; 3]);
}
