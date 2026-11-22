//! The `entity_break_door` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0198

pub trait Hooks {
    /// A mob broke the door at `pos`.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_break_door",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_break_door(entity: u32, kind: &str, pos: [i32; 3]);
}
