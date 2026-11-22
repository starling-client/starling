//! Client visual-order text and styled measurements, outside drawing.
#![allow(unused_variables)]
use crate::*;

/// Read-only component resolution and access to the client tooltip font.
#[sunrise_mod_api_macros::host_area]
pub trait ClientTextClientHost {
    /// Side: client.
    /// `Component.getVisualOrderText()`: current-language, styled characters in
    /// visual order, including bidi shaping and mirroring. Available outside
    /// drawing, including client setup. Sink indices are UTF-16 offsets within
    /// the current directional and effective-style run, not within the input
    /// component tree. Independently set equal styles do not restart a run.
    /// Custom click IDs compare by namespace and path for grouping, without
    /// rewriting each character's returned click event.
    /// A resolved color is returned as uppercase `#RRGGBB`.
    /// Action keybinds resolve to their assigned keys as in `text_string`.
    /// In-memory text reads do not enforce the 512-node serialization depth cap
    /// or `ExtraCodecs` profile/property bounds.
    /// No NeoForge hook counterpart is documented.
    #[forward_api_inputs]
    fn text_visual_order(&self, text: &Text) -> Vec<VisualTextCharacter>;

    /// Side: client.
    /// `Font.width(FormattedText)`: the width of a component in unscaled GUI
    /// pixels, using the current language, styles and resource-pack fonts.
    /// Available outside a draw, including client setup and tooltip-component
    /// sizing. Like `ClientTooltipComponent.getWidth(Font)`, this measures with
    /// the rendering client's active font, not a font chosen by the mod.
    /// Build a [`Text`] component directly and lend it to this read.
    /// In-memory text reads do not enforce the 512-node serialization depth cap
    /// or `ExtraCodecs` profile/property bounds.
    #[forward_api_inputs]
    fn text_width(&self, text: &Text) -> i32;

    /// Side: client.
    /// `Font.split(FormattedText, width)`: wrap a component at `width` unscaled
    /// GUI pixels. Lines retain styles, resolve the current language and use
    /// the same active font as tooltip layout. Newlines force breaks; a width
    /// below one follows vanilla's one-pixel minimum, without rejecting input.
    /// Available outside drawing. In-memory text reads do not enforce the
    /// 512-node serialization depth cap or `ExtraCodecs` profile/property bounds.
    #[forward_api_inputs]
    #[forward_api_output]
    fn text_split(&self, text: &Text, width: i32) -> Vec<Text>;
}
