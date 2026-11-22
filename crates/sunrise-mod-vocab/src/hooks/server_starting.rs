//! The `server_starting` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0005

pub trait Hooks {
    /// Fired once before the server accepts play. On a dedicated server this
    /// follows world and data loading, and the adopted packs' English is available.
    /// On an integrated server it follows `server_about_to_start` immediately,
    /// where text uses the client's active language. A failed startup still
    /// reaches `server_stopped`.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_server_starting",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ServerStartingEvent",
        cancellable = false,
        version = 2
    )]
    fn server_starting(dedicated: bool);
}
