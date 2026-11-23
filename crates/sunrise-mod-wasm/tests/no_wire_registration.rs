//! Both build targets resolve registration through the shared constructor emitter.
fn refused(prefix: &str, target: &str) {
    static POINTS: std::sync::OnceLock<Vec<sunrise_mod_point_emit::Point>> =
        std::sync::OnceLock::new();
    let points = POINTS.get_or_init(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let points = sunrise_mod_api_gen::points::game_points(&root).unwrap();
        serde_json::from_value(serde_json::to_value(points).unwrap()).unwrap()
    });
    let source = r#"id = "refused_fixture"
name = "Refused fixture"
version = "1.0.0"
api = "0.2"
capabilities = ["registries"]
[[inject]]
id = "no_wire"
at = "listener_fixture::no_wire"
kind = "head"
with = "no_wire"
hash = "fixture"
"#;
    let manifest = sunrise_mod_package::Manifest::parse(&format!("{source}\n{target}")).unwrap();
    assert_eq!(
        sunrise_mod_point_emit::api(&manifest, points, prefix).unwrap_err(),
        "no_wire: no API listener at listener_fixture::no_wire: argument 1 `value` (`&RegistrationData`) has no API type yet"
    );
    assert_eq!(
        sunrise_mod_point_emit::places(&manifest, points).unwrap_err(),
        "no_wire: no API listener at listener_fixture::no_wire: argument 1 `value` (`&RegistrationData`) has no API type yet"
    );
}

#[test]
#[ignore = "daily"]
fn native_registration_refuses_registration_data_without_wire() {
    refused(
        "mod_0::",
        r#"[native]
game = "26.3"
sunrise = "0000000000000000000000000000000000000000"
mixins = "0000000000000000000000000000000000000000""#,
    );
}
#[test]
#[ignore = "daily"]
fn wasm_registration_refuses_registration_data_without_wire() {
    refused(
        "original_mod::",
        &format!(
            "[wasm]\nsha512 = \"{}\"\ntoolchain = \"fixture\"",
            "0".repeat(128)
        ),
    );
}
