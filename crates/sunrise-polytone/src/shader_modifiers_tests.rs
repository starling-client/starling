use super::*;
use serde_json::json;

fn names(m: &ShaderModifier) -> Vec<&str> {
    m.uniforms.iter().map(|(n, _)| n.as_str()).collect()
}

/// The wiki's example: names to expressions, strings, numbers and booleans alike.
#[test]
fn a_file_maps_block_names_to_expressions() {
    let m = ShaderModifier::parse(&json!({
        "MyIntensity": "sin(g.time * 0.05) * 0.5 + 0.5",
        "DayProgress": "g.dayTime / 24000.0",
        "Half": 0.5,
        "On": true,
    }))
    .unwrap();
    assert_eq!(names(&m), ["DayProgress", "Half", "MyIntensity", "On"]);
    assert!(
        ShaderModifier::parse(&json!({}))
            .unwrap()
            .uniforms
            .is_empty()
    );
}

/// Each refusal is named, and refuses the whole file.
#[test]
fn a_bad_file_is_refused_by_what_is_wrong() {
    let refused = |j: Json| ShaderModifier::parse(&j).unwrap_err();
    assert_eq!(refused(json!(["Tint"])), Refused::NotAnObject);
    assert_eq!(refused(json!("Tint")), Refused::NotAnObject);
    for bad in [
        "",
        "9Tint",
        "Tint-2",
        "gl_Tint",
        "Ünder",
        &"T".repeat(MAX_NAME + 1),
    ] {
        assert_eq!(
            refused(json!({ bad: "1", "Fine": "1" })),
            Refused::Name(bad.to_owned()),
            "{bad:?}"
        );
    }
    assert!(ShaderModifier::parse(&json!({ "T".repeat(MAX_NAME): "1", "_x9": "1" })).is_ok());
    assert!(matches!(
        refused(json!({"Tint": "1 +"})),
        Refused::Expression { name, .. } if name == "Tint"
    ));
    assert!(matches!(
        refused(json!({"Tint": [1]})),
        Refused::Expression { name, .. } if name == "Tint"
    ));
    assert!(matches!(
        refused(json!({"Tint": null})),
        Refused::Expression { name, .. } if name == "Tint"
    ));
}

/// At most [`MAX_UNIFORMS`] (16) a file: 16 are read, 17 refused.
#[test]
fn a_file_has_at_most_sixteen_uniforms() {
    let file =
        |n: usize| Json::Object((0..n).map(|i| (format!("U{i}"), Json::from("1"))).collect());
    assert_eq!(ShaderModifier::parse(&file(16)).unwrap().uniforms.len(), 16);
    assert_eq!(
        ShaderModifier::parse(&file(17)).unwrap_err(),
        Refused::TooMany { count: 17 }
    );
}
