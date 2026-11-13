//! The `player_clone` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0248

pub trait Hooks {
    /// A player's data was copied to their new player (NeoForge's
    /// `PlayerEvent.Clone`, from `ServerPlayer.restoreFrom`): `was_death` is
    /// true after a death, false when they left the End after the credits
    /// (`respawn` with `keepAllPlayerData`). The uuid stays the same, so the
    /// old and the new player share `player`. Heard before the new player is
    /// placed, on a server, with no player's state swapped in. Not
    /// cancellable.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.Clone (not cancellable)",
        cancellable = false
    )]
    fn player_clone();
}
