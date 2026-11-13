//! The `client_information_updated` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ClientInformationUpdatedEvent`: a server player's client sent new
    /// client information (language, view distance, chat, skin parts, main hand,
    /// filtering, listing, particles) in play. `old` and `new` are the values before and
    /// after this packet, one call per packet; they can be equal. The join's own
    /// information (the configuration phase, `ServerPlayer`'s constructor) is not
    /// reported. The player is not swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ClientInformationUpdatedEvent",
        cancellable = false
    )]
    fn client_information_updated();
}
