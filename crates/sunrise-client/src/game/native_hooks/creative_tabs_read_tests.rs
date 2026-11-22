//! Client item-model lighting queries.
#![cfg(test)]

/// A block item's model is lit from the side, a flat item's from the front.
#[test]
#[ignore = "daily"]
fn block_items_use_block_light_and_flat_items_do_not() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let light = |id: &str, context| {
            let stack = sunrise_client::items::ItemStack::new(id, 1);
            game.item_uses_block_light(&stack, context)
        };
        for context in [
            mod_api::ItemDisplayContext::Gui,
            mod_api::ItemDisplayContext::Ground,
        ] {
            assert!(light("minecraft:stone", context));
            assert!(light("minecraft:chest", context));
            assert!(!light("minecraft:stick", context));
            assert!(!light("minecraft:diamond", context));
        }
        assert!(!light("minecraft:air", mod_api::ItemDisplayContext::Gui));
    });
}
