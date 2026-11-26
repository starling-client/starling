use super::*;

impl Game<'_> {
    /// Observes the play pong at its ordered inbox turn, with the live server host in place.
    pub(in crate::game) fn native_player_pong(&mut self, call: fire::PlayerPongCall<'_>) {
        self.fire_player_pong(call);
    }
}
