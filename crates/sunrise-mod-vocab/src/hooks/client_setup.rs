//! The `client_setup` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `FMLClientSetupEvent`: the client has started. It fires once per game
    /// process, on the render thread, after the first client load has applied (the
    /// atlas, models and every client reload listener's published state are final)
    /// and before the title screen's first frame. Returning to the title screen and
    /// opening another world is not a new start, so
    /// it never fires again. This is
    /// the opposite of `client_resources_reloaded`, which never runs at initial
    /// startup and runs after every later reload. The listener runs on the main thread;
    /// work a mod wants done in parallel is its own. The dedicated server has no client and never fires it
    /// (see `server_about_to_start` for its `FMLDedicatedServerSetupEvent`). Receives a
    /// [`ClientSetupContext`] for client setup and published resource queries without
    /// world handles or transactions.
    #[hook(
        receiver = "associated",
        context = "client_setup",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "FMLClientSetupEvent",
        cancellable = false,
        version = 2
    )]
    fn client_setup();
}
