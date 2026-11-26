//! The `player_pong` hook: a play-phase packet notice on the logical server.

pub trait Hooks {
    /// A connected `player` sent a play-phase pong (`ServerCommonPacketListenerImpl.handlePong`).
    /// Runs on the server thread at the packet's turn in that connection's inbox. `name` is
    /// the player's profile name; `id` is the signed 32-bit value the client sent.
    /// Unsolicited and repeated IDs are reported as-is: an ID does not prove the client
    /// answered a particular `PlayerRef::send_ping`. This notice is not cancellable and
    /// does not affect keep-alive latency. Configuration and status traffic do not fire it.
    /// Each listener gets its own event.
    #[hook(side = "server", contract = "listen", event_per_listener = true)]
    fn player_pong(player: PlayerRef, name: &str, id: i32);
}
