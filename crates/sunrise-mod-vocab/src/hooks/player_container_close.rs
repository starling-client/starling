//! The `player_container_close` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0164

pub trait Hooks {
    /// `player` (`name`) closed the `menu_type` that `player_container_open` told
    /// of (NeoForge's `PlayerContainerEvent.Close`, posted in
    /// `ServerPlayer.doCloseContainer` after the menu's `removed`): they
    /// closed it, walked out of reach, died, changed level, or another menu
    /// opened. A player who leaves closes it before `player_logged_out`
    /// (vanilla posts nothing for a disconnect; Sunrise does, so each open has
    /// its close). Once for each open; not told when the server stops. The
    /// menu's items are where it left them: the block's, and the cursor stack
    /// back in the inventory. State swapped in, when the player has a body.
    /// Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_container_close",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "PlayerContainerEvent.Close",
        cancellable = false
    )]
    fn player_container_close(
        player: u128,
        name: &str,
        dimension: &str,
        menu_type: &str,
        pos: Option<[i32; 3]>,
    );
}
