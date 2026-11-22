use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// An anvil at `pos` made vanilla's result with a left input, for
    /// `player` (UUID) and `player_entity_id` (`AnvilUpdateEvent`, after
    /// `AnvilMenu.createResult`), whenever an input or the name changed:
    /// lend `inputs` and let listeners stage only the result and cost
    /// `edits`. Both sides: the server's menu and a server's client menu
    /// with the local player.
    pub(in crate::game) fn native_anvil_update(
        &mut self,
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
        edits: &mut mod_api::item::menu::AnvilUpdateEdits,
        cancel: &mut bool,
    ) {
        self.fire_anvil_update(player, player_entity_id, pos, inputs, edits, cancel);
    }
}
