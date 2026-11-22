//! The `prepare_item_enchant` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0325

pub trait Hooks {
    /// Enchanting offers were calculated for a server player at `pos`. Change `costs` before they are sent to the menu.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_prepare_item_enchant",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn prepare_item_enchant(player: u128, name: &str, pos: [i32; 3], costs: &mut [i32; 3]);
}
