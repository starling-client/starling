// game-module-visibility: pub(crate)
//! The item Part methods the client library asks through `native_client_hooks` about a stack's
//! rules in a menu: `can_grindstone_repair` and `can_fit_inside_container_items`. Only a stack
//! whose item has a subscribing part is converted.
use super::api_values::TryIntoApi;
use horizon_items::items::ItemStack;
use horizon_world::native_load_context::LoopContext;

/// The game's side of `horizon_client::stations::native_can_grindstone_repair`.
pub(crate) fn can_grindstone_repair(
    context: &LoopContext,
    stack: &ItemStack,
    can_repair: &mut bool,
) {
    if !mod_api::__private::item_listens(mod_api::HookId::CanGrindstoneRepair, &stack.item) {
        return;
    }
    let Ok(stack) = stack.try_into_api() else {
        return;
    };
    mod_api::dispatch_can_grindstone_repair(
        &horizon_client_render::api_context::LoopReader(context).context(),
        &stack,
        can_repair,
    );
}

/// The game's side of `native_can_fit_inside_container_items`, which a shulker box's slots ask
/// (`horizon_client::menu::shulker_box_slot_takes`).
pub(crate) fn can_fit_inside_container_items(
    context: &LoopContext,
    stack: &ItemStack,
    fits: &mut bool,
) {
    if !mod_api::__private::item_listens(mod_api::HookId::CanFitInsideContainerItems, &stack.item) {
        return;
    }
    let Ok(stack) = stack.try_into_api() else {
        return;
    };
    mod_api::dispatch_can_fit_inside_container_items(
        &horizon_client_render::api_context::LoopReader(context).context(),
        &stack,
        fits,
    );
}
