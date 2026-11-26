//! `IBlockExtension.isScaffolding` for the block a climbing player stands in.
use crate::game::api_values::{TryIntoApi, block_state::block_listens};

/// Ask the parts of `state`'s block whether it is scaffolding to the player `entity` at `pos`:
/// `scaffolding` arrives with vanilla's answer and the parts may change it. Both sides.
pub(crate) fn is_scaffolding(
    context: &sunrise_world::native_load_context::LoopContext,
    entity: u32,
    pos: sunrise_core::position::BlockPos,
    state: sunrise_world::registry::BlockStateId,
    scaffolding: &mut bool,
) {
    if !block_listens(mod_api::HookId::IsScaffolding, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_is_scaffolding(
        &sunrise_client_render::api_context::LoopReader(context).context(),
        entity,
        pos,
        state,
        scaffolding,
    );
}
