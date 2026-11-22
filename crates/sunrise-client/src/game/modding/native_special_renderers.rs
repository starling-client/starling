//! Game-side stack and draw context for special item renderers. The client item renderer owns
//! scoped submission, rollback and the session's callback failure policy.
use super::{
    Game,
    api_values::{TryIntoApi, engine_display_context, stack_value},
};
use mod_api::{ItemDisplayContext, ItemStack, SpecialModelContext};
use std::num::NonZeroU32;
use sunrise_assets::item_model::{Context, DisplayContext, StackView};
use sunrise_client::item_render::{SpecialModelInput, stack_view};

pub(in crate::game) fn draw_input<'a>(
    game: &'a Game<'_>,
    stack: &StackView,
    context: &Context<'_>,
) -> Option<SpecialModelInput<'a>> {
    // Match vanilla's `ItemModelResolver.updateForTopItem`: empty stacks are skipped cleanly.
    let count = u32::try_from(stack.count).ok().and_then(NonZeroU32::new)?;
    let item = mod_api::__private::item_key(&stack.item)?;
    let Ok(display) = context.display.try_into_api();
    Some(SpecialModelInput {
        host: game,
        item,
        count,
        context: SpecialModelContext::new(
            display,
            context.level.map(|level| level.game_time),
            game.frame_world_partial_tick() as f32,
        ),
    })
}

/// Convert only an explicitly submitted nested stack; ordinary input stays borrowed.
pub(in crate::game) fn nested_stack(
    stack: &ItemStack,
    display: ItemDisplayContext,
) -> Option<(StackView, DisplayContext)> {
    let display = engine_display_context(display)?;
    // The item resolver's stack count is signed.
    i32::try_from(stack.count()).ok()?;
    let stack = stack_value(stack).ok()?;
    Some((stack_view(&stack), display))
}

#[cfg(test)]
#[path = "native_special_renderers/cost_tests.rs"]
mod cost_tests;
#[cfg(test)]
#[path = "native_special_renderers/frame_tests.rs"]
pub(in crate::game) mod frame_tests;
