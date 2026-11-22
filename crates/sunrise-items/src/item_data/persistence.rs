//! Persistent stack patches; live and network component views stay separate.
use super::*;
pub(super) fn encode_stack_components(
    stack: &ItemStack,
    include_damage: bool,
    selector: &impl Fn(&str) -> bool,
    template: &impl Fn(&ItemStack) -> Tag,
) -> Option<Tag> {
    let mut components = components_tag_inner(
        stack.damage,
        stack.data(),
        include_damage,
        selector,
        template,
    )?;
    // DataComponentPatch.CODEC filters transient SET/REMOVE entries, including
    // typed nested stack patches carried by persistent text and item codecs.
    sunrise_world::item_defaults::strip_transient_component_map_tag(&mut components);
    if matches!(&components, Tag::Compound(entries) if entries.is_empty()) {
        return None;
    }
    // MobEffectInstance.Details' codec always saves the resolved icon flag.
    // An omitted icon follows show_particles, whose default is true.
    if let Some(Tag::List(effects)) = components
        .get_mut("minecraft:potion_contents")
        .and_then(|potion| potion.get_mut("custom_effects"))
    {
        for effect in effects {
            let shown = effect.get("show_particles").and_then(int).unwrap_or(1) != 0;
            if let Tag::Compound(fields) = effect {
                fields
                    .entry(JavaString::from("show_icon"))
                    .or_insert(Tag::Byte(i8::from(shown)));
            }
        }
    }
    // LodestoneTracker.CODEC: optionalFieldOf("tracked", true).
    if let Some(Tag::Compound(tracker)) = components.get_mut("minecraft:lodestone_tracker")
        && tracker.get(&JavaString::from("tracked")) == Some(&Tag::Byte(1))
    {
        tracker.remove(&JavaString::from("tracked"));
    }
    Some(components)
}
