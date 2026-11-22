//! The `item_tooltip` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `ItemTooltipEvent`: the whole tooltip of `stack` (`ItemStack.getTooltipLines`),
    /// once its name, the item's own lines, the components' lines, the
    /// advanced lines and the operator warning are in `lines`, each with a
    /// colour as `rgba` packs it (`u32::MAX` keeps the text's own). Add,
    /// change, reorder or remove lines. `player` and `player_entity_id` are
    /// the local player's, `stack` borrows the stack lazily (`ItemStackView::project`
    /// copies components on demand), `display` borrows the stack's
    /// `minecraft:tooltip_display`, `advanced` (F3+H) and `creative` (a creative
    /// tab's own slot) are its `TooltipFlag`, and `lines` mutates the live tooltip
    /// directly with per-edit `TextFormError` validation so one invalid edit leaves
    /// all other lines intact. Asked when item tooltip lines are built, for the
    /// game's display or `ItemResource::tooltip`. Never asked for a hidden tooltip,
    /// a crossbow's loaded projectile or a query made during item tooltip line
    /// construction. Queries outside line construction can dispatch this event, but
    /// never gather images or draw. Client-side.
    /// This global event is distinct from [`ItemPart::tooltip`], which runs at
    /// the earlier `Item.appendHoverText` position, before component lines.
    /// Parts' lines therefore come first; this event sees the finished list
    /// including them, for any item, even one with no attached parts.
    /// Replacements are used in memory; they are not subject to serialized component limits.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        contract = "listen",
        neoforge = "ItemTooltipEvent",
        version = 3
    )]
    fn item_tooltip(
        player: u128,
        player_entity_id: i32,
        stack: ItemStackView<'_>,
        display: &TooltipDisplay<'_>,
        advanced: bool,
        creative: bool,
        lines: TooltipLines<'_>,
    );
}
