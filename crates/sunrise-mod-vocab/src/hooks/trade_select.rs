//! The `trade_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0337

pub trait Hooks {
    /// A server player selected trade `index` from `villager` in the trading menu.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_trade_select",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn trade_select(player: u128, name: &str, villager: Option<u32>, index: usize);
}
