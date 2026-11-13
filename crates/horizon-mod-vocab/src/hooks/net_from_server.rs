//! The `net_from_server` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0046

pub trait Hooks {
    /// A message from the server on `channel` (`mod_id:name`), whose `data`
    /// is the sender's own bytes. The window's game only. The NeoForge
    /// counterpart is the registered `IPayloadHandler.handle` callback, not
    /// the registration event.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "IPayloadHandler.handle"
    )]
    fn net_from_server();
}
