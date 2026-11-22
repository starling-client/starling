//! The `item_despawn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0230

pub trait Hooks {
    /// A dropped item entity reached its 6000-tick lifetime and is about to be
    /// removed (Bukkit's `ItemDespawnEvent`).
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_item_despawn",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's ItemDespawnEvent)"
    )]
    fn item_despawn(entity: u32, item: &str, count: i32, pos: [f64; 3]);
}
