//! The `add_client_reload_listeners` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register client asset reload listeners before a reload stages its
    /// assets (NeoForge's `AddClientReloadListenersEvent`, the client's
    /// counterpart of `add_server_reload_listeners`): the first load
    /// on the loader thread, then every later reload — the player
    /// changed resource packs, F3+T was pressed, or a server sent its packs.
    /// The listener's `prepare` then reads the merged asset view with
    /// resource-pack overrides resolved, and its returned apply publishes the
    /// parsed state, all applies after all prepares, before
    /// `client_resources_reloaded`. Dependencies on the model manager order
    /// both its real model build and its publication. Atlas upload precedes the
    /// model manager. A listener prepare or engine preflight that refuses the
    /// candidate aborts the load before any listener applies.
    /// Client-side, taking a [`ClientReloadListenerContext`]. Later
    /// registration is on the render thread, prepare on the loader, and apply
    /// on the owning game thread. On initial startup before any resources are
    /// published, `context.published()` is `None`; on later reloads it returns
    /// `Some(&dyn ClientReloadHost)`.
    /// Registration errors are returned and logged at the call, without stopping
    /// the reload. A panicking registration callback contributes no listeners or
    /// dependencies; unrelated callbacks still run.
    #[hook(
        receiver = "associated",
        context = "client_reload_listener",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        staged_surface = "listeners",
        neoforge = "AddClientReloadListenersEvent",
        version = 4
    )]
    fn add_client_reload_listeners(listeners: &mut dyn ClientReloadListeners);
}
