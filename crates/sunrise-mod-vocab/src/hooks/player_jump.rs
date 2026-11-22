//! The `player_jump` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0293

pub trait Hooks {
    /// A guest's accepted movement made the server jump them from the ground. Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_jump",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerJumpEvent)"
    )]
    fn player_jump(player: u128, name: &str);
}
