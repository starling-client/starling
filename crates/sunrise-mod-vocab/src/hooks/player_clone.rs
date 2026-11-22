//! The `player_clone` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0248

pub trait Hooks {
    /// A player's data was copied to their new player (NeoForge's
    /// `PlayerEvent.Clone`, from `ServerPlayer.restoreFrom`): `was_death` is
    /// true after a death, false when they left the End after the credits
    /// (`respawn` with `keepAllPlayerData`). `original` is a direct,
    /// callback-scoped loan ([`OriginalPlayer`]) of the retained original
    /// player, enforced at compile time so it cannot outlive the hook. It
    /// supports attachment reads on the old player without default insertion.
    /// Saved values decode once, and cache writes, explicit writes and removals
    /// are journaled to `host.transaction` without touching the replacement.
    /// It also provides `with_inventory` over the 43-slot
    /// vanilla inventory, `persistent_data`, health, experience, active
    /// effects, position, source dimension and source level. `player` is the
    /// replacement's UUID, already assigned its final level, including any
    /// `player_respawn_position` redirect. Heard after removal of the old
    /// player's menus and after the data copy, before placement or bulk
    /// attachment publication. Old bulk tokens are stale. Not cancellable.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_clone",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "listen",
        neoforge = "PlayerEvent.Clone (not cancellable)",
        cancellable = false
    )]
    fn player_clone(
        player: u128,
        name: &str,
        was_death: bool,
        original: mod_api::OriginalPlayer<'_>,
    );
}
