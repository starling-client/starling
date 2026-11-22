use super::*;

#[test]
fn a_folder_without_prepared_files_is_named_before_loading() {
    let root = std::env::temp_dir().join(format!("sunrise-unprepared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let unprepared = root.join(".local/sunrise-client/26.3");
    let missing = missing_resources(&unprepared, None).expect("no manifest is reported");
    assert!(missing.contains("sunrise-resources.json"), "{missing}");
    assert!(missing.contains("sunrise-unprepared-"), "{missing}");
    assert!(missing.contains("sunrise-prepare --prepare"), "{missing}");
    // No hook: no site, no launcher.
    assert!(!missing.contains("playsunrise"), "{missing}");
    assert!(!missing.contains("launcher"), "{missing}");
    let get = sunrise_hooks::GetTheGame {
        address: "https://example.org/get",
        title: "",
        paragraphs: &[],
        button: "",
    };
    let missing = missing_resources(&unprepared, Some(get)).expect("no manifest is reported");
    assert!(
        missing.contains("launcher: download it at https://example.org/get"),
        "{missing}"
    );
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("sunrise-resources.json"), "{}").unwrap();
    assert_eq!(missing_resources(&root, None), None);

    // A killed export leaves an empty file behind; the error names it
    // instead of serde's "expected value at line 1 column 1".
    std::fs::write(root.join("gameplay-facts.json"), "").unwrap();
    let damaged = missing_resources(&root, None).expect("an empty prepared file is reported");
    assert!(damaged.contains("gameplay-facts.json"), "{damaged}");
    assert!(damaged.contains("damaged"), "{damaged}");
    std::fs::write(root.join("gameplay-facts.json"), "{}").unwrap();
    assert_eq!(missing_resources(&root, None), None);

    std::fs::write(root.join("sunrise-resources.json"), " \n").unwrap();
    let damaged = missing_resources(&root, None).expect("an empty manifest is reported");
    assert!(damaged.contains("sunrise-resources.json"), "{damaged}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn worker_prepares_owned_models_and_inventory_before_handoff() {
    let root = std::env::temp_dir().join(format!("sunrise-startup-cpu-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("entity-models.json"),
        r#"{
        "minecraft:end_crystal#main": {}, "minecraft:ender_dragon#main": {},
        "minecraft:arrow#main": {}, "minecraft:trident#main": {},
        "minecraft:armor_stand#main": {}, "minecraft:cushion#main": {},
        "minecraft:leash_knot#main": {}, "minecraft:wind_charge#main": {},
        "minecraft:shulker_bullet#main": {}, "minecraft:wither_skull#main": {},
        "minecraft:evoker_fangs#main": {}, "minecraft:standing_banner#main": {},
        "minecraft:standing_banner#flag": {}
    }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("creative-inventory.json"),
        r#"{"schema":4,"item_features":{},"potion_features":{},"tabs":[{
        "id":"minecraft:building_blocks","title":"Building Blocks","type":"CATEGORY","build":[
            {"kind":"stack","visibility":"PARENT_AND_SEARCH_TABS","snbt":"{count:1,id:\"minecraft:stone\"}"}
        ]
    }]}"#,
    )
    .unwrap();
    use std::collections::BTreeMap;
    use sunrise_assets::{
        model::CuboidModel,
        model_loaders::{LoaderProvider, LoaderQuad, LoaderSchema, LoaderTexture},
        resolve::ModelLibrary,
        resource_source::ResourceRoot,
    };
    let models = |sprite: &str| {
        let frame = CuboidModel::from_json_with_loaders(
            &format!(r#"{{"loader":"example:frame_loader","textures":{{"frame":"{sprite}"}}}}"#),
            &|loader| {
                assert_eq!(loader, "example:frame_loader");
                Some(LoaderProvider {
                    schema: LoaderSchema::ModelProperties,
                    bake: Arc::new(|_, _, _| {
                        Ok(vec![LoaderQuad {
                            positions: [
                                [0.75, 0.0, 0.5],
                                [0.25, 0.0, 0.5],
                                [0.25, 1.0, 0.5],
                                [0.75, 1.0, 0.5],
                            ],
                            uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
                            texture: LoaderTexture::Slot("frame".into()),
                            cull: None,
                            tint_index: -1,
                            shade: true,
                        }])
                    }),
                })
            },
        )
        .unwrap();
        let item_frame = CuboidModel::from_json(r#"{"parent":"example:block/frame"}"#).unwrap();
        ModelLibrary::with_item_generator(BTreeMap::from([
            ("minecraft:block/item_frame".into(), item_frame),
            ("example:block/frame".into(), frame),
        ]))
        .unwrap()
    };
    let worker_root = root.clone();
    let prepared = std::thread::spawn(move || {
        let root = ResourceRoot::directory(worker_root);
        let library = models("example:block/startup_frame");
        GameCpuResources::load(&root, None, Some(&library)).map_err(|error| error.to_string())
    })
    .join()
    .unwrap()
    .unwrap();
    assert_eq!(prepared.frame_model.len(), 1);
    assert_eq!(prepared.frame_model[0].2, "example:block/startup_frame");
    let reloaded = models("example:block/reloaded_frame");
    let [frame, ..] = frames::bake_frames(Some(&reloaded));
    assert_eq!(frame.len(), 1);
    assert_eq!(frame[0].2, "example:block/reloaded_frame");
    std::fs::remove_dir_all(&root).unwrap();
    assert!(prepared.crystal_model.is_some());
    assert!(prepared.cushion_model.is_some());
    let models = &prepared.entity_models;
    assert!(models.leash_knot.is_some() && models.wind_charge.is_some());
    assert!(models.shulker_bullet.is_some() && models.wither_skull.is_some());
    assert!(models.evoker_fangs.is_some() && models.banner.is_some());
    assert!(prepared.dragon_model.is_some());
    assert!(prepared.arrow_model.is_some());
    assert!(prepared.trident_model.is_some());
    assert!(prepared.armor_stand_assets.main.is_some());
    assert!(!prepared.creative_screen.has_contents());
    let contents = prepared.creative_screen.catalog().build(
        sunrise_items::creative_catalog::Parameters {
            operator: false,
            features: &[],
            holders: &Default::default(),
            items: &sunrise_items::items::Items::empty(),
        },
        None,
        |_, _, _| {},
    );
    assert_eq!(contents[0].display[0].item.as_ref(), "minecraft:stone");
    // No documents from the old generation remain available to reload.
    assert!(
        GameCpuResources::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            None,
            None,
        )
        .is_err()
    );
}
