use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// `IItemExtension.canPerformAction`: can `stack` perform the item ability
    /// `ability` (`shield_block`, `shears_harvest`...)?
    /// `can_perform` arrives with vanilla's answer; the parts of the stack's
    /// item may set it. Both sides.
    #[inline(never)]
    pub(in crate::game) fn native_can_perform_action(
        &self,
        stack: &ItemStack,
        ability: &str,
        can_perform: &mut bool,
    ) {
        let Ok(metadata) = crate::game::api_values::stack_read_metadata(stack) else {
            return;
        };
        let project = |metadata| crate::game::api_values::project_stack(stack, metadata).ok();
        let host: &dyn mod_api::Host = self;
        let context = mod_api::ItemAbilityContext::new(host.tags());
        Self::fire_can_perform_action(
            &context,
            mod_api::__private::CanPerformActionCall {
                stack: mod_api::ItemStackView::resident(metadata, &project),
                ability,
                can_perform,
            },
        );
    }
}
