//! The `tags_updated` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Tags changed, and what depends on them should be rebuilt (NeoForge's
    /// `TagsUpdatedEvent`); `cause` says how. The place to drop caches
    /// derived from the data and to rebind what depends on it.
    ///
    /// `ServerDataLoad`: the server's data tables are built. The recipes,
    /// tags, loot and advancements of the world's data packs, and the files
    /// mods added through `data_pack_files`, are what the helpers read
    /// from now on. `reload` is false on a world's first open, before its
    /// chunks and players are loaded, and true after `/reload` rebuilt them,
    /// once the players were sent the new data. Fires on the server, the
    /// single player's integrated server included.
    ///
    /// `ClientPacketReceived`: a client applied the block, item and fluid
    /// tags its server sent, and the `tags()` service reads them from now on.
    /// Fires once at login, for configuration's tags, and once for each update in play, on the game
    /// thread. `integrated_server` is true in single player, where both
    /// causes fire: `cause.should_update_static_data()` is then false here,
    /// so that data the two threads share is updated once, on the server.
    ///
    /// Receives a [`ReloadContext`] for published data queries; world handles and
    /// transactions are not present on this context.
    #[hook(
        receiver = "associated",
        context = "reload",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "TagsUpdatedEvent",
        version = 2
    )]
    fn tags_updated(cause: mod_api::TagsUpdateCause);
}
