//! The `projectile_launch` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0327

pub trait Hooks {
    /// A server projectile was added to the level. `player` is set only for
    /// a player launch; other sources have an empty `name`.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_projectile_launch",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn projectile_launch(player: Option<u128>, name: &str, entity: u32, kind: &str);
}
