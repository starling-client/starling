//! Registered raw custom-payload vocabulary.
#[path = "payload/codec.rs"]
mod codec;
pub use codec::{
    PAYLOAD_DECODE_BUDGET, PAYLOAD_MAX_DEPTH, PAYLOAD_MAX_LIST_ENTRIES, PAYLOAD_MAX_STRING_BYTES,
    PAYLOAD_SCHEMA_MARKER, decode_payload, encode_payload, payload_schema, typed_payload_version,
};
#[path = "payload/registrar.rs"]
mod registrar;
pub use registrar::{
    PayloadRegistrar, RegisterClientPayloadHandlersEvent, RegisterPayloadHandlersEvent,
};
#[path = "payload/typed.rs"]
mod typed;
pub use typed::{PayloadToken, ToClient, ToServer};

use crate::{ClientHost, ConfigurationTaskError, PlayerRef, ServerHost, Text};
use std::sync::Arc;

/// Opaque application bytes. The transport never prepends a mod envelope.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the single public field is the opaque-byte constructor contract.
pub struct PayloadBytes(pub Vec<u8>);

/// A namespaced application channel, validated by the registration boundary.
/// Sends resolve it against the frozen declarations; constructing one grants no authority.
#[derive(Clone, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: the single public field is the channel constructor contract; registration owns validation.
pub struct PayloadId(pub String);

/// The protocols occupied by a registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the closed registration protocol set is configuration, play, or their union.
pub enum PayloadProtocol {
    /// Configuration only.
    Configuration,
    /// Play only.
    Play,
    /// Both configuration and play.
    Common,
}

/// The declared flow; bidirectional declarations must agree on both peers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the closed registration flow set is clientbound, serverbound, or both.
pub enum PayloadDirection {
    /// Server to client.
    ToClient,
    /// Client to server.
    ToServer,
    /// Both directions.
    Bidirectional,
}

/// Where a receiving handler executes. Network handlers must enqueue world work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the two executors define the complete world-access scheduling contract.
pub enum HandlerThread {
    /// The logical receiving main thread (the default).
    Main,
    /// The connection's network executor, without direct world access.
    Network,
}

/// What a send may suffer on a congested connection. `Reliable`, the default, is vanilla's: a
/// queued send is delivered in order or the connection fails. `Droppable` is for data that is
/// worthless late (voice, a live position): a send waits in the engine's own queue for the
/// writer and is dropped, and counted, past a watermark, after 250 ms or at the switch to play,
/// and over the connection's byte budget. Droppable channels are play-only, and a peer that
/// declares the other delivery refuses the join like another version: the delivery is part of
/// the negotiated version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: a send is either never lost or may be dropped; a third kind would change every sender's contract.
pub enum PayloadDelivery {
    /// Queued sends are delivered in order (the default).
    Reliable,
    /// Queued sends may be dropped, never reordered within one channel and sending thread.
    Droppable,
}

/// Marks a `Droppable` declaration in its negotiated version, ahead of a typed schema's marker.
pub const PAYLOAD_DROPPABLE_MARKER: &str = "+delivery.droppable";

/// How many sends of one `Droppable` channel the engine dropped while they waited, by cause,
/// since the game started (this process, both directions and every connection).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the closed set of drop causes the delivery contract names.
pub struct PayloadDropCounts {
    /// Waited longer than 250 ms.
    pub stale: u64,
    /// Pushed out by newer sends past the queue's byte watermark.
    pub watermark: u64,
    /// Refused or pushed out by the connection's byte budget.
    pub budget: u64,
    /// Still waiting when the connection left configuration for play, or ended.
    pub switch: u64,
}

/// A registration's complete compatibility and scheduling declaration.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: callers construct the complete compatibility and scheduling record explicitly.
pub struct PayloadDeclaration {
    /// Exact channel written into vanilla custom-payload framing.
    pub id: PayloadId,
    /// Exact, nonblank schema version; no semver ranges.
    pub version: String,
    /// Protocols in which the channel exists.
    pub protocol: PayloadProtocol,
    /// Receiving directions.
    pub direction: PayloadDirection,
    /// Whether an absent peer declaration permits admission.
    pub optional: bool,
    /// The receiving executor.
    pub thread: HandlerThread,
    /// Whether queued sends may be dropped; `Droppable` is play-only.
    pub delivery: PayloadDelivery,
}

impl PayloadDeclaration {
    /// Constructs a complete declaration with explicit compatibility and scheduling choices.
    /// The registration boundary validates the channel and exact, nonblank version.
    pub fn new(
        id: PayloadId,
        version: String,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
        optional: bool,
        thread: HandlerThread,
        delivery: PayloadDelivery,
    ) -> Self {
        Self {
            id,
            version,
            protocol,
            direction,
            optional,
            thread,
            delivery,
        }
    }
}

/// Queued locally, not a delivery acknowledgement. Disconnect invalidates pending work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: local queue acceptance is the only success state, never a remote acknowledgement.
pub enum PayloadAccepted {
    /// Accepted by the current connection and transaction.
    Queued,
}

/// Which whole-packet framing bound refused a send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: physical framing has exactly encoded and pre-compression bounds.
pub enum PayloadFrameStage {
    /// Contents after the optional compression header and compression.
    Encoded,
    /// Packet bytes before compression, including packet id and channel framing.
    Uncompressed,
}

/// Explicit local refusal. No rejected send writes any application bytes.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: callers can handle every explicit local refusal in the closed Wire result schema.
pub enum PayloadSendError {
    /// No local declaration has this id.
    UnknownPayload(PayloadId),
    /// The optional channel is absent from the compatible peer intersection.
    UnsupportedPeer(PayloadId),
    /// The declaration does not permit this sending direction.
    WrongDirection(PayloadId),
    /// The declaration is not present in the connection's current protocol.
    WrongProtocol(PayloadId),
    /// There is no active connection.
    Disconnected,
    /// The configuration exchange has not frozen a peer table yet.
    NotNegotiated,
    /// Work belongs to a replaced connection incarnation.
    StaleEpoch,
    /// The connection's byte budget cannot accept the message now.
    Backpressure,
    /// Whole-packet framing refused the send, not an arbitrary raw-body cap.
    FrameTooLarge {
        /// Application channel.
        id: PayloadId,
        /// Refusing stage.
        stage: PayloadFrameStage,
        /// Whole-packet size at that stage.
        size: u64,
        /// Inclusive bound at that stage.
        limit: u64,
    },
    /// A connection codec could not prepare the packet.
    EncodeFailed(String),
    /// A raw channel's body is above vanilla's custom-payload limit for this
    /// direction (`ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`, 1,048,576 bytes).
    PayloadTooLarge {
        /// Raw channel.
        id: PayloadId,
        /// Body size in bytes, excluding the channel.
        size: u64,
        /// Inclusive vanilla limit.
        limit: u64,
    },
    /// A typed payload's token is not one this load's registrar handed out for the
    /// declaration, direction and value type it claims: forged, from another load, of another
    /// type, or its owner is off. Nothing is sent.
    StaleToken(PayloadId),
    /// `IPayloadContext::send_to` is for `Droppable` channels only.
    NotDroppable(PayloadId),
}

impl crate::NoBackend for PayloadSendError {
    fn no_backend() -> Self {
        Self::Disconnected
    }
}

/// A context operation cannot run in this phase, side or executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the closed Wire result schema enumerates all context capability refusals.
pub enum PayloadContextError {
    /// Configuration has no authenticated play player.
    NoPlayer,
    /// The requested host is on the opposite logical side.
    WrongSide,
    /// World access requires enqueueWork on the main thread.
    WrongThread,
    /// The connection incarnation has ended.
    Disconnected,
    /// No space remains for queued work.
    Backpressure,
}

/// Status of work submitted through the payload context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: work either completes in the current root or queues on the receiving main thread.
pub enum PayloadWorkStatus {
    /// A main-thread submission ran immediately in the current transaction.
    Complete,
    /// A network-thread submission is scheduled on the receiving main thread.
    Queued,
}

/// A receiving native callback's failure.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: native callback failure.
pub enum PayloadHandleError {
    /// The mod refuses this delivery, such as when application decoding fails.
    Failed(String),
}

/// One receiving owner for a registered id and direction, never an all-mod broadcast.
/// Return `Failed` when application decoding fails. For a typed or declared payload the session
/// names the channel and fails instead of silently dropping the malformed message. For a raw
/// channel (`PayloadRegistrar::play_raw`) a failure logs the owner and the
/// channel, drops only that delivery and rolls back its staged effects (a staged disconnect
/// included); the other owners' copies still run. A successful handler's explicit disconnect
/// and a main-queue refusal disconnect either way. A panic is not `Failed`: it disables this
/// callback for the session, reports the owning mod, and discards the current journal or staged
/// sends without disconnecting or skipping other handlers. Later packets for this callback
/// still undergo framing and channel checks but do not reserve callback queue capacity.
pub trait IPayloadHandler: Send + Sync + 'static {
    /// Handle exact application bytes using connection-derived context.
    fn handle(
        &self,
        payload: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError>;
}

/// Work scheduled on the logical receiving main thread. It retains the handler's owner and
/// disablement guard: a panic aborts the work's transaction and disables that receiving callback
/// for the session, without disconnecting the peer. Later deliveries to that callback
/// follow the same rules as a panic in [`IPayloadHandler::handle`].
pub trait PayloadWork: Send + Sync + 'static {
    /// Execute with the same connection-derived context and transaction rules.
    fn run(&self, context: &mut dyn IPayloadContext) -> Result<(), String>;
}

/// The receiving connection's context, not a player chosen from application bytes.
/// World hosts are available only on the logical main thread. Network handlers
/// use enqueue_work; main-thread submissions execute immediately.
pub trait IPayloadContext {
    /// The sending server player or receiving local player, only in play.
    fn player(&self) -> Result<PlayerRef, PayloadContextError>;
    /// The single receiving direction, never Bidirectional.
    fn flow(&self) -> PayloadDirection;
    /// The current protocol, never Common.
    fn protocol(&self) -> PayloadProtocol;
    /// The server host on the server main thread.
    fn server_host(&mut self) -> Result<&mut dyn ServerHost, PayloadContextError>;
    /// The client host on the client main thread.
    fn client_host(&mut self) -> Result<&mut dyn ClientHost, PayloadContextError>;
    /// Queue a reply on this same connection. It cannot execute inline at the peer.
    fn reply(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError>;
    /// Check a typed payload's token against this load's frozen declarations before a typed
    /// reply: `StaleToken` for one the registrar did not hand out as it claims.
    fn check_payload_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError>;
    /// Queue the same bytes on a `Droppable` channel to each of `players`, from a `Network`
    /// handler or the main thread, on the server only. `players` is the mod's own snapshot,
    /// refreshed on the main thread (a `PlayerRef` is a plain value: keep a list in the handler's
    /// state behind a lock and replace it from a tick or join hook); a player who left or whose
    /// connection is not in play is skipped. Returns how many sends were queued: each follows the
    /// channel's drop rules, so a queued send may still be dropped later. `Reliable` channels
    /// refuse with `NotDroppable`, the client with `WrongDirection`. Sends from the network thread and
    /// the main thread on one channel have no order relative to each other.
    fn send_to(
        &mut self,
        players: &[PlayerRef],
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<u32, PayloadSendError>;
    /// In a server configuration payload handler (on either the main thread or the
    /// network thread), complete the running configuration task matching `task_type`
    /// on this receiving connection (matching NeoForge's
    /// `ServerPayloadContext.finishCurrentTask(ConfigurationTask.Type)` and vanilla's
    /// `ServerConfigurationPacketListenerImpl.finishCurrentTask`).
    ///
    /// Returns `WrongType` if the currently running task on this connection has a
    /// different type, `NotRunning` if no configuration task is currently running or
    /// completion was already requested, `WrongContext` if called on the client or
    /// outside the configuration phase, and `Disconnected` if the connection or its
    /// configuration epoch has ended or a disconnect was already requested. No refusal
    /// advances a task.
    ///
    /// On the main thread, completion takes effect immediately in the configuration
    /// session (without transaction staging; the server advances to the next task or
    /// `FinishConfiguration` on the next tick, after this handler's replies commit).
    /// On the network thread, completion is bound to the currently running task turn
    /// and publishes inside the same active-epoch gate as staged replies after they
    /// succeed; handler failure, an explicit disconnect, or epoch invalidation discards
    /// that staged completion.
    fn finish_configuration_task(
        &mut self,
        task_type: &crate::Identifier,
    ) -> Result<(), ConfigurationTaskError>;
    /// Disconnect this session with the supplied component. On the network thread this
    /// also discards any staged configuration task completion from this handler.
    fn disconnect(&mut self, reason: Text);
    /// Run now on the main thread or schedule there from the network executor.
    /// Failure of the work is reported as a named session failure.
    fn enqueue_work(
        &mut self,
        work: Arc<dyn PayloadWork>,
    ) -> Result<PayloadWorkStatus, PayloadContextError>;
}
