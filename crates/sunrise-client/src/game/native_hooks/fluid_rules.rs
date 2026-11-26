use super::*;

impl GameLevel<'_> {
    /// How a mod's world fluid flows at `pos`, starting from what it declared (NeoForge's
    /// `BaseFlowingFluid` and `FluidType` getters: `getTickDelay`, `getDropOff`,
    /// `getSlopeFindDistance`, `canConvertToSource`). `fluid` is the id it was declared with.
    /// Asked for every flow step and every cell a spread looks at: answer fast. A negative
    /// `drop_off` answer is refused and the declared value stands. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_fluid_rules(&self, call: mod_api::__private::FluidRulesCall<'_>) {
        self.fire_fluid_rules(call);
    }

    /// What a mod's world fluid does at `pos` in `dimension`: `registered`, then as the parts of the
    /// fluid answer through `dispatch`. A negative `drop_off` would compute a level outside
    /// `FlowingFluid.LEVEL`, so that answer alone is refused. The one place that builds the hook's
    /// call, for a block-behaviour level and for a mod's read alike.
    pub(in crate::game) fn answered_fluid_rules(
        dimension: sunrise_core::dimension::Dimension,
        pos: BlockPos,
        fluid: sunrise_world::fluid::ModFluid,
        registered: sunrise_world::fluid::ModFluidRules,
        dispatch: impl FnOnce(mod_api::__private::FluidRulesCall<'_>),
    ) -> sunrise_world::fluid::ModFluidRules {
        let mut rules = registered;
        dispatch(mod_api::__private::FluidRulesCall {
            dimension,
            pos,
            fluid: &fluid.get().id,
            tick_delay: &mut rules.tick_delay,
            drop_off: &mut rules.drop_off,
            slope_find_distance: &mut rules.slope_find_distance,
            converts_to_source: &mut rules.converts_to_source,
        });
        if rules.drop_off < 0 {
            log::warn!(
                "fluid_rules: {} answered drop_off {} at {pos:?}; keeping {}",
                fluid.get().id,
                rules.drop_off,
                registered.drop_off
            );
            rules.drop_off = registered.drop_off;
        }
        rules
    }
}
