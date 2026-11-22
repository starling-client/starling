//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `network` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait NetworkClientHost {
    /// Side: client.
    /// Opens the chat screen with `text` typed in for the player to finish and
    /// send, as T and / open it; nothing is sent. Returns an error with no
    /// player in a world or while another screen is open.
    fn open_chat(&mut self, text: &str) -> Result<(), String>;
    /// Side: client.
    /// Adds `line` to this client's chat HUD and history (vanilla's
    /// `ChatComponent.addClientSystemMessage`): it shows here only, so
    /// nothing is sent to the server. The Chat option's server-message filter
    /// doesn't hide it (`GuiMessageSource.SYSTEM_CLIENT`), and
    /// `client_chat_received` is not told: vanilla fires that event from
    /// `ChatListener` for lines the server sent, not for lines the client adds
    /// itself. Returns an error if the text exceeds the boundary's size or
    /// nesting limits; rejected text is not added.
    fn chat_line(&mut self, line: Text) -> Result<(), TextFormError>;
    /// Side: client.
    /// A mod's chat line or command (`/` first), sent as `send_chat` sends
    /// it (ClientPacketListener.sendChat / sendCommand), to any server: no
    /// chat history, and no client chat event (vanilla's `ClientChatEvent`
    /// comes only from ChatScreen). An error when the player isn't in a
    /// server's play state or the Chat option blocks the message. API
    /// 404.
    fn send_chat(&mut self, message: &str) -> Result<(), String>;
    /// Side: client.
    /// Builds a vanilla container click; it needs the host because the packet carries hashed stacks.
    /// Uses this connection's registries and component codecs, refusing components that
    /// cannot be encoded or hashed, unrepresentable IDs/slots and more than 128 changed slots.
    /// Applies no menu checks or local effects; submit the result with `send_packet`.
    fn container_click(
        &self,
        request: &ContainerClickRequest,
    ) -> Result<ServerboundPlayPacket, PacketBuildError>;
    /// Side: client.
    /// Submits a vanilla play packet to any server, including unrelated plugin
    /// payloads. The body is validated once and outgoing credit is reserved before
    /// acceptance. Publication waits for the current root transaction to commit;
    /// rollback, disconnect, or a protocol incarnation change cancels it.
    /// Registered and engine-reserved channels are refused: use their payload or
    /// service API. Unrelated plugin bodies retain vanilla's discard-codec limit.
    /// No effects are applied locally. Typed packet builders validate wire fields;
    /// sequence numbers and required client state remain the mod's responsibility.
    /// Chat and commands use `send_chat`, which owns signing. API 409.
    fn send_packet(&mut self, packet: ServerboundPlayPacket) -> Result<(), SendPacketError>;
    /// Side: client. Starts an ordered hold of uncancelled outgoing play packets.
    /// A second hold is refused with Limit while one is active. The limits are 256 packets
    /// and 1,048,576 encoded bytes. Overflow ends and flushes the hold oldest first;
    /// its next release or drop returns Limit. Network-thread replies (keep-alives, chunk batch
    /// acknowledgements, cookies) bypass holds.
    fn hold_packets(&mut self) -> Result<HoldId, HoldError>;
    /// Side: client. Sends the queue oldest first and ends the hold. On failure,
    /// Send reports the number published and the remaining packets are dropped.
    /// Disconnect, protocol changes and aborted travel invalidate the id.
    fn release_held(&mut self, hold: HoldId) -> Result<usize, HoldError>;
    /// Side: client. Ends the hold and discards its packets without publication.
    fn drop_held(&mut self, hold: HoldId) -> Result<(), HoldError>;
    /// Side: client.
    /// Leaves the server or the single-player world (NeoForge's
    /// `IClientCommonPacketListenerExtension.disconnect(Component)`), at the end of this frame,
    /// not inside the calling hook. With a reason, the disconnect screen shows it (a
    /// single-player world is saved first); with none, it leaves as the pause menu's Disconnect
    /// does. The request belongs to the world it was made in: one made while that world closes
    /// is dropped, never carried into the next. A second call in the same frame replaces the
    /// reason. The retained reason renders in the active language at draw and narration time;
    /// literal text stays unchanged. `DisconnectError::Unavailable` when no server or world is joined.
    /// The reason is local to the client, not encoded into a packet.
    fn disconnect(&mut self, reason: Option<Text>) -> Result<(), DisconnectError>;
}
