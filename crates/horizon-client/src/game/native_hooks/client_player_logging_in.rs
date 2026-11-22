use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The client accepted a play login and initialized its level and player
    /// identity (NeoForge's `ClientPlayerNetworkEvent.LoggingIn`). `player_id`
    /// is the server's entity ID. Client-side,
    /// once per connection, including single player.
    pub(in crate::game) fn native_client_player_logging_in(&mut self, player_id: i32, uuid: u128) {
        // Starling: the old hook first, for the mods made for it (native_compat.rs).
        self.native_client_player_network_logging_in();
        self.fire_client_player_logging_in(player_id, uuid);
    }
}
