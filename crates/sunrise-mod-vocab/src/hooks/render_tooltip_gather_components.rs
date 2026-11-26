//! The `render_tooltip_gather_components` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `RenderTooltipEvent.GatherComponents`: the elements of `stack`'s
    /// tooltip (`ItemStack.getTooltipLines` as `TooltipElement::Text` lines,
    /// plus any `TooltipElement::Component(TooltipComponent)` supplied by the
    /// built-in bundle tooltip or the item's `ItemPart::tooltip_image`), once
    /// `item_tooltip` has run and before any wrapping, factory preparation, or
    /// measuring. Add, change, reorder or remove elements; each element is then
    /// prepared into a `PreparedTooltipComponent` (custom components via
    /// factories registered in `register_client_tooltip_component_factories`)
    /// before `render_tooltip_pre` runs. Gather wraps logical text in the item's
    /// tooltip font when it cannot fit on either side of the cursor. Assign a positive
    /// GUI-pixel width to `event.max_width` to further constrain wrapping (`-1` by
    /// default). Return `Flow::Cancel` and nothing is drawn; neither
    /// `render_tooltip_pre` nor `render_tooltip_texture` is asked.
    /// A panicking destructor of custom data published here disables this listener
    /// for the session without disabling other listeners.
    /// Asked each frame the tooltip shows, never for one the stack hides. An input
    /// conversion refusal skips Gather and preserves the unwrapped engine tooltip.
    /// Text is used in memory without serialized component limits. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "RenderTooltipEvent.GatherComponents",
        cancellable = true,
        version = 4
    )]
    fn render_tooltip_gather_components(
        stack: &mod_api::ItemStack,
        screen_width: i32,
        screen_height: i32,
        elements: &mut Vec<TooltipElement>,
        max_width: &mut i32,
        cancel: &mut bool,
    );
}
