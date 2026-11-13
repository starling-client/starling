//! The `net_from_client` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0045

pub trait Hooks {
    /// A message from `player`'s client on `channel` (`mod_id:name`), within
    /// the caps (`native_net.rs`): check `channel`, and treat `data` as the
    /// player's own bytes, checked by nothing. Runs in the tick after it
    /// arrives, with no player's state swapped in. Server-side. The NeoForge
    /// counterpart is the registered `IPayloadHandler.handle` callback, not
    /// the registration event.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "IPayloadHandler.handle"
    )]
    fn net_from_client();
}
