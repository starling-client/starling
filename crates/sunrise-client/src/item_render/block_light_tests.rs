//! A model that selects on `minecraft:context_dimension` takes its fallback when it resolves with
//! no level, which is how `item_uses_block_light` resolves: the world the player is in is not
//! part of that read.
use super::*;
use std::fs;
use sunrise_assets::{item_model::LevelState, resource_source::ResourceRoot};

struct Plain;
impl ItemEnv for Plain {}
impl ItemDrawEnv for Plain {}

#[test]
fn a_dimension_select_takes_its_fallback_without_a_level() {
    let root = std::env::temp_dir().join(format!("sunrise-item-light-{}", std::process::id()));
    let write = |name: &str, data: &str| {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    };
    write("items.json", "{}");
    write(
        "assets/minecraft/atlases/items.json",
        r#"{"sources":[{"type":"single","resource":"lit:item/grey"}]}"#,
    );
    super::glint_tests::png(
        &root.join("assets/lit/textures/item/grey.png"),
        16,
        16,
        [128, 128, 128, 255],
    );
    for (name, light) in [("side", "side"), ("flat", "front")] {
        write(
            &format!("assets/lit/models/item/{name}.json"),
            &format!(
                r##"{{"gui_light":"{light}","textures":{{"layer0":"lit:item/grey"}},"elements":[{{"from":[0,0,8],"to":[16,16,8],
                "faces":{{"south":{{"uv":[0,0,16,16],"texture":"#layer0"}}}}}}]}}"##
            ),
        );
    }
    write(
        "assets/lit/items/dim.json",
        r#"{"model":{"type":"minecraft:select","property":"minecraft:context_dimension",
            "cases":[{"when":"minecraft:overworld","model":{"type":"minecraft:model","model":"lit:item/side"}}],
            "fallback":{"type":"minecraft:model","model":"lit:item/flat"}}}"#,
    );
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(&ResourceRoot::directory(root.to_path_buf()), &data).unwrap();
    let mut stack = StackView::new("lit:dim", 1);
    stack.patch.insert(
        "minecraft:item_model".into(),
        Some(serde_json::json!("lit:dim")),
    );
    let overworld = LevelState {
        dimension: "minecraft:overworld".to_owned(),
        game_time: 0,
        sun_angle: 0.0,
        moon_phase: 0,
        respawn: None,
        runs_normally: true,
    };
    let lit = |level: Option<&LevelState>| {
        renderer
            .resolve(&stack, DisplayContext::Gui, None, level, 0, &Plain)
            .uses_block_light()
    };
    assert!(lit(Some(&overworld)));
    assert!(!lit(None));
    fs::remove_dir_all(root).unwrap();
}
