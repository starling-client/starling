//! The `render_tooltip_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `RenderTooltipEvent.Pre`: a tooltip whose components have been prepared and are
    /// about to be measured, positioned and drawn. `stack` is the item the tooltip
    /// describes, or `None` for a non-item tooltip. `components` are the read-only
    /// prepared components (`PreparedTooltipComponent`, in display order, built-in and
    /// custom alike) as `render_tooltip_gather_components` and component preparation
    /// left them; measure them with a font borrowed via `draw.with_tooltip_font(font, ...)`
    /// or inspect them with `as_text`, `as_bundle`, and `downcast_ref`. `place` holds the
    /// anchor point in GUI pixels before `positioner` adds its offset and keeps the box on
    /// screen; cursor anchors truncate to whole GUI pixels, and a widget positioner may
    /// ignore the anchor. `screen` is `[screen_width, screen_height]` in GUI pixels.
    /// `positioner` is the active `ClientTooltipPositioner` that places the
    /// tooltip box once measured. Text lines are wrapped only where
    /// `render_tooltip_gather_components` ran: with no Gather listener, each line arrives
    /// whole, as vanilla passes it. A stack that can't be converted skips Gather and Pre,
    /// and the engine's prepared tooltip is drawn as it is. `font` is
    /// the font identifier used for measurement and drawing (`minecraft:default`, or the
    /// item font, queried again before Pre); change it to measure and draw with a
    /// replacement font without rewrapping the lines. A prepared component's `width` and `height` return `None` on failure,
    /// distinct from a valid size of zero. A failed component measurement or draw
    /// aborts the whole tooltip. `draw` is the tooltip's GUI drawing context; paint
    /// from Pre stays visible on cancellation or failure. Return `Flow::Cancel` and the
    /// tooltip box and components are not drawn and `render_tooltip_texture` is not asked.
    /// Asked each frame a tooltip shows, only while a mod listens. An input conversion
    /// refusal skips Pre and leaves the prepared tooltip intact. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        version = 4,
        contract = "cancel",
        out = "cancel",
        neoforge = "RenderTooltipEvent.Pre",
        cancellable = true
    )]
    fn render_tooltip_pre(
        stack: Option<&ItemStack>,
        components: &[mod_api::PreparedTooltipComponent],
        place: &mut TooltipPlace,
        screen: [f32; 2],
        positioner: &dyn mod_api::ClientTooltipPositioner,
        font: &mut Identifier,
        draw: &mut dyn DrawContext,
        cancel: &mut bool,
    );
}
