//! Client shutdown completion hook.

pub trait Hooks {
    /// The client has finished shutting down, after `client_stopping`. Fires once
    /// on the render thread for an orderly client exit, not for a dedicated server.
    /// NeoForge's `ClientStoppedEvent`.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ClientStoppedEvent",
        cancellable = false
    )]
    fn client_stopped();
}
