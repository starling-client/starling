//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
use crate::{ItemStack, Text, TextFormError};
use sunrise_core::nbt::Tag;
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// The `minecraft:tooltip_display` component applied by `ItemStack` while it
/// builds a hover tooltip.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TooltipDisplay<'a> {
    pub(crate) component: Option<&'a Tag>,
    pub(crate) ominous_banner: bool,
}
impl<'a> TooltipDisplay<'a> {
    // hidden: a constructor for generated wire code, not for mods
    /// Reconstruct the read-only projection transported by the mod boundary.
    #[doc(hidden)]
    pub fn __wire_from_fields(component: Option<&'a Tag>, ominous_banner: bool) -> Self {
        Self {
            component,
            ominous_banner,
        }
    }

    /// Read tooltip visibility from an owned API stack's effective component patch.
    /// The game supplies this stack only after the tooltip subscription guard.
    pub fn for_stack(stack: &'a ItemStack) -> Self {
        Self::from_component(
            stack.components().and_then(|components| {
                components
                    .as_nbt()
                    .iter()
                    .find(|(id, _)| {
                        id.0.iter()
                            .copied()
                            .eq("minecraft:tooltip_display".encode_utf16())
                    })
                    .map(|(_, tag)| tag)
            }),
            false,
        )
    }

    /// Borrow a tooltip component without copying it. `ominous_banner` represents
    /// the vanilla banner default when the engine stores that default implicitly.
    pub fn from_component(component: Option<&'a Tag>, ominous_banner: bool) -> Self {
        Self {
            component,
            ominous_banner,
        }
    }

    fn hides_tooltip(&self) -> bool {
        matches!(
            self.component.and_then(|display| display.get("hide_tooltip")),
            Some(Tag::Byte(value)) if *value != 0
        )
    }

    /// `ItemStack.getTooltipLines`: `hide_tooltip` suppresses the name unless
    /// the tooltip flag is creative.
    pub fn name_visible(&self, creative: bool) -> bool {
        creative || !self.hides_tooltip()
    }

    /// `TooltipDisplay.shows`: whether details for a data component are shown.
    pub fn shows(&self, component: &str) -> bool {
        if self.hides_tooltip() {
            return false;
        }
        // `item_data::decode_components` models the raid captain banner's
        // default hidden pattern tooltip as `ominous_banner` instead of
        // retaining its redundant `tooltip_display` component.
        if self.ominous_banner
            && self.component.is_none()
            && component == "minecraft:banner_patterns"
        {
            return false;
        }
        let Some(Tag::List(hidden)) = self
            .component
            .and_then(|display| display.get("hidden_components"))
        else {
            return true;
        };
        !hidden.iter().any(|tag| {
            matches!(tag, Tag::String(id) if id.0.iter().copied().eq(component.encode_utf16()))
        })
    }
}

/// Engine backing for a live item tooltip line list.
// hidden: the engine owns the live tooltip vector; listeners use TooltipLines.
#[doc(hidden)]
pub trait TooltipLinesAccess {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn color(&self, index: usize) -> Option<u32>;
    fn get(&self, index: usize) -> Option<Result<(Text, u32), TextFormError>>;
    fn push(&mut self, text: Text, color: u32) -> Result<(), TextFormError>;
    fn insert(&mut self, index: usize, text: Text, color: u32) -> Result<bool, TextFormError>;
    fn set(&mut self, index: usize, text: Text, color: u32) -> Result<bool, TextFormError>;
    fn set_color(&mut self, index: usize, color: u32) -> bool;
    fn remove(&mut self, index: usize) -> Option<Result<(Text, u32), TextFormError>>;
    fn pop(&mut self) -> Option<Result<(Text, u32), TextFormError>>;
    fn clear(&mut self);
    fn truncate(&mut self, len: usize);
    fn swap(&mut self, a: usize, b: usize) -> bool;
}

/// A live mutable view of an item tooltip's lines.
///
/// Each entry pairs a [`Text`] component with a packed RGBA fallback colour
/// (`u32::MAX` keeps the text's own style). Lines are projected on demand and
/// edits are validated individually so a single invalid [`Text`] leaves all
/// other lines intact.
// exhaustive: only the engine constructs a live tooltip view.
// no wire: borrows the live tooltip line vector.
pub struct TooltipLines<'a> {
    access: &'a mut dyn TooltipLinesAccess,
}

impl<'a> TooltipLines<'a> {
    // hidden: the engine constructs a live tooltip view over its own line vector.
    /// Wrap the engine's live tooltip line storage.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn TooltipLinesAccess) -> Self {
        Self { access }
    }

    /// Number of lines currently in the tooltip.
    pub fn len(&self) -> usize {
        self.access.len()
    }

    /// Whether the tooltip currently has no lines.
    pub fn is_empty(&self) -> bool {
        self.access.is_empty()
    }

    /// Read the packed RGBA colour at `index` without projecting its text.
    /// Returns `None` if `index` is out of bounds.
    pub fn color(&self, index: usize) -> Option<u32> {
        self.access.color(index)
    }

    /// Project and read the line at `index`.
    /// Returns `None` if `index` is out of bounds, or `Some(Err(..))` if that line
    /// cannot be represented in the mod API.
    pub fn get(&self, index: usize) -> Option<Result<(Text, u32), TextFormError>> {
        self.access.get(index)
    }

    /// Iterate over every line in order, projecting each line's text on demand.
    pub fn iter(&self) -> impl Iterator<Item = Result<(Text, u32), TextFormError>> + '_ {
        (0..self.len()).filter_map(|index| self.get(index))
    }

    /// Append a line to the end of the tooltip.
    /// Returns `Err(TextFormError)` without mutating the tooltip if `text` is invalid.
    pub fn push(&mut self, text: Text, color: u32) -> Result<(), TextFormError> {
        self.access.push(text, color)
    }

    /// Insert a line at `index`.
    /// Returns `Ok(false)` without panicking if `index > self.len()`, or
    /// `Err(TextFormError)` without mutating the tooltip if `text` is invalid.
    pub fn insert(&mut self, index: usize, text: Text, color: u32) -> Result<bool, TextFormError> {
        self.access.insert(index, text, color)
    }

    /// Replace the line at `index`.
    /// Returns `Ok(false)` without panicking if `index >= self.len()`, or
    /// `Err(TextFormError)` leaving the existing line intact if `text` is invalid.
    pub fn set(&mut self, index: usize, text: Text, color: u32) -> Result<bool, TextFormError> {
        self.access.set(index, text, color)
    }

    /// Update the packed RGBA colour at `index` without converting its text.
    /// Returns `false` if `index` is out of bounds.
    pub fn set_color(&mut self, index: usize, color: u32) -> bool {
        self.access.set_color(index, color)
    }

    /// Remove and return the projected line at `index`.
    /// Returns `None` if `index` is out of bounds.
    pub fn remove(&mut self, index: usize) -> Option<Result<(Text, u32), TextFormError>> {
        self.access.remove(index)
    }

    /// Remove and return the projected last line, or `None` if empty.
    pub fn pop(&mut self) -> Option<Result<(Text, u32), TextFormError>> {
        self.access.pop()
    }

    /// Remove every line from the tooltip.
    pub fn clear(&mut self) {
        self.access.clear();
    }

    /// Shorten the tooltip to at most `len` lines.
    pub fn truncate(&mut self, len: usize) {
        self.access.truncate(len);
    }

    /// Swap the lines at `a` and `b` without converting their text.
    /// Returns `false` if either index is out of bounds.
    pub fn swap(&mut self, a: usize, b: usize) -> bool {
        self.access.swap(a, b)
    }

    /// Retain only the lines for which `keep` returns `true`, preserving order.
    pub fn retain(&mut self, mut keep: impl FnMut(Result<(Text, u32), TextFormError>) -> bool) {
        let len = self.len();
        let mut write = 0;
        for read in 0..len {
            let Some(line) = self.get(read) else {
                continue;
            };
            if keep(line) {
                if write != read {
                    self.swap(write, read);
                }
                write += 1;
            }
        }
        self.truncate(write);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum FakeLine {
        Valid(Text, u32),
        Corrupt(TextFormError, u32),
    }

    impl FakeLine {
        fn color(&self) -> u32 {
            match self {
                Self::Valid(_, color) | Self::Corrupt(_, color) => *color,
            }
        }

        fn project(&self) -> Result<(Text, u32), TextFormError> {
            match self {
                Self::Valid(text, color) => Ok((text.clone(), *color)),
                Self::Corrupt(err, _) => Err(err.clone()),
            }
        }
    }

    struct FakeTooltipLines(Vec<FakeLine>);

    impl TooltipLinesAccess for FakeTooltipLines {
        fn len(&self) -> usize {
            self.0.len()
        }

        fn color(&self, index: usize) -> Option<u32> {
            self.0.get(index).map(FakeLine::color)
        }

        fn get(&self, index: usize) -> Option<Result<(Text, u32), TextFormError>> {
            self.0.get(index).map(FakeLine::project)
        }

        fn push(&mut self, text: Text, color: u32) -> Result<(), TextFormError> {
            text.validate()?;
            self.0.push(FakeLine::Valid(text, color));
            Ok(())
        }

        fn insert(&mut self, index: usize, text: Text, color: u32) -> Result<bool, TextFormError> {
            if index > self.0.len() {
                return Ok(false);
            }
            text.validate()?;
            self.0.insert(index, FakeLine::Valid(text, color));
            Ok(true)
        }

        fn set(&mut self, index: usize, text: Text, color: u32) -> Result<bool, TextFormError> {
            if index >= self.0.len() {
                return Ok(false);
            }
            text.validate()?;
            self.0[index] = FakeLine::Valid(text, color);
            Ok(true)
        }

        fn set_color(&mut self, index: usize, color: u32) -> bool {
            let Some(slot) = self.0.get_mut(index) else {
                return false;
            };
            match slot {
                FakeLine::Valid(_, c) | FakeLine::Corrupt(_, c) => *c = color,
            }
            true
        }

        fn remove(&mut self, index: usize) -> Option<Result<(Text, u32), TextFormError>> {
            if index >= self.0.len() {
                return None;
            }
            Some(self.0.remove(index).project())
        }

        fn pop(&mut self) -> Option<Result<(Text, u32), TextFormError>> {
            self.0.pop().map(|entry| entry.project())
        }

        fn clear(&mut self) {
            self.0.clear();
        }

        fn truncate(&mut self, len: usize) {
            self.0.truncate(len);
        }

        fn swap(&mut self, a: usize, b: usize) -> bool {
            if a >= self.0.len() || b >= self.0.len() {
                return false;
            }
            self.0.swap(a, b);
            true
        }
    }

    fn too_deep_text() -> Text {
        let mut text = Text::literal("leaf");
        for _ in 0..crate::text::MAX_TEXT_DEPTH {
            text = Text::literal("parent").append(text);
        }
        text
    }

    #[test]
    fn tooltip_lines_reads_edits_and_oob_never_panic() {
        let mut backing = FakeTooltipLines(Vec::new());
        let mut lines = TooltipLines::new(&mut backing);
        assert!(lines.is_empty());
        assert_eq!(lines.len(), 0);
        assert_eq!(lines.color(0), None);
        assert_eq!(lines.get(0), None);
        assert_eq!(lines.pop(), None);
        assert_eq!(lines.remove(0), None);
        assert!(!lines.set_color(0, 0x11223344));
        assert!(!lines.swap(0, 0));
        assert_eq!(lines.insert(1, Text::literal("oob"), u32::MAX), Ok(false));
        assert_eq!(lines.set(0, Text::literal("oob"), u32::MAX), Ok(false));

        lines.push(Text::literal("first"), 0xAA000001).unwrap();
        lines.push(Text::literal("third"), 0xAA000003).unwrap();
        assert_eq!(
            lines.insert(1, Text::literal("second"), 0xAA000002),
            Ok(true)
        );
        assert_eq!(
            lines.insert(3, Text::literal("fourth"), 0xAA000004),
            Ok(true)
        );
        assert_eq!(lines.insert(5, Text::literal("oob"), u32::MAX), Ok(false));
        assert!(!lines.is_empty());
        assert_eq!(lines.len(), 4);
        assert_eq!(lines.color(1), Some(0xAA000002));
        assert!(lines.set_color(1, 0xBB000002));
        assert!(!lines.set_color(4, 0xBB000002));
        assert_eq!(lines.color(1), Some(0xBB000002));
        assert_eq!(lines.set(2, Text::literal("THIRD"), 0xCC000003), Ok(true));
        assert_eq!(lines.set(4, Text::literal("oob"), u32::MAX), Ok(false));
        assert!(lines.swap(0, 2));
        assert!(!lines.swap(0, 4));
        assert!(!lines.swap(4, 0));

        let snapshot: Vec<_> = lines.iter().collect();
        assert_eq!(
            snapshot,
            vec![
                Ok((Text::literal("THIRD"), 0xCC000003)),
                Ok((Text::literal("second"), 0xBB000002)),
                Ok((Text::literal("first"), 0xAA000001)),
                Ok((Text::literal("fourth"), 0xAA000004)),
            ]
        );
        assert_eq!(
            lines.remove(1),
            Some(Ok((Text::literal("second"), 0xBB000002)))
        );
        assert_eq!(lines.remove(3), None);
        assert_eq!(lines.pop(), Some(Ok((Text::literal("fourth"), 0xAA000004))));
        lines.truncate(10);
        assert_eq!(lines.len(), 2);
        lines.truncate(1);
        assert_eq!(lines.len(), 1);
        lines.clear();
        assert!(lines.is_empty());
    }

    #[test]
    fn tooltip_lines_invalid_edit_preserves_existing_lines() {
        let mut backing = FakeTooltipLines(vec![
            FakeLine::Valid(Text::literal("name"), 0xFFFFFFFF),
            FakeLine::Valid(Text::literal("kept"), 0xFF00FF00),
        ]);
        let mut lines = TooltipLines::new(&mut backing);
        let invalid = too_deep_text();

        assert_eq!(
            lines.push(invalid.clone(), u32::MAX),
            Err(TextFormError::TooDeep)
        );
        assert_eq!(
            lines.insert(1, invalid.clone(), u32::MAX),
            Err(TextFormError::TooDeep)
        );
        assert_eq!(
            lines.set(0, invalid.clone(), u32::MAX),
            Err(TextFormError::TooDeep)
        );
        // Out-of-bounds indices return Ok(false) before text validation.
        assert_eq!(lines.insert(9, invalid.clone(), u32::MAX), Ok(false));
        assert_eq!(lines.set(9, invalid, u32::MAX), Ok(false));

        assert_eq!(
            lines.iter().collect::<Vec<_>>(),
            vec![
                Ok((Text::literal("name"), 0xFFFFFFFF)),
                Ok((Text::literal("kept"), 0xFF00FF00)),
            ]
        );
    }

    #[test]
    fn tooltip_lines_retain_preserves_order_and_error_entries() {
        let mut backing = FakeTooltipLines(vec![
            FakeLine::Valid(Text::literal("a"), 1),
            FakeLine::Valid(Text::literal("drop1"), 2),
            FakeLine::Corrupt(TextFormError::TooDeep, 3),
            FakeLine::Valid(Text::literal("drop2"), 4),
            FakeLine::Valid(Text::literal("b"), 5),
        ]);
        let mut lines = TooltipLines::new(&mut backing);
        lines.retain(|entry| match entry {
            Ok((text, _)) => text == Text::literal("a") || text == Text::literal("b"),
            Err(_) => true,
        });
        assert_eq!(
            lines.iter().collect::<Vec<_>>(),
            vec![
                Ok((Text::literal("a"), 1)),
                Err(TextFormError::TooDeep),
                Ok((Text::literal("b"), 5)),
            ]
        );
        assert_eq!(lines.color(1), Some(3));
        assert!(lines.set_color(1, 33));
        assert_eq!(lines.color(1), Some(33));

        // Now drop the Err element while keeping valid lines in order.
        lines.retain(|entry| entry.is_ok());
        assert_eq!(
            lines.iter().collect::<Vec<_>>(),
            vec![Ok((Text::literal("a"), 1)), Ok((Text::literal("b"), 5))]
        );
    }

    #[test]
    fn owned_stack_tooltip_visibility_uses_its_patch() {
        let mut stack = ItemStack::of(crate::ItemKey(0));
        assert!(TooltipDisplay::for_stack(&stack).name_visible(false));
        stack
            .set_components(
                [(
                    "minecraft:tooltip_display".into(),
                    Tag::compound(vec![("hide_tooltip", Tag::Byte(1))]),
                )]
                .into(),
            )
            .unwrap();
        let display = TooltipDisplay::for_stack(&stack);
        assert!(!display.name_visible(false));
        assert!(display.name_visible(true));
        assert!(!display.shows("minecraft:lore"));
        stack
            .set_components(
                [(
                    "minecraft:tooltip_display".into(),
                    Tag::compound(vec![(
                        "hidden_components",
                        Tag::List(vec![Tag::String("minecraft:lore".into())]),
                    )]),
                )]
                .into(),
            )
            .unwrap();
        let display = TooltipDisplay::for_stack(&stack);
        assert!(display.name_visible(false));
        assert!(!display.shows("minecraft:lore"));
        assert!(display.shows("minecraft:attribute_modifiers"));
    }
}
