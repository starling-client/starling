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
        let _dispatch = AnvilUpdateDispatch::enter();
        self.fire_anvil_update(player, player_entity_id, pos, inputs, edits, cancel);
    }

    /// `PlayerRef::anvil_update`: a mod's own anvil menu runs the `anvil_update` listeners
    /// (`CommonHooks.onAnvilUpdate`) for `player`, from `Keep` and the menu's costs. None when
    /// a listener canceled, for a stale player or a client's other players, and from inside an
    /// `anvil_update` listener.
    pub(in crate::game) fn player_anvil_update(
        &mut self,
        player: mod_api::PlayerRef,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
    ) -> Option<mod_api::item::menu::AnvilUpdateEdits> {
        use mod_api::item::menu::{AnvilUpdateEdits, StackResult};
        if AnvilUpdateDispatch::running() {
            return None;
        }
        let entity = i32::try_from(player.entity().id()).ok()?;
        let mut answer = None;
        let mut run = |game: &mut Self| {
            let mut edits = AnvilUpdateEdits {
                output: StackResult::Keep,
                xp_cost: inputs.vanilla_xp_cost,
                material_cost: inputs.vanilla_material_cost,
            };
            let mut cancel = false;
            game.native_anvil_update(player.uuid(), entity, pos, inputs, &mut edits, &mut cancel);
            answer = (!cancel).then_some(edits);
        };
        // A server runs it in the player's turn, as its own menu does; a client for its
        // local player only.
        if self.is_logical_server() {
            self.native_as_player_handle(player, run);
        } else if self.local_player() == Some(player) {
            run(self);
        }
        answer
    }
}

/// Marks an `anvil_update` dispatch on this thread while it runs, so a listener's
/// `PlayerRef::anvil_update` declines instead of running the listeners again without bound.
struct AnvilUpdateDispatch(bool);

thread_local! {
    static ANVIL_UPDATE_RUNNING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl AnvilUpdateDispatch {
    fn enter() -> Self {
        Self(ANVIL_UPDATE_RUNNING.with(|running| running.replace(true)))
    }
    fn running() -> bool {
        ANVIL_UPDATE_RUNNING.with(std::cell::Cell::get)
    }
}

impl Drop for AnvilUpdateDispatch {
    fn drop(&mut self) {
        ANVIL_UPDATE_RUNNING.with(|running| running.set(self.0));
    }
}
