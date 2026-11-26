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
    /// Client-side, on any server, single player's own included. With no
    /// listener, the receive path checks one flag and builds no view.
    /// Every packet exposes its protocol type id (`&Identifier`). Custom payloads also expose
    /// their channel (`&str`) and raw bytes (`&[u8]`); no other decoded fields or mutable
    /// bytes cross the API.
    /// The immutable `ClientPacket<'_>` view is valid only for the callback; copy what you
    /// store (`id.clone()`, `channel.to_owned()`, `data.to_vec()`). A panic is logged and
    /// shown in an error toast naming the mod, disables only that mod's `client_packet`
    /// callback for the session, and does not stop later listeners. Reassigning `event.packet`
    /// only changes this listener's event; later listeners see the original packet.
    #[notes = r#"## Decoded client packets

`Hooks::client_packet` receives `&mut dyn ClientHost` and a `ClientPacketEvent<'_>`
before the window handles a decoded server play packet. `event.packet` borrows
an immutable callback-lifetime `mod_api::ClientPacket<'a>`: `id: &'a Identifier`
identifies the packet type (for example, `minecraft:custom_payload`); `custom_payload`
is `Option<ClientPayload<'a>>` (`channel: &'a str`, `data: &'a [u8]`) for a custom
payload, or `None` otherwise. An id outside the protocol's packet table is reported as
`sunrise:unknown`. Other decoded fields and mutable packet bytes are not exposed. Copy
only what you retain after the callback returns (`id.clone()`, `channel.to_owned()`,
`data.to_vec()`). With no listener, the receive path checks one flag and builds nothing.

The hook cannot cancel or replace a packet. It also observes chunk packets
handled by the batching path, once each. Configuration and control packets (including keep-alives,
chunk batch starts and finishes, and resource pack messages) do not fire it. Standalone chunk batch
controls run on the receive thread; bundled controls run with their members on the main thread.
The chunks between a batch's start and finish still fire it.

`ClientHost::hold_packets` gives the calling mod its own hold. Overlapping holds share one
ordered queue; releasing one cannot send past another. Dropping one discards only packets it
covered, including overlaps, without ending any other hold. Held packets keep their ordinary
writer credit and are never flushed to make room. Disconnect, protocol incarnation changes and
aborted travel invalidate holds. Keep-alives, chunk batch acknowledgements, configuration,
cookies and pack answers bypass holds, standalone or bundled. A hold exposes no packet bodies.

`Hooks::client_packet_send` is the other direction: a `ClientPacketSendEvent<'_>` with the same
borrowed `ClientPacket<'_>` view for each play packet the window's game is about to send
(`Connection.send`), in order, including the selected slot sent before an attack or a use.
Returning `Flow::Cancel` drops the packet unless a later `#[listener(receive_cancelled)]`
listener sets `event.cancelled = false` and returns `Flow::Continue`. Play-phase pongs pass
through this hook. Control replies (keep-alives, chunk batch acknowledgements, configuration
pongs and cookies), configuration packets, the phase-dependent answers (resource pack status
and a dialog's custom click action), and far terrain requests do not fire it. With no listener
the send path checks one flag and builds nothing.

Both hooks give later listeners the original borrowed view. A panic logs and shows an error
toast naming the mod, disables only its callback for that hook, and leaves any accepted send
cancellation unchanged. Later eligible listeners still run."#]
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        cancellable = false,
        event_per_listener = true,
        version = 3,
        neoforge = "none (NeoForge has no raw play-packet receive event)"
    )]
    fn client_packet(packet: &ClientPacket<'_>);
}
