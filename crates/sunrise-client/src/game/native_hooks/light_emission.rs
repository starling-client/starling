use super::*;
use crate::game::native_world_read::BlockProviderQuery;

impl Game<'_> {
    pub(in crate::game) fn light_emission_dimension(level: mod_api::LevelRef) -> LevelDimension {
        LevelDimension::of_level(sunrise_core::LevelKey::from_interned_id(
            level.dimension_id(),
        ))
    }

    /// One dynamic provider answer: outer `None` refuses an active cycle, `Some(None)` keeps
    /// the state's own value. Publication is separate from reading this raw answer.
    pub(in crate::game) fn query_dynamic_light_emission(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        dimension: impl FnOnce() -> LevelDimension,
    ) -> Option<Option<u8>> {
        if !crate::game::api_values::block_state::block_listens(
            mod_api::HookId::LightEmission,
            state,
        ) {
            return Some(None);
        }
        self.with_block_provider_query(state, BlockProviderQuery::LightEmission, || {
            let mut emission = None;
            self.native_light_emission(dimension(), pos, state, &mut emission);
            emission
        })
    }

    /// Raw Part dispatch shared by contextual reads and the light publication path.
    #[inline(always)]
    pub(in crate::game) fn native_light_emission(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        emission: &mut Option<u8>,
    ) {
        self.fire_light_emission(dimension, pos, state, emission);
    }
}
