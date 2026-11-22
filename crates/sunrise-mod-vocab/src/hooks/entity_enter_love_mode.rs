//! The `entity_enter_love_mode` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0203

pub trait Hooks {
    /// A mob entered love mode after player interaction. `player` is the
    /// feeder when known.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_enter_love_mode",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_enter_love_mode(entity: u32, kind: &str, player: Option<u128>);
}
