//! The `client_packet_send` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A play packet the window's game is about to send to the server (`Connection.send`):
    /// every packet of the play phase it sends, in the order it sends them, the selected slot
    /// that goes out before an attack or a use included. Not heard: what the network thread
    /// sends itself (keep-alives, chunk batch acknowledgements, configuration pongs, cookies, the
    /// configuration phase), the answers the
    /// connection sends in whichever phase it is in (a resource pack's status, a dialog's custom
    /// click action), and far terrain requests. Cancel drops the packet: the game goes on as if
    /// it had been sent, so the server never hears of it. Client-side, on any server, single
    /// player's own included. Every packet exposes its protocol type id; custom payloads also
    /// expose their channel and raw bytes, as `client_packet` does for the packets that arrive.
    /// Play-phase pongs pass through this hook; return `Flow::Cancel` to drop a packet, or
    /// mark a later listener `#[listener(receive_cancelled)]`, set `event.cancelled = false`,
    /// and return `Flow::Continue` to clear a cancellation. With no listener, the send path
    /// checks one flag and builds no view.
    /// The immutable `ClientPacket<'_>` view (`id: &Identifier`, `channel: &str`,
    /// `data: &[u8]`) is valid only for the callback; copy what you store (`id.clone()`,
    /// `channel.to_owned()`, `data.to_vec()`). A panic is logged and shown in an error toast
    /// naming the mod, disables only that mod's `client_packet_send` callback for the session,
    /// and leaves the last accepted cancellation unchanged for later eligible listeners (even
    /// if `event.cancelled` was written before panicking). Reassigning `event.packet` only
    /// changes this listener's event; later listeners see the original packet.
    /// Beyond NeoForge: there is no counterpart. Diverges from upstream: new hook.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        cancellable = false,
        event_per_listener = true,
        version = 3,
        neoforge = "none (NeoForge has no raw play-packet send event)"
    )]
    fn client_packet_send(packet: &ClientPacket<'_>, canceled: &mut bool);
}
