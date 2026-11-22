//! The `player_stop_spectating_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0312

pub trait Hooks {
    /// A server player stopped spectating `entity_id`, after their camera returned to the player.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_stop_spectating_entity",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_stop_spectating_entity(player: u128, name: &str, entity_id: i32);
}
