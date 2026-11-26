//! Persistent stack patches; live and network component views stay separate.
use super::*;
pub(super) fn encode_persistent_components(
    stack: &ItemStack,
    include_damage: bool,
    selector: &impl Fn(&str) -> bool,
) -> Option<Tag> {
    let mut components = components_tag_inner(
        stack.damage,
        stack.data(),
        include_damage,
        false,
        selector,
        &raw_template_tag,
    )?;
    // DataComponentPatch.CODEC filters transient SET/REMOVE entries, including
    // typed nested stack patches carried by persistent text and item codecs.
    sunrise_world::item_defaults::strip_transient_component_map_tag(&mut components);
    if matches!(&components, Tag::Compound(entries) if entries.is_empty()) {
        return None;
    }
    canonicalize_component_patch(&mut components);
    Some(components)
}
