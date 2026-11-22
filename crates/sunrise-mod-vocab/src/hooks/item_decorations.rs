//! The `item_decorations` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0100

pub trait Hooks {
    /// Append an item's custom decorations after its default bar, cooldown and count.
    /// Coordinates are the icon's top-left in GUI units, as the batch takes them;
    /// no item translation is active.
    /// Runs wherever the shared item-decoration path is used (hotbar, menus, tooltips,
    /// and the other screens that draw item icons: advancements, beacon, stats, trading).
    /// The NeoForge counterpart is `IItemDecorator.render`; registration through
    /// `RegisterItemDecorationsEvent` is a separate moment.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_item_decorations",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemDecorator.render"
    )]
    fn item_decorations(stack: &ItemStack, x: f32, y: f32, draw: &mut dyn DrawContext);
}
