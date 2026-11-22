//! The `held_item_tint` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Tint one third-person held-item submission in the world, for a player, mob or armor
    /// stand. `entity_id` is the holder's current client-level entity id, `hand` is the
    /// logical main or off hand (independent of the dominant arm), `stack` is the rendered
    /// stack, and `partial_tick` is the frame's tick fraction.
    ///
    /// Only Parts attached to the stack's item are asked, once per submitted nonempty stack.
    /// Each Part gets the original draw inputs; only its return value is used.
    /// The first `Some(0xRRGGBB)` multiplies the resolved model's RGB before lighting,
    /// including special models such as a shield's base and patterns. The high byte is ignored;
    /// alpha, foil, outline color and pose are unchanged. `None` leaves the answer to the
    /// next Part, then to the model's own colors. First-person hands, GUI previews, worn
    /// equipment, item frames and dropped items are not asked. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 1,
        contract = "replace",
        out = "tint",
        scope = "item",
        key = "stack",
        event_per_listener = true,
        neoforge = "none (vanilla ItemInHandLayer.submitArmWithItem and PlayerItemInHandLayer.submitArmWithItem)"
    )]
    fn held_item_tint(
        entity_id: u32,
        hand: Hand,
        stack: &ItemStack,
        partial_tick: f32,
        tint: &mut Option<u32>,
    );
}
