//! The `player_kicked` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0294

pub trait Hooks {
    /// A guest was disconnected by a server kick, after the hub recorded the rendered `reason`. The guest's state is
    /// swapped in unless they were already removed in the same poll.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerKickEvent)"
    )]
    fn player_kicked();
}
