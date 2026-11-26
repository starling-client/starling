//! The Minecraft Java Edition network protocol (of the version in `sunrise_core::version`):
//! primitive codecs, framing, compression, encryption, and typed packets for
//! every connection state, shared by the Sunrise client and (later) server.
// `deny`, not `forbid`: crypto/blocks.rs allows the ARMv8 AES intrinsics, nothing else.
#![deny(unsafe_code)]
pub mod anvil_cost;
pub mod attachment_sync;
pub mod buf;
pub mod carry_age;
pub mod chunk;
pub mod chunk_batch;
pub mod collider_sync;
pub mod connection;
pub mod connection_groups;
pub mod crypto;
pub mod custom_book;
pub mod entity_data;
pub mod frame;
pub mod general_entity;
pub(crate) mod hash_ops;
pub mod ids;
pub mod item;
pub mod menu_extra;
pub mod mod_list;
pub mod packets;
pub mod passage_sync;
pub mod payload_negotiation;
pub mod quiet;
pub mod registry;
mod resolver;
pub use resolver::{server_redirect, set_name_servers};
pub mod rsa;
pub mod secure_chat;
pub mod servers;
pub mod text;
pub mod viewed_levels;

/// SharedConstants.getProtocolVersion().
pub const PROTOCOL_VERSION: i32 = sunrise_core::version::PROTOCOL_VERSION;
/// The game version servers report in their status.
pub const GAME_VERSION: &str = sunrise_core::VANILLA_VERSION;

/// Connection protocol states (ConnectionProtocol).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    Handshake,
    Status,
    Login,
    Configuration,
    Play,
}
/// PacketFlow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Clientbound,
    Serverbound,
}

/// A malformed or unsupported packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String, Option<ErrorDetail>);
#[derive(Clone, Debug, PartialEq, Eq)]
enum ErrorDetail {
    Payload(packets::custom_payload::PayloadTooLarge),
    Frame(frame::FrameTooLarge),
    UnknownPacketId(i32),
    ExtraBytes { extra: usize },
    SignLine(usize),
}
impl Error {
    const TIMED_OUT: &str = "Timed out";
    const END_OF_STREAM: &str = "Connection closed by the server";
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into(), None)
    }
    /// `IdDispatchCodec.decode`: packet id outside the state/direction table.
    pub fn unknown_packet_id(id: i32) -> Self {
        Self(
            format!("Received unknown packet id {id}"),
            Some(ErrorDetail::UnknownPacketId(id)),
        )
    }
    pub fn is_unknown_packet_id(&self) -> bool {
        matches!(self.1, Some(ErrorDetail::UnknownPacketId(_)))
    }
    /// `PacketDecoder.decode`: unread bytes left after decoding a packet.
    pub fn packet_extra_bytes(id: i32, packet_name: &str, extra: usize) -> Self {
        Self(
            format!("packet {id} ({packet_name}) was larger than expected: {extra} extra bytes"),
            Some(ErrorDetail::ExtraBytes { extra }),
        )
    }
    pub fn extra_packet_bytes(&self) -> Option<usize> {
        match self.1 {
            Some(ErrorDetail::ExtraBytes { extra }) => Some(extra),
            _ => None,
        }
    }
    /// A sign line past `ServerboundSignUpdatePacket.MAX_STRING_LENGTH`; `line` is its index.
    pub fn sign_line_too_long(line: usize, max: usize) -> Self {
        Self(
            format!("sign line {line} longer than {max} chars"),
            Some(ErrorDetail::SignLine(line)),
        )
    }
    /// The index of the sign line a validation refused as too long.
    pub fn oversize_sign_line(&self) -> Option<usize> {
        match self.1 {
            Some(ErrorDetail::SignLine(line)) => Some(line),
            _ => None,
        }
    }
    /// A directional custom payload exceeded its body capacity.
    pub fn payload_too_large(id: &str, size: usize, limit: usize) -> Self {
        let detail = packets::custom_payload::PayloadTooLarge {
            id: id.into(),
            size,
            limit,
        };
        Self(
            format!("PayloadTooLarge {{ id: {id}, size: {size}, limit: {limit} }}"),
            Some(ErrorDetail::Payload(detail)),
        )
    }
    /// Structured size refusal, retained for callers that expose their own error vocabulary.
    pub fn payload_limit(&self) -> Option<&packets::custom_payload::PayloadTooLarge> {
        match &self.1 {
            Some(ErrorDetail::Payload(detail)) => Some(detail),
            _ => None,
        }
    }
    /// A packet cannot fit the connection's framing or compression pipeline.
    pub fn frame_too_large(detail: frame::FrameTooLarge) -> Self {
        Self(
            format!("FrameTooLarge {detail:?}"),
            Some(ErrorDetail::Frame(detail)),
        )
    }
    /// Structured whole-packet refusal, before any bytes are written.
    pub fn frame_limit(&self) -> Option<&frame::FrameTooLarge> {
        match &self.1 {
            Some(ErrorDetail::Frame(detail)) => Some(detail),
            _ => None,
        }
    }
    /// The read timeout passed with nothing received (netty's
    /// `ReadTimeoutException`, which `Connection.exceptionCaught` reports as
    /// `disconnect.timeout`).
    pub fn timed_out() -> Self {
        Self::new(Self::TIMED_OUT)
    }
    pub fn is_timed_out(&self) -> bool {
        self.0 == Self::TIMED_OUT
    }
    /// The peer closed the stream (`Connection.channelInactive`).
    pub fn end_of_stream() -> Self {
        Self::new(Self::END_OF_STREAM)
    }
    pub fn is_end_of_stream(&self) -> bool {
        self.0 == Self::END_OF_STREAM
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Engine-owned packet allocation accounting.
pub mod retained;
