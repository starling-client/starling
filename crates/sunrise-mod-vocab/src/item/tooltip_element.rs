use crate::{Text, TooltipComponent};

/// One element of an item's tooltip as `render_tooltip_gather_components`
/// hands it over, before any wrapping or component preparation
/// (`Either<FormattedText, TooltipComponent>`). Text deeper than 512 nodes
/// is skipped during preparation. Built-in bundle components share this list
/// with text lines and custom components from `ItemPart::tooltip_image` or
/// gather listeners. Gather may add, reorder or remove them; each element is
/// then prepared into a [`crate::PreparedTooltipComponent`] before
/// `render_tooltip_pre` runs.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
// no wire: carries in-process custom tooltip component data.
pub enum TooltipElement {
    /// A line (`FormattedText`) with its colour as `rgba` packs it
    /// (`u32::MAX` keeps the text's own).
    Text(Text, u32),
    /// A tooltip component (`TooltipComponent`), built-in or custom.
    Component(TooltipComponent),
}

/// Where a tooltip hangs from, in GUI pixels: what `render_tooltip_pre` lets a mod move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods change both fields
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TooltipPlace {
    /// The point's x, before the positioner's offset and clamp; set it to move the tooltip.
    pub x: i32,
    /// The point's y, before the positioner's offset and clamp; set it to move the tooltip.
    pub y: i32,
}
