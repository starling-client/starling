//! What follows a block a client writes from the server or a prediction (vanilla's
//! `Level.setBlock` on a `ClientLevel`), out of remote.rs, which may not grow: the moving piston
//! it replaces stops, the block parts hear `on_block_state_change` and the mods hear
//! `client_block_changed`.
use super::Game;
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

impl Game<'_> {
    /// The block at `pos` went from `previous` to `state` in the client's world.
    pub(super) fn client_block_written(
        &mut self,
        pos: [i32; 3],
        previous: BlockStateId,
        state: BlockStateId,
        predicted: bool,
    ) {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        // LevelChunk.setBlockState removes the block entity of a block
        // that changes: the moving piston the client's moveBlocks made
        // stops when the server places what it moved (#3002).
        let facts = self.res.facts();
        let moving = |s: BlockStateId| &*facts.get(s).name == "minecraft:moving_piston";
        if moving(previous) && !moving(state) {
            self.forget_moving_piston(at);
            self.moving_pistons_stale = true;
        }
        if previous == state {
            return;
        }
        let listens = |s| {
            crate::game::api_values::block_state::block_listens(
                mod_api::HookId::OnBlockStateChange,
                s,
            )
        };
        // No event values or level adapter while no part listens: one flag check.
        if mod_api::__private::has_scoped_hook(mod_api::HookId::OnBlockStateChange)
            && (listens(previous) || listens(state))
        {
            self.with_level(|_, level, _| level.block_state_change(at, previous, state, predicted));
        }
        // Then the client-wide listeners, a map's invalidation among them.
        if mod_api::__private::listened(mod_api::HookId::ClientBlockChanged)
            && let Some(dimension) = self.remote.as_ref().and_then(|r| r.heard_level.as_deref())
        {
            self.native_client_block_changed(dimension, at, previous, state, predicted);
        }
    }
}
