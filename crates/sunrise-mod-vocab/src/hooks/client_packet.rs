//! The `client_packet` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A play packet from the server reached the window's game, decoded,
    /// just before the game handles it (`Connection.genericsFtw` handing it
    /// to `ClientPacketListener`): every packet of the play phase, custom
    /// payloads included, in the order the server sent them. Control packets
    /// are not heard, whether standalone or bundled: keep-alives, chunk batch
    /// starts and finishes, bundle delimiters, resource pack pushes and pops,
    /// cookies, server links, transfers, disconnects, the configuration switch,
    /// `minecraft:brand` and far terrain tiles; neither is the configuration
    /// phase.
    /// Read-only: the game handles the packet as it would without the mod.
    /// Client-side, on any server, single player's own included.
    /// Every packet exposes its protocol type id. Custom payloads also expose
    /// their channel and raw bytes; no other decoded fields cross the API.
    #[notes = r#"## Decoded client packets

`Hooks::client_packet` receives `&mut dyn ClientHost` and a read-only
`ClientPacketEvent` before the window handles a decoded server play packet.
The read-only `event.packet` is a `mod_api::ClientPacket`: `id` identifies the
packet type (for example, `minecraft:custom_payload`); `custom_payload` contains
the channel and raw bytes for a custom payload, or `None` otherwise. An id
outside the protocol's packet table is reported as `sunrise:unknown`.
Other decoded fields are not exposed.

The hook cannot cancel or replace a packet. It also observes chunk packets
handled by the batching path, once each. Configuration and control packets (including keep-alives,
chunk batch starts and finishes, and resource pack messages) do not fire it. Standalone chunk batch
controls run on the receive thread; bundled controls run with their members on the main thread.
The chunks between a batch's start and finish still fire it.

`ClientHost::hold_packets` retains uncancelled play packets, unmodified and in order, up to 256
packets or 1,048,576 encoded bytes. Release sends oldest first; drop discards them. Overflow ends
and flushes the hold; its next release/drop returns `HoldError::Limit` with those configured bounds.
A failed release reports `HoldError::Send { sent, error }` and discards the unsent tail.
Starting another hold while one is active returns `HoldError::Limit` with the configured bounds.
Disconnect, protocol incarnation changes
and aborted travel invalidate holds. Keep-alives, chunk batch acknowledgements, configuration,
cookies and pack answers bypass the queue, standalone or bundled. Held packets expose no additional
inspection or editing API.

`Hooks::client_packet_send` is the other direction: a `ClientPacketSendEvent` with the same
`ClientPacket` view for each play packet the window's game is about to send (`Connection.send`),
in order, including the selected slot sent before an attack or a use. Returning `Flow::Cancel` drops
the packet, and the game goes on as if it had been sent. Play-phase pongs pass through this hook.
Control replies (keep-alives, chunk batch acknowledgements, configuration pongs and cookies),
configuration packets, the phase-dependent answers (resource pack status and a dialog's custom
click action), and far terrain requests do not fire it. With no listener the
send path checks one flag and builds nothing."#]
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        cancellable = false,
        version = 2,
        neoforge = "none (NeoForge has no raw play-packet receive event)"
    )]
    fn client_packet(packet: &ClientPacket);
}
