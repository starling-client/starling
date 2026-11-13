//! The `tooltip_component_draw` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0077

pub trait Hooks {
    /// Render the measured component at origin, inside the tooltip background.
    /// Stack is the component's data. Use native_sprite/native_fill/native_text;
    /// size is the measurement returned by native_tooltip_component this frame.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "RegisterClientTooltipComponentFactoriesEvent"
    )]
    fn tooltip_component_draw();
}
