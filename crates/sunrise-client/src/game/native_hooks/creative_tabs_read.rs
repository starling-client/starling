//! Client item-model lighting queries.
use super::*;
use crate::game::api_values::engine_display_context;
use sunrise_client::cit::Hand as CitHand;

impl Game<'_> {
    /// `ItemStackRenderState.usesBlockLight` of `stack` resolved for `context`.
    pub(in crate::game) fn item_uses_block_light(
        &self,
        stack: &ItemStack,
        context: mod_api::ItemDisplayContext,
    ) -> bool {
        let Some(display) = engine_display_context(context) else {
            return false;
        };
        self.resolve_item_in_level(stack, display, None, None, 0, CitHand::Main)
            .uses_block_light()
    }
}
