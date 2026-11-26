//! The address and identity available during a client's initial network configuration.

use crate::Identifier;

/// Read-only Host for [`crate::Hooks::client_identity`] on the network thread.
/// It has no access to live game state or to the connection itself.
pub trait ClientIdentityHost {
    /// The original target of this join, before SRV or DNS resolution. For a
    /// transfer this is the new destination supplied by the server.
    fn server_address(&self) -> &str;
}

// no wire: an in-process hook reply; the network thread encodes vanilla packets.
/// The complete identity to report during initial configuration, in place of
/// the engine's default identity for this server.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientIdentity {
    /// The protocol brand, not display text. It is sent first on `minecraft:brand`.
    pub brand: String,
    /// Additional payloads sent after the brand, in order. An empty list sends only the brand.
    pub payloads: Vec<ClientIdentityPayload>,
}

impl ClientIdentity {
    /// Choose the brand and the ordered list of additional configuration payloads.
    pub fn new(brand: impl Into<String>, payloads: Vec<ClientIdentityPayload>) -> Self {
        Self {
            brand: brand.into(),
            payloads,
        }
    }
}

// no wire: an in-process hook reply carries the application's already-encoded bytes.
/// An additional custom payload for the server's initial configuration listener.
/// Reserved channels and channels declared through Sunrise's payload transport
/// cannot be sent here. Use [`ClientIdentity::brand`] to set `minecraft:brand`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientIdentityPayload {
    /// The payload's namespaced channel.
    pub channel: Identifier,
    /// The exact application bytes, at most 32,767 bytes (`ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`).
    pub bytes: Vec<u8>,
}

impl ClientIdentityPayload {
    /// Choose the channel and the exact bytes following the channel identifier on the wire.
    pub fn new(channel: Identifier, bytes: Vec<u8>) -> Self {
        Self { channel, bytes }
    }
}
