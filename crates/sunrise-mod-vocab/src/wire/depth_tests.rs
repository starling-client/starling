use super::*;
use crate::item::crafting::Shape;
use crate::world::snapshots::LootEntity;
use crate::{RecipeSlotDisplay, Tag, Text};

fn round<T: Wire>(value: &T, initial: u16) {
    let mut out = Writer::new();
    out.put(value);
    assert_eq!(out.error(), None);
    let mut reader = Reader::new(out.bytes());
    reader.depth = initial;
    assert!(reader.take::<T>().is_ok());
    let mut reader = Reader::new(out.bytes());
    reader.depth = initial + 1;
    assert!(matches!(reader.take::<T>(), Err(WireError::Depth)));
}

/// The fixed deepest value crosses the wire; one extra wrap does not encode, and the bytes
/// of the deepest value do not decode with one more reader envelope.
fn recursive_at_limit<T: Wire>(leaf: T, wraps: usize, reader_depth: u16, wrap: impl Fn(T) -> T) {
    let value = (0..wraps).fold(leaf, |value, _| wrap(value));
    round(&value, reader_depth);
    let mut out = Writer::new();
    out.put(&wrap(value));
    assert_eq!(out.error(), Some(WireError::Depth));
}

/// #3945: a datapack registry's schema and values are recursive over the Wire. The generic
/// envelope bounds them: the deepest value or schema the writer takes decodes, one level more
/// is refused on both sides, and it fits a small stack.
#[test]
fn datapack_schemas_and_values_stop_at_the_generic_depth() {
    use crate::world::datapack_registry::{DatapackField, DatapackFieldKind, DatapackValue};
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            // Value variants each take one envelope; their Vec and tuple add none.
            recursive_at_limit(DatapackValue::Int(1), 31, 0, |v| {
                DatapackValue::List(vec![v])
            });
            recursive_at_limit(DatapackValue::Int(1), 31, 0, |v| {
                DatapackValue::Record(vec![("a".into(), v)])
            });
            // A boxed list kind uses two envelopes per wrapper and one at the leaf.
            recursive_at_limit(DatapackFieldKind::Bool, 15, 1, |of| {
                DatapackFieldKind::List {
                    of: Box::new(of),
                    min_len: 0,
                    max_len: None,
                }
            });
            // A record field and its kind use two envelopes per wrapper and at the leaf.
            recursive_at_limit(
                DatapackField::required("a", DatapackFieldKind::Bool),
                15,
                0,
                |field| {
                    DatapackField::required(
                        "a",
                        DatapackFieldKind::Record {
                            fields: vec![field],
                        },
                    )
                },
            );
            // A shallow tree, the usual case, crosses unchanged.
            let shallow = DatapackValue::Map(vec![(
                "minecraft:zombie".into(),
                DatapackValue::List(vec![DatapackValue::Record(vec![(
                    "count".into(),
                    DatapackValue::Int(2),
                )])]),
            )]);
            let mut out = Writer::new();
            out.put(&shallow);
            assert_eq!(
                Reader::decode::<DatapackValue>(out.bytes()).unwrap(),
                shallow
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn structural_families_fit_a_small_stack() {
    // Verify that the depth limit also bounds stack use.
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            // A raised limit is not safe merely because the depth counter still works.
            assert_eq!(MAX_DEPTH, 32);
            let mut display = RecipeSlotDisplay::Empty;
            for _ in 1..32 {
                display = RecipeSlotDisplay::Composite(vec![display]);
            }
            round(&display, 0);
            let mut entity = LootEntity::default();
            for _ in 0..15 {
                entity = LootEntity {
                    vehicle: Some(Box::new(entity)),
                    ..Default::default()
                };
            }
            round(&Box::new(entity), 0);
            // These public SCC members no longer recurse structurally: every Text
            // edge crosses through the stack-independent iterative codec. Their own
            // envelope still guards depth.
            round(&crate::text::TextContent::Literal(String::new()), 31);
            round(&crate::text::TextStyle::default(), 31);
            round(
                &crate::text::TextHover::Item {
                    id: String::new(),
                    count: 1,
                    components: None,
                },
                31,
            );
            round(
                &crate::text::TextObject::Atlas {
                    atlas: String::new(),
                    sprite: String::new(),
                    fallback: None,
                },
                31,
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn vanilla_trees_keep_their_depth_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            for compound in [false, true] {
                let mut tag = Tag::Int(7);
                for _ in 0..512 {
                    tag = if compound {
                        Tag::compound([("k", tag)])
                    } else {
                        Tag::List(vec![tag])
                    };
                }
                let mut out = Writer::new();
                out.put(&tag);
                assert_eq!(out.error(), None);
                let read = Reader::decode::<Tag>(out.bytes()).unwrap();
                sunrise_core::nbt::discard_tag(read);
                let mut bytes = if compound {
                    vec![10, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 107, 0]
                } else {
                    vec![9, 0, 0, 0, 1, 0, 0, 0]
                };
                bytes.extend_from_slice(out.bytes());
                assert!(matches!(
                    Reader::decode::<Tag>(&bytes),
                    Err(WireError::Depth)
                ));
                sunrise_core::nbt::discard_tag(tag);
            }
            let mut text = Text::literal("leaf");
            for _ in 1..512 {
                text = Text::literal("").append(text);
            }
            let mut out = Writer::new();
            out.put(&text);
            assert_eq!(out.error(), None);
            // Wrap the 512-deep encoded tree in one more parent node to exceed MAX_TEXT_DEPTH.
            let mut too_deep_bytes = vec![0; 4 + 4 + 11]; // Literal(""), unset style options.
            too_deep_bytes.extend([10, 0, 0, 0, 0, 0, 0, 0]); // Empty style metadata compound.
            too_deep_bytes.extend([1, 0, 0, 0]); // One child.
            too_deep_bytes.extend_from_slice(out.bytes());
            too_deep_bytes.extend([10, 0, 0, 0, 0, 0, 0, 0]); // Empty component metadata compound.
            assert_eq!(
                Reader::decode::<Text>(&too_deep_bytes),
                Err(WireError::Depth)
            );
            let decoded = Reader::decode::<Text>(out.bytes()).unwrap();
            let text = Text::literal("").append(text);
            let mut too_deep = Writer::new();
            too_deep.put(&text);
            assert_eq!(too_deep.error(), Some(WireError::Depth));
            // Drop both deep trees directly on the 128 KiB stack (iterative Drop for Text).
            drop(text);
            drop(decoded);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn json_backed_codecs_preflight_before_serde_recursion() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let quoted = serde_json::Value::String("[\\\"{".repeat(1024));
            let mut quoted_bytes = Writer::new();
            quoted_bytes.put(&quoted);
            assert_eq!(
                Reader::decode::<serde_json::Value>(quoted_bytes.bytes()).unwrap(),
                quoted
            );
            let mut value = serde_json::Value::Null;
            for _ in 0..32 {
                value = serde_json::Value::Array(vec![value]);
            }
            let mut out = Writer::new();
            out.put(&value);
            assert_eq!(out.error(), None);
            let mut map_at_limit = serde_json::Map::new();
            let mut child = serde_json::Value::Null;
            for _ in 0..31 {
                child = serde_json::Value::Array(vec![child]);
            }
            map_at_limit.insert("leaf".into(), child);
            let mut map_bytes = Writer::new();
            map_bytes.put(&map_at_limit);
            assert_eq!(map_bytes.error(), None);
            assert!(
                Reader::decode::<serde_json::Map<String, serde_json::Value>>(map_bytes.bytes())
                    .is_ok()
            );
            let decoded = Reader::decode::<serde_json::Value>(out.bytes()).unwrap();
            drop(decoded);
            value = serde_json::Value::Array(vec![value]);
            let mut out = Writer::new();
            out.put(&value);
            assert_eq!(out.error(), Some(WireError::Depth));
            let text = format!("{}null{}", "[".repeat(33), "]".repeat(33));
            let mut raw = Writer::new();
            raw.put(&text);
            assert!(matches!(
                Reader::decode::<serde_json::Value>(raw.bytes()),
                Err(WireError::Depth)
            ));
            for _ in 33..1024 {
                value = serde_json::Value::Array(vec![value]);
            }
            let mut map = serde_json::Map::new();
            map.insert("deep".into(), value);
            let mut out = Writer::new();
            out.put(&map);
            assert_eq!(out.error(), Some(WireError::Depth));
            let mut pending = vec![serde_json::Value::Object(map)];
            while let Some(value) = pending.pop() {
                match value {
                    serde_json::Value::Array(values) => pending.extend(values),
                    serde_json::Value::Object(values) => pending.extend(values.into_values()),
                    _ => {}
                }
            }
            let mut model = crate::ModelPart::new(vec![], vec![], Default::default());
            for _ in 0..10 {
                model = crate::ModelPart::new(
                    vec![],
                    vec![("child".into(), model)],
                    Default::default(),
                );
            }
            let mut valid_model = Writer::new();
            valid_model.put(&model);
            assert_eq!(valid_model.error(), None);
            let _ = Reader::decode::<crate::ModelPart>(valid_model.bytes()).unwrap();
            for depth in 10..1024 {
                if depth == 11 {
                    let mut refused = Writer::new();
                    refused.put(&model);
                    assert_eq!(refused.error(), Some(WireError::Depth));
                }
                model = crate::ModelPart::new(
                    vec![],
                    vec![("child".into(), model)],
                    Default::default(),
                );
            }
            let mut out = Writer::new();
            out.put(&model);
            assert_eq!(out.error(), Some(WireError::Depth));
            let mut parts = vec![model];
            while let Some(mut part) = parts.pop() {
                for (_, child) in part.children_mut() {
                    parts.push(std::mem::replace(
                        child,
                        crate::ModelPart::new(vec![], vec![], Default::default()),
                    ));
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn ingredient_shape_contract_keeps_existing_tags_and_adds_custom_display_at_three() {
    for (shape, bytes) in [
        (Shape::Compound, [0, 0, 0, 0]),
        (Shape::Intersection, [1, 0, 0, 0]),
        (Shape::Difference, [2, 0, 0, 0]),
        (Shape::CustomDisplay, [3, 0, 0, 0]),
    ] {
        let mut out = Writer::new();
        out.put(&shape);
        assert_eq!(out.bytes(), bytes);
        assert_eq!(Reader::decode::<Shape>(&bytes).unwrap(), shape);
    }
}

#[test]
fn explosion_interactions_cross_in_vanillas_order() {
    use crate::world::types::ExplosionInteraction;
    for (interaction, bytes) in [
        (ExplosionInteraction::None, [0, 0, 0, 0]),
        (ExplosionInteraction::Block, [1, 0, 0, 0]),
        (ExplosionInteraction::Mob, [2, 0, 0, 0]),
        (ExplosionInteraction::Tnt, [3, 0, 0, 0]),
        (ExplosionInteraction::Trigger, [4, 0, 0, 0]),
    ] {
        let mut out = Writer::new();
        out.put(&interaction);
        assert_eq!(out.bytes(), bytes);
        assert_eq!(
            Reader::decode::<ExplosionInteraction>(&bytes).unwrap(),
            interaction
        );
    }
}

#[test]
fn boxed_ingredient_metadata_preserves_its_value_encoding() {
    let rule = Some(crate::item::crafting::IngredientRule::Components {
        patch: [("minecraft:damage".into(), Tag::Int(5))].into(),
        removed: vec!["minecraft:enchantments".into()],
        strict: true,
    });
    let mut inline = Writer::new();
    inline.put(&rule);
    let mut boxed = Writer::new();
    boxed.put(&rule.map(Box::new));
    assert_eq!(inline.bytes(), boxed.bytes());
    let display = Some(sunrise_core::nbt::Compound::from([
        ("type".into(), Tag::String("minecraft:item".into())),
        ("item".into(), Tag::String("minecraft:diamond".into())),
    ]));
    let mut inline = Writer::new();
    inline.put(&display);
    let mut boxed = Writer::new();
    boxed.put(&display.map(Box::new));
    assert_eq!(inline.bytes(), boxed.bytes());
}
