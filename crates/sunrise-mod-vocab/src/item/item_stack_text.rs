use super::{ItemStack, Tag, component_edits::ComponentEdit};
use crate::{
    Host, Text,
    text::{TextContent, TextFormError, TextHover, TextObject},
};
use std::borrow::Cow;

const CUSTOM_NAME: &str = "minecraft:custom_name";
const ITEM_NAME: &str = "minecraft:item_name";
const LORE: &str = "minecraft:lore";
// ItemLore.MAX_LINES, used by its persistent and stream codecs.
const MAX_LORE_LINES: usize = 256;

impl ItemStack {
    /// Read the effective `minecraft:custom_name` component, using the prototype
    /// if the patch neither sets nor removes it. Does not apply tooltip styling
    /// or the written-book title fallback. An invalid present value is refused.
    pub fn custom_name(&self, host: &dyn Host) -> Result<Option<Text>, TextFormError> {
        self.read_name(host, CUSTOM_NAME)
    }

    /// Set `minecraft:custom_name` without adding style. `None` clears the name,
    /// including an inherited default. A conversion refusal leaves the stack unchanged.
    pub fn set_custom_name(
        &mut self,
        host: &dyn Host,
        name: Option<&Text>,
    ) -> Result<(), TextFormError> {
        self.set_name(host, CUSTOM_NAME, name)
    }

    /// Read the effective `minecraft:item_name` component, using the prototype
    /// if the patch neither sets nor removes it. This is not the computed name
    /// from `host.hover_name`. An invalid present value is refused.
    pub fn item_name(&self, host: &dyn Host) -> Result<Option<Text>, TextFormError> {
        self.read_name(host, ITEM_NAME)
    }

    /// Set `minecraft:item_name` without adding style. `None` clears the name,
    /// including an inherited default. A conversion refusal leaves the stack unchanged.
    pub fn set_item_name(
        &mut self,
        host: &dyn Host,
        name: Option<&Text>,
    ) -> Result<(), TextFormError> {
        self.set_name(host, ITEM_NAME, name)
    }

    /// Read the effective `minecraft:lore` lines, using the prototype if the patch
    /// neither sets nor removes it. An absent component gives an empty vector.
    /// The lines carry their stored style, not the tooltip's added style. An invalid
    /// list or line is refused rather than returning a partial list.
    pub fn lore(&self, host: &dyn Host) -> Result<Vec<Text>, TextFormError> {
        let Some(tag) = self.text_component(host, LORE) else {
            return Ok(Vec::new());
        };
        let lines = match tag.as_ref() {
            Tag::List(lines) if lines.len() <= MAX_LORE_LINES => lines,
            // NbtOps.getStream also accepts typed arrays. Only empty ones can be lore.
            Tag::ByteArray(values) if values.is_empty() => return Ok(Vec::new()),
            Tag::IntArray(values) if values.is_empty() => return Ok(Vec::new()),
            Tag::LongArray(values) if values.is_empty() => return Ok(Vec::new()),
            _ => return Err(TextFormError::Malformed),
        };
        lines
            .iter()
            .map(|line| host.text_from_nbt(line)?.ok_or(TextFormError::Malformed))
            .collect()
    }

    /// Set `minecraft:lore` without adding style. An empty slice clears any
    /// inherited lore. More than 256 lines (`ItemLore.MAX_LINES`) or a conversion
    /// refusal leaves the stack unchanged.
    pub fn set_lore(&mut self, host: &dyn Host, lines: &[Text]) -> Result<(), TextFormError> {
        if lines.len() > MAX_LORE_LINES {
            return Err(TextFormError::Malformed);
        }
        let value = if lines.is_empty() {
            None
        } else {
            Some(Tag::List(
                lines
                    .iter()
                    .map(|line| host.text_to_nbt(line))
                    .collect::<Result<_, _>>()?,
            ))
        };
        self.write_text_component(host, LORE, value);
        Ok(())
    }

    fn text_component(&self, host: &dyn Host, id: &str) -> Option<Cow<'_, Tag>> {
        match self.component_override(id) {
            ComponentEdit::Value(value) => Some(Cow::Borrowed(value)),
            ComponentEdit::Remove => None,
            ComponentEdit::Inherit => host.item_default_component(self.item(), id).map(Cow::Owned),
        }
    }

    fn read_name(&self, host: &dyn Host, id: &str) -> Result<Option<Text>, TextFormError> {
        self.text_component(host, id)
            .map(|tag| host.text_from_nbt(&tag)?.ok_or(TextFormError::Malformed))
            .transpose()
    }

    fn set_name(
        &mut self,
        host: &dyn Host,
        id: &str,
        name: Option<&Text>,
    ) -> Result<(), TextFormError> {
        let value = name.map(|name| host.text_to_nbt(name)).transpose()?;
        self.write_text_component(host, id, value);
        Ok(())
    }

    fn write_text_component(&mut self, host: &dyn Host, id: &str, value: Option<Tag>) {
        let default = host.item_default_component(self.item(), id);
        match value {
            Some(value) => {
                // PatchedDataComponentMap.set compares component values, not the tag
                // forms they were read from (e.g. a string vs a literal compound).
                let inherits = default
                    .as_ref()
                    .is_some_and(|default| match (&value, default) {
                        (Tag::List(lines), Tag::List(defaults)) if id == LORE => {
                            lines.len() == defaults.len()
                                && lines
                                    .iter()
                                    .zip(defaults)
                                    .all(|(a, b)| same_text_value(host, a, b))
                        }
                        (_, _) if id == LORE => false,
                        _ => same_text_value(host, &value, default),
                    });
                if inherits {
                    self.remove_component_value(id, false);
                } else {
                    self.put_component_value(id, value);
                }
            }
            // PatchedDataComponentMap.remove records a removal only for prototype entries.
            None => self.remove_component_value(id, default.is_some()),
        }
    }
}

fn same_text_value(host: &dyn Host, a: &Tag, b: &Tag) -> bool {
    let (Ok(Some(mut a)), Ok(Some(mut b))) = (host.text_from_nbt(a), host.text_from_nbt(b)) else {
        return false;
    };
    // TextColor.equals compares RGB, while the codec preserves the spelling.
    // Only adjust these temporary decoded values used for the comparison.
    let mut pending = vec![&mut a, &mut b];
    while let Some(text) = pending.pop() {
        if let Some(color) = &mut text.style.color
            && let Some(rgb) = sunrise_core::text::named_color(color)
        {
            *color = format!("#{rgb:06X}");
        }
        pending.extend(&mut text.children);
        match &mut text.content {
            TextContent::Translatable { arguments, .. } => pending.extend(arguments),
            TextContent::Selector { separator, .. } | TextContent::Nbt { separator, .. } => {
                pending.extend(separator.as_deref_mut());
            }
            TextContent::Object(
                TextObject::Player { fallback, .. } | TextObject::Atlas { fallback, .. },
            ) => pending.extend(fallback.as_deref_mut()),
            _ => {}
        }
        match &mut text.style.hover {
            Some(TextHover::Text(child)) => pending.push(child),
            Some(TextHover::Entity {
                name: Some(child), ..
            }) => pending.push(child),
            _ => {}
        }
    }
    sunrise_core::nbt::with_tree_stack(|| a == b)
}
