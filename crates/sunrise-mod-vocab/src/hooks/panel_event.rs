//! The `panel_event` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0021

pub trait Hooks {
    /// `event` happened on mod `mod_id`'s panel (`native_panel_items`): the
    /// player's click, key or edit, or (`Search`, since version 2) the
    /// owner's own `panel_search` reaching its box, on the first frame that
    /// draws it, with no player edit. Act on it when it is yours.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_panel_event",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "listen",
        neoforge = "none (Sunrise-only; NeoForge has no counterpart)"
    )]
    fn panel_event(mod_id: &str, event: &PanelEvent);
}
