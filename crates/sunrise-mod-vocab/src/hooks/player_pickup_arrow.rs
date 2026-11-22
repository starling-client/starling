//! The `player_pickup_arrow` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0302

pub trait Hooks {
    /// A server player picked up an arrow after it was removed and its pickup was applied. `item` is
    /// the arrow's pickup item id. Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_pickup_arrow",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_pickup_arrow(player: u128, name: &str, entity_id: u32, item: &str);
}
