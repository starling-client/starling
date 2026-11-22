//! A server mod's respawn of a dead player (`PlayerRef::respawn`): the Respawn button, pressed
//! for them.
use super::{Game, native_server::PlayerRead};
use mod_api::{PlayerRef, RespawnRefusal};

impl Game<'_> {
    /// The Respawn button for a dead player (`handleClientCommand`'s `PERFORM_RESPAWN`), outside
    /// every player's hook. Each refusal is a variant of [`RespawnRefusal`], checked in its order.
    pub(super) fn player_respawn(&mut self, player: PlayerRef) -> Result<(), RespawnRefusal> {
        if self.remote.is_some() {
            return Err(RespawnRefusal::Client);
        }
        if self.lan_acting.is_some() || !self.player_state.cloning_players.is_empty() {
            return Err(RespawnRefusal::InPlayerHook);
        }
        let (conn, has_health) = match self.player_handle_read(player) {
            None => {
                let guest = self
                    .online_player_record(player)
                    .filter(|g| g.respawning)
                    .ok_or(RespawnRefusal::Stale)?;
                (guest.conn, guest.health > 0.0)
            }
            // The world's own player has no connection: it respawns through its own screen.
            Some(PlayerRead::Acting { .. }) => return Err(RespawnRefusal::NoConnection),
            Some(PlayerRead::Guest(guest)) if guest.is_fake() => {
                return Err(RespawnRefusal::NoConnection);
            }
            Some(PlayerRead::Guest(guest)) => (guest.conn, guest.health > 0.0),
        };
        // `handleClientCommand`: the credits button respawns even a living player.
        let won_game = self
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .is_some_and(|play| play.won_game);
        if has_health && !won_game {
            return Err(RespawnRefusal::Alive);
        }
        self.guest_respawn(conn);
        Ok(())
    }
}
