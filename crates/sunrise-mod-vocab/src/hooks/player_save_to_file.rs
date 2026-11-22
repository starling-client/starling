//! The `player_save_to_file` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0260

pub trait Hooks {
    /// A player was saved to the world's store (NeoForge's
    /// `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the
    /// world closes, when a dedicated or integrated server saves its guests,
    /// and for an integrated server's owner when the world saves them.
    /// Once for each save. `directory`
    /// is the world's player directory (`players/data`): write your own file
    /// there, named `<uuid>.<suffix>`, never the player's `.dat`. The player
    /// may be leaving, so no player's state is swapped in. Not cancellable.
    /// Autosave snapshots are written asynchronously: this notification runs
    /// on a later server tick after a successful write, possibly after WorldSaved.
    /// Live state may have advanced beyond the saved snapshot; a process exit
    /// before completions are drained can prevent notification delivery.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_save_to_file",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.SaveToFile (not cancellable)",
        cancellable = false
    )]
    fn player_save_to_file(player: u128, name: &str, directory: &std::path::Path);
}
