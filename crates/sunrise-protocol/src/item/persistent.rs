//! Current pinned component codecs with explicit registry callbacks.
//! Canonical values retain stream-only state; persistent encoding is a read-only projection.
use super::{Component, Names, Stack, effective_wire_patch};
use crate::{Error, Result, buf::Reader};
use sunrise_core::nbt::Tag;
use sunrise_world::component_value::{Patch, Template as ValueTemplate, Value};

mod source;
pub use source::{
    component_from_admitted_tag, component_from_tag, component_from_value, template_from_parts,
};

/// The output direction only needs names to ids, not a bidirectional registry.
pub trait Ids {
    fn id(&self, registry: &str, name: &str) -> Option<i32>;
}
impl<F: Fn(&str, &str) -> Option<i32>> Ids for F {
    fn id(&self, registry: &str, name: &str) -> Option<i32> {
        self(registry, name)
    }
}
/// Adapter for a name-only lookup callback.
#[non_exhaustive]
pub struct NameLookup<'a>(
    pub &'a dyn Fn(&str, i32) -> Option<String>,
    Option<&'a crate::registry::Dynamic>,
);
impl<'a> NameLookup<'a> {
    pub fn new(lookup: &'a dyn Fn(&str, i32) -> Option<String>) -> Self {
        Self(lookup, None)
    }
    pub fn with_registries(mut self, registries: &'a crate::registry::Dynamic) -> Self {
        self.1 = Some(registries);
        self
    }
}
impl Names for NameLookup<'_> {
    fn consume_effect_kind(&self, id: i32) -> Option<super::consume_effects::Kind> {
        match self.1 {
            Some(registries) => registries.consume_effect_kind(id),
            None => super::consume_effects::builtin_name(id)
                .map(|_| super::consume_effects::Kind::Builtin(id)),
        }
    }

    fn name(&self, registry: &str, id: i32) -> Option<String> {
        (self.0)(registry, id)
    }
}

pub use sunrise_world::component_value::types::{
    BUILTIN_COMPONENT_COUNT, builtin_component_id, builtin_component_name,
};
/// Trusted canonical values may also refer to an installed native component.
pub fn component_name(kind: i32) -> Option<&'static str> {
    builtin_component_name(kind).or_else(|| super::custom::name(kind))
}
pub fn component_id(name: &str) -> Option<i32> {
    builtin_component_id(name).or_else(|| super::custom::id(name))
}

/// Decode the effective own patch, without prototype merging or persistence conversion.
pub fn template_value(stack: &Stack, names: &dyn Names) -> Result<ValueTemplate> {
    if stack.count == 0 || stack.item <= 0 {
        return Err(Error::new(
            "item template must have a non-air item and nonzero count",
        ));
    }
    let item = names
        .name("minecraft:item", stack.item)
        .ok_or_else(|| Error::new("item template registry id has no name"))?;
    let item = match sunrise_world::component_value::source::normalized_identifier(&item)
        .map_err(Error::new)?
    {
        std::borrow::Cow::Borrowed(_) => item,
        std::borrow::Cow::Owned(normalized) => normalized,
    };
    if item == "minecraft:air" {
        return Err(Error::new("air is not a template item"));
    }
    let mut patch: Patch = Vec::with_capacity(stack.added.len() + stack.removed.len());
    let (sets, removals) = effective_wire_patch(&stack.added, &stack.removed);
    for (kind, value) in sets {
        let _nested = super::Nested::enter()?;
        let name = component_name(*kind)
            .ok_or_else(|| Error::new(format!("unknown pinned component type {kind}")))?;
        patch.push((
            name.to_owned(),
            Some(component_value_named(*kind, name, value, names)?),
        ));
    }
    for kind in removals {
        let name = component_name(kind)
            .ok_or_else(|| Error::new(format!("unknown removed pinned component type {kind}")))?;
        patch.push((name.to_owned(), None));
    }
    patch.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(ValueTemplate::new(item, stack.count, patch))
}
/// The actual canonical value, not an opaque stream cache or a component hash.
pub fn component_value(kind: i32, value: &Component, names: &dyn Names) -> Result<Value> {
    let name = component_name(kind)
        .ok_or_else(|| Error::new(format!("unknown pinned component type {kind}")))?;
    component_value_named(kind, name, value, names)
}
pub(super) fn component_value_named(
    kind: i32,
    name: &str,
    value: &Component,
    names: &dyn Names,
) -> Result<Value> {
    value.code_with::<super::hash::ops::Values>(kind, names)
        .ok_or_else(|| Error::new(format!("cannot decode complete value of component {name}: invalid payload or missing registry name")))
}
/// `None` is a deliberately transient component; errors do not modify the value.
pub fn component_to_tag(kind: i32, value: &Component, names: &dyn Names) -> Result<Option<Tag>> {
    let name = component_name(kind)
        .ok_or_else(|| Error::new(format!("unknown pinned component type {kind}")))?;
    if !sunrise_world::component_value::is_persistent(name) {
        return Ok(None);
    }
    let value = component_value_named(kind, name, value, names)?;
    let projected = value.to_component_tag_with(name, is_native_component);
    projected.map(Some).map_err(Error::new)
}

pub(super) fn is_native_component(name: &str) -> bool {
    super::custom::id(name).is_some()
}

pub(crate) fn special_value(
    kind: i32,
    value: &Component,
    names: &dyn Names,
) -> Option<Option<Value>> {
    let result = if matches!(kind, 20 | 42 | 50) {
        Some(transient_value(kind, value))
    } else if let Component::Raw(bytes) = value {
        // Typed components may also arrive from a caller as pinned raw values.
        // Interpret them through the existing typed stream codec, not another parser.
        if matches!(kind, 0..=4 | 6 | 9..=13 | 17..=19 | 21 | 30 | 45 | 47 | 48 | 51..=53 | 55..=57 | 65 | 69..=72 | 75)
            || super::custom::contains(kind)
        {
            Some((|| {
                let mut reader = Reader::new(bytes).with_names(names);
                let decoded = Component::read(kind, &mut reader)?;
                if reader.remaining() != 0 {
                    return Err(Error::new("trailing component bytes"));
                }
                component_value(kind, &decoded, names)
            })())
        } else {
            None
        }
    } else {
        None
    };
    result.map(Result::ok)
}
fn transient_value(kind: i32, value: &Component) -> Result<Value> {
    match (kind, value) {
        (20, Component::Unit) => Ok(Value::Map(Vec::new())),
        (42 | 50, Component::Int(value)) => Ok(Value::Int(*value)),
        (20, Component::Raw(bytes)) if bytes.is_empty() => Ok(Value::Map(Vec::new())),
        (42 | 50, Component::Raw(bytes)) => {
            let mut reader = Reader::new(bytes);
            let value = reader.varint()?;
            if reader.remaining() != 0 {
                return Err(Error::new("trailing transient component bytes"));
            }
            Ok(Value::Int(value))
        }
        _ => Err(Error::new("wrong transient component value")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{buf::Codec, item::Template};
    use sunrise_core::vanilla_bytes::Writer;

    struct Registry;
    impl Names for Registry {
        fn name(&self, registry: &str, id: i32) -> Option<String> {
            match (registry, id) {
                ("minecraft:item", 1) => Some("minecraft:stone".into()),
                ("minecraft:entity_type", 0) => Some("minecraft:cow".into()),
                ("minecraft:sound_event", 0) => Some("minecraft:item.armor.equip_generic".into()),
                ("minecraft:sound_event", 1) => Some("minecraft:item.shears.snip".into()),
                _ => None,
            }
        }
    }
    fn registry_id(registry: &str, name: &str) -> Option<i32> {
        match (registry, name) {
            ("minecraft:item", "minecraft:stone") => Some(1),
            ("minecraft:entity_type", "minecraft:cow") => Some(0),
            ("minecraft:sound_event", "minecraft:item.armor.equip_generic") => Some(0),
            ("minecraft:sound_event", "minecraft:item.shears.snip") => Some(1),
            _ => None,
        }
    }
    #[test]
    fn full_equippable_and_unit_survive_parent_patch_change() {
        let mut wire = Writer::new();
        wire.varint(3); // chest
        wire.varint(1); // reference equip sound 0
        wire.bool(true);
        wire.string("minecraft:equipment/test");
        wire.bool(true);
        wire.string("minecraft:overlay/test");
        wire.bool(true);
        wire.varint(2);
        wire.varint(0); // one allowed entity
        for flag in [false, false, false, true, true] {
            wire.bool(flag);
        }
        wire.varint(2); // reference shearing sound 1
        let mut stack = Stack::new(1, 1);
        stack.added.push((32, Component::Raw(wire.data)));
        stack.added.push((34, Component::Raw(Vec::new())));
        let original = template_value(&stack, &Registry).unwrap();
        let mut mutated = original.clone();
        mutated
            .patch
            .push(("minecraft:damage".into(), Some(Value::Int(9))));
        let rebuilt =
            template_from_parts(&mutated.item, mutated.count, &mutated.patch, &registry_id)
                .unwrap();
        let rebuilt = template_value(&rebuilt, &Registry).unwrap();
        for id in ["minecraft:equippable", "minecraft:glider"] {
            assert_eq!(
                original.patch.iter().find(|(key, _)| key == id),
                rebuilt.patch.iter().find(|(key, _)| key == id)
            );
        }
    }
    #[test]
    fn inline_jukebox_song_survives_wire_but_refuses_source() {
        let mut wire = Writer::new();
        wire.varint(0); // direct song
        wire.varint(0);
        wire.string("minecraft:test.song");
        wire.bool(false); // direct sound
        wire.nbt(&Tag::String("song".into()));
        wire.f32(1.5);
        wire.varint(7);
        let value = component_value(66, &Component::Raw(wire.data), &Registry).unwrap();
        assert!(matches!(&value, Value::StreamOnly(_)));
        assert!(value.to_tag().is_err());
        let rebuilt = component_from_value(66, &value, &registry_id).unwrap();
        assert_eq!(component_value(66, &rebuilt, &Registry).unwrap(), value);
        let reference = component_value(
            66,
            &Component::Raw(vec![1]),
            &NameLookup::new(&|registry, id| {
                (registry == "minecraft:jukebox_song" && id == 0)
                    .then(|| "minecraft:test_song".into())
            }),
        )
        .unwrap();
        assert_ne!(reference, value);
    }
    #[test]
    fn transient_components_are_wire_values_not_persistent_entries() {
        let template = ValueTemplate::new(
            "minecraft:stone".into(),
            1,
            vec![
                (
                    "minecraft:creative_slot_lock".into(),
                    Some(Value::Map(Vec::new())),
                ),
                (
                    "minecraft:additional_trade_cost".into(),
                    Some(Value::Int(4)),
                ),
                ("minecraft:map_post_processing".into(), Some(Value::Int(1))),
            ],
        );
        assert!(template.to_tag().unwrap().get("components").is_none());
        let wire = template_from_parts(
            &template.item,
            template.count,
            &template.patch,
            &registry_id,
        )
        .unwrap();
        assert_eq!(template_value(&wire, &Registry).unwrap(), template);
    }
    #[test]
    fn java_float_identity_keeps_negative_zero_in_default_records() {
        let reduction = |base: f32| {
            let mut wire = Writer::new();
            wire.f32(0.0);
            wire.f32(1.0);
            wire.varint(1);
            wire.f32(90.0);
            wire.bool(false);
            wire.f32(base);
            wire.f32(1.0);
            wire.f32(1.0);
            wire.f32(0.0);
            wire.f32(1.0);
            wire.bool(false);
            wire.bool(false);
            wire.bool(false);
            component_value(37, &Component::Raw(wire.data), &Registry).unwrap()
        };
        assert_ne!(reduction(0.0), reduction(-0.0));
        let nan = |bits: u32| {
            let mut wire = Writer::new();
            wire.f32(f32::from_bits(bits));
            component_value(7, &Component::Raw(wire.data), &Registry).unwrap()
        };
        assert_eq!(nan(0x7fc00001), nan(0x7fc00002));
    }
    #[test]
    fn template_stream_refuses_air_and_zero_but_keeps_negative_count() {
        for (item, count) in [
            (0, 1),
            (1, 0),
            (1, -7),
            (1, 127),
            (1, i32::MIN),
            (1, i32::MAX),
        ] {
            let mut bytes = Writer::new();
            bytes.varint(item);
            bytes.varint(count);
            bytes.varint(0);
            bytes.varint(0);
            let value = Template::read(&mut Reader::new(&bytes.data));
            if item > 0 && count != 0 {
                let stack = value.unwrap();
                assert_eq!(stack.count, count);
                let canonical = template_value(&stack, &Registry).unwrap();
                assert!(canonical.to_tag().is_err());
                assert_eq!(
                    template_from_parts(
                        &canonical.item,
                        canonical.count,
                        &canonical.patch,
                        &registry_id
                    )
                    .unwrap(),
                    stack
                );
            } else {
                assert!(value.is_err());
            }
        }
    }
    #[test]
    fn literal_text_wrappers_share_source_and_stream_identity() {
        let literal = Tag::String("literal".into());
        let compound = Tag::compound([("text", literal.clone())]);
        let singleton = Tag::List(vec![compound.clone()]);
        let expected = component_value(6, &Component::Text(literal), &Registry).unwrap();
        for tag in [compound, singleton] {
            assert_eq!(
                component_value(6, &Component::Text(tag.clone()), &Registry).unwrap(),
                expected
            );
            assert_eq!(
                sunrise_world::component_value::from_component_tag("minecraft:custom_name", &tag)
                    .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn negative_holder_ids_refuse_without_arithmetic_overflow() {
        for id in [-1, i32::MIN] {
            let mut wire = Writer::new();
            wire.varint(10);
            wire.varint(0);
            wire.bool(false);
            wire.bool(false);
            wire.bool(false);
            wire.f32(0.0);
            wire.f32(1.0);
            wire.bool(true);
            wire.varint(id);
            wire.bool(false);
            assert!(component_value(39, &Component::Raw(wire.data), &Registry).is_err());
            for kind in [27, 58, 66, 109] {
                let mut wire = Writer::new();
                wire.varint(id);
                assert!(component_value(kind, &Component::Raw(wire.data), &Registry).is_err());
            }
        }
    }
    fn bundle_of(template: ValueTemplate) -> Value {
        Value::List(vec![Value::Template(Box::new(template))])
    }
    fn stone(patch: Patch) -> ValueTemplate {
        ValueTemplate::new("minecraft:stone".into(), 1, patch)
    }
    #[test]
    fn a_component_named_twice_keeps_its_last_entry_in_every_projection() {
        let damage = || "minecraft:damage".to_owned();
        let set_last = vec![(damage(), None), (damage(), Some(Value::Int(9)))];
        let removed_last = vec![(damage(), Some(Value::Int(9))), (damage(), None)];
        for (patch, kept, key, absent) in [
            (
                set_last,
                Some(Value::Int(9)),
                "minecraft:damage",
                "!minecraft:damage",
            ),
            (removed_last, None, "!minecraft:damage", "minecraft:damage"),
        ] {
            let stack = template_from_parts("minecraft:stone", 1, &patch, &registry_id).unwrap();
            assert_eq!(
                template_value(&stack, &Registry).unwrap().patch,
                vec![(damage(), kept)]
            );
            // The same patch nested in a bundle takes the nested-writer path.
            let nested = component_from_value(51, &bundle_of(stone(patch.clone())), &registry_id);
            let Component::Stacks(inner) = nested.unwrap() else {
                panic!("bundle contents are stacks");
            };
            assert_eq!(inner[0].added, stack.added);
            assert_eq!(inner[0].removed, stack.removed);
            let tag = stone(patch).to_tag().unwrap();
            let components = tag.get("components").unwrap();
            assert!(components.get(key).is_some() && components.get(absent).is_none());
        }
    }
    #[test]
    fn component_chains_up_to_the_readers_depth_convert_on_a_network_stack() {
        let convert = |levels: usize, name: &'static str, wrap: fn(ValueTemplate) -> Value| {
            std::thread::Builder::new()
                .stack_size(2 << 20)
                .spawn(move || {
                    let mut top = stone(vec![]);
                    for _ in 0..levels {
                        top = stone(vec![(name.into(), Some(wrap(top)))]);
                    }
                    let stack =
                        template_from_parts(&top.item, top.count, &top.patch, &registry_id)?;
                    Ok::<_, Error>(template_value(&stack, &Registry).unwrap() == top)
                })
                .unwrap()
                .join()
                .unwrap()
        };
        let max = crate::item::MAX_NESTING as usize;
        for (name, wrap) in [
            (
                "minecraft:bundle_contents",
                bundle_of as fn(ValueTemplate) -> Value,
            ),
            ("minecraft:use_remainder", |t| Value::Template(Box::new(t))),
        ] {
            assert!(convert(max, name, wrap).unwrap(), "{name}");
            assert!(convert(max + 1, name, wrap).is_err(), "{name}");
        }
    }
    #[test]
    fn persistent_projection_refuses_what_the_saved_codec_refuses() {
        assert!(component_to_tag(3, &Component::Int(-1), &Registry).is_err());
        assert_eq!(
            component_to_tag(3, &Component::Int(4), &Registry).unwrap(),
            Some(Tag::Int(4))
        );
        assert!(component_to_tag(0, &Component::Nbt(Tag::Int(4)), &Registry).is_err());
        let custom_data = Tag::compound([("four", Tag::Int(4))]);
        assert_eq!(
            component_to_tag(0, &Component::Nbt(custom_data.clone()), &Registry).unwrap(),
            Some(custom_data)
        );
        let use_effects = |speed: f32| {
            let mut wire = Writer::new();
            wire.bool(true);
            wire.bool(true);
            wire.f32(speed);
            Component::Raw(wire.data)
        };
        for speed in [2.0, -0.5, -0.0] {
            assert!(component_to_tag(5, &use_effects(speed), &Registry).is_err());
        }
        assert!(
            component_to_tag(5, &use_effects(0.5), &Registry)
                .unwrap()
                .is_some()
        );
        // The retained value is untouched by the refusal.
        let value = Component::Int(-1);
        let _ = component_to_tag(3, &value, &Registry);
        assert_eq!(value, Component::Int(-1));
        let template = stone(vec![("minecraft:damage".into(), Some(Value::Int(-1)))]);
        assert!(template.to_tag().is_err());
    }
    #[test]
    fn persistent_input_names_the_transient_component_it_refuses() {
        let error = component_from_tag(20, &Tag::Compound(Default::default()), &registry_id)
            .unwrap_err()
            .to_string();
        assert!(error.contains("creative_slot_lock"), "{error}");
    }
    #[test]
    fn saved_presence_predicate_reconstructs_as_a_type_not_a_component_value() {
        use crate::item::component as k;
        let source = Tag::compound([(
            "predicates",
            Tag::compound([(
                "minecraft:creative_slot_lock",
                Tag::Compound(Default::default()),
            )]),
        )]);
        let malformed = Tag::compound([(
            "predicates",
            Tag::compound([("minecraft:creative_slot_lock", Tag::Int(7))]),
        )]);
        assert!(component_from_tag(k::CAN_BREAK, &malformed, &registry_id).is_err());
        let Component::Raw(bytes) =
            component_from_tag(k::CAN_BREAK, &source, &registry_id).unwrap()
        else {
            panic!("can_break uses a predicate stream");
        };
        // One block predicate with no block/state/NBT/exact tests, one partial
        // test, false discriminator, creative_slot_lock, empty compound.
        assert_eq!(bytes, [1, 0, 0, 0, 0, 1, 0, 20, 10, 0]);
        let mut r = Reader::new(&bytes);
        let component = Component::read(k::CAN_BREAK, &mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        assert_eq!(
            component_to_tag(k::CAN_BREAK, &component, &Registry).unwrap(),
            Some(source)
        );
    }

    #[test]
    fn bundle_source_reconstructs_more_than_1024_templates() {
        use crate::item::component as k;
        let source = Tag::List(vec![Tag::String("minecraft:stone".into()); 1025]);
        let Component::Stacks(items) =
            component_from_tag(k::BUNDLE_CONTENTS, &source, &registry_id).unwrap()
        else {
            panic!("bundle contents reconstructs as templates");
        };
        assert_eq!(items.len(), 1025);
        assert!(items.iter().all(|item| item.item == 1 && item.count == 1));
    }

    #[test]
    fn tooltip_wire_values_keep_hidden_types_in_first_seen_order() {
        use crate::item::component as k;
        let mut r = Reader::new(&[0, 3, 3, 20, 3]);
        let component = Component::read(k::TOOLTIP_DISPLAY, &mut r).unwrap();
        let hidden = Tag::List(vec![
            Tag::String("minecraft:damage".into()),
            Tag::String("minecraft:creative_slot_lock".into()),
        ]);
        assert_eq!(
            component_to_tag(k::TOOLTIP_DISPLAY, &component, &Registry).unwrap(),
            Some(Tag::compound([("hidden_components", hidden)]))
        );
        // Hash from the pinned Java codec for the two-type sequenced set.
        assert_eq!(
            component.hash(k::TOOLTIP_DISPLAY, &Registry),
            Some(1575522631)
        );
    }

    #[test]
    fn live_hashes_reject_values_the_saved_codecs_cannot_encode() {
        use crate::item::component as k;
        assert_eq!(Component::Int(-1).hash(k::DAMAGE, &Registry), None);
        assert_eq!(
            Component::Raw(vec![1, 1, 0x40, 0, 0, 0]).hash(5, &Registry),
            None
        );
        for count in [-1, 100] {
            let bundle = Component::Stacks(vec![Stack::new(1, count)]);
            assert_eq!(bundle.hash(k::BUNDLE_CONTENTS, &Registry), None);
        }
    }

    #[test]
    fn template_hash_uses_last_wire_entries_and_omits_transients() {
        use crate::item::component as k;
        let mut w = Writer::new();
        w.varint(1); // one bundle template
        w.varint(1); // stone
        w.varint(1); // count
        w.varint(4); // set entries
        w.varint(3); // removals
        for (kind, value) in [
            (k::DAMAGE, 1),
            (k::DAMAGE, 7),
            (k::REPAIR_COST, 9),
            (k::MAP_POST_PROCESSING, 1),
        ] {
            w.varint(kind);
            w.varint(value);
        }
        for kind in [k::REPAIR_COST, k::CREATIVE_SLOT_LOCK, k::REPAIR_COST] {
            w.varint(kind);
        }
        let mut r = Reader::new(&w.data);
        let component = Component::read(k::BUNDLE_CONTENTS, &mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        // Pinned Java BundleContents.CODEC for stone with damage=7 and !repair_cost.
        assert_eq!(
            component.hash(k::BUNDLE_CONTENTS, &Registry),
            Some(848228328)
        );
    }
    #[test]
    fn a_registry_id_at_the_top_of_the_range_is_refused_not_wrapped() {
        let top = |_: &str, _: &str| Some(i32::MAX);
        for (kind, registry) in [
            (83, "minecraft:sound_event"),
            (64, "minecraft:trim_material"),
            (66, "minecraft:jukebox_song"),
            (109, "minecraft:painting_variant"),
        ] {
            let error = component_from_value(kind, &Value::string("minecraft:test"), &top)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(registry) && error.contains("overflows"),
                "{error}"
            );
        }
    }
}
