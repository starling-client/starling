use horizon_core::nbt::Tag;
use serde_json::{Value, json};

fn describe(tag: &Tag) -> Value {
    let value = match tag {
        Tag::End => Value::Null,
        Tag::Byte(n) => json!(n.to_string()),
        Tag::Short(n) => json!(n.to_string()),
        Tag::Int(n) => json!(n.to_string()),
        Tag::Long(n) => json!(n.to_string()),
        Tag::Float(n) => json!(n.to_string()),
        Tag::Double(n) => json!(n.to_string()),
        Tag::String(s) => json!(s.to_string().unwrap()),
        Tag::List(list) => list.iter().map(describe).collect(),
        Tag::Compound(map) => map
            .iter()
            .map(|(k, v)| (k.to_string().unwrap(), describe(v)))
            .collect(),
        _ => panic!("JSON must never become a typed NBT array"),
    };
    json!({"type": tag.id(), "value": value})
}

#[test]
fn matches_actual_vanilla_jsonops_to_nbtops() {
    let inputs: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("JSON_NBT_SAMPLES").unwrap()).unwrap(),
    )
    .unwrap();
    let expected: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("JSON_NBT_ORACLE").unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(inputs.len(), expected.len());
    for (input, expected) in inputs.iter().zip(expected) {
        let actual = match crate::production::json_to_tag(input) {
            Ok(tag) => describe(&tag),
            Err(crate::production::JsonNbtError::NestedNull) => json!({"error": "NestedNull"}),
        };
        assert_eq!(actual, expected, "{input}");
    }
}
