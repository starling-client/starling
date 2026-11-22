//! The `player_level_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0296

pub trait Hooks {
    /// A server player's experience level changed, after the new level was applied.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_level_change",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerLevelChangeEvent)",
        paper = "PlayerLevelChangeEvent"
    )]
    fn player_level_change(player: u128, name: &str, from: i32, to: i32);
}
