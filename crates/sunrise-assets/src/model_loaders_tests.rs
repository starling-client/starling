use super::*;
use crate::{
    bake::{BlockModelTransform, ModelQuad, bake_block_model},
    model::{CuboidModel, Loader, Material, TextureSlot},
    resolve::{ModelLibrary, ResolvedLoader},
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The north face of a 4-pixel post in the middle of the block.
fn post(texture: LoaderTexture) -> LoaderQuad {
    LoaderQuad {
        positions: [
            [0.625, 0.0, 0.375],
            [0.375, 0.0, 0.375],
            [0.375, 1.0, 0.375],
            [0.625, 1.0, 0.375],
        ],
        uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
        texture,
        cull: Some(Direction::North),
        tint_index: 2,
        shade: false,
    }
}

fn library(models: &[(&str, &str)], providers: &[(&str, LoaderProvider)]) -> ModelLibrary {
    let find = |id: &str| {
        providers
            .iter()
            .find_map(|(key, provider)| (*key == id).then(|| provider.clone()))
    };
    let models = models
        .iter()
        .map(|(name, json)| {
            (
                (*name).to_owned(),
                CuboidModel::from_json_with_loaders(json, &find).unwrap(),
            )
        })
        .collect();
    ModelLibrary::new(models).unwrap()
}

fn bake(library: &ModelLibrary, name: &str, turns: [u8; 3]) -> Vec<ModelQuad> {
    bake_block_model(
        &library.resolve(name).unwrap(),
        BlockModelTransform::new(turns, false),
    )
    .unwrap()
}

const CUBOID: &str = r##"{"textures":{"side":"block/post"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#side"}}}]}"##;

#[test]
fn loader_ids_are_canonicalized_before_binding() {
    for (id, expected) in [
        ("asmtest:kept", "asmtest:kept"),
        ("asmtest_vanilla_ns", "minecraft:asmtest_vanilla_ns"),
        (":leading", "minecraft:leading"),
        ("asmtest:", "asmtest:"),
    ] {
        let json = format!(r#"{{"loader":"{id}","width":3}}"#);
        let kept = CuboidModel::from_json_with_loaders(&json, &|found| {
            assert_eq!(found, expected);
            Some(LoaderProvider {
                schema: LoaderSchema::Custom,
                bake: Arc::new(|_, _, _| Ok(Vec::new())),
            })
        })
        .unwrap();
        assert!(matches!(kept.loader, Some(Loader::Registered(_))));
    }
}

#[test]
fn loader_quads_turn_with_the_variant_and_take_the_models_textures() {
    let calls = Arc::new(AtomicUsize::new(0));
    let library = library(
        &[
            ("t:base", r#"{"textures":{"side":"block/post"}}"#),
            ("t:post", r#"{"parent":"t:base","loader":"asmtest:post"}"#),
        ],
        &[(
            "asmtest:post",
            LoaderProvider {
                schema: LoaderSchema::ModelProperties,
                bake: Arc::new({
                    let calls = Arc::clone(&calls);
                    move |_, _, _| {
                        calls.fetch_add(1, Ordering::Relaxed);
                        Ok(vec![
                            post(LoaderTexture::Slot("side".into())),
                            post(LoaderTexture::Slot("absent".into())),
                        ])
                    }
                }),
            },
        )],
    );
    let quads = bake(&library, "t:post", [0, 1, 0]);
    // The quad whose texture the model lacks is skipped; the other is baked.
    assert_eq!(quads.len(), 1);
    let quad = &quads[0];
    assert_eq!(
        quad.positions,
        [
            [0.625, 0.0, 0.625],
            [0.625, 0.0, 0.375],
            [0.625, 1.0, 0.375],
            [0.625, 1.0, 0.625]
        ]
    );
    assert_eq!(quad.cull, Some(Direction::East));
    assert_eq!(quad.material.sprite, "minecraft:block/post");
    assert_eq!(quad.uv, [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]]);
    assert_eq!((quad.tint_index, quad.shade), (2, false));
    bake(&library, "t:post", [0, 0, 0]);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn a_provider_owns_its_model_schema() {
    let library = library(
        &[(
            "t:custom",
            r#"{"loader":"asmtest:own","parent":[],"textures":null,"elements":{"sprite":"t:custom"}}"#,
        )],
        &[(
            "asmtest:own",
            LoaderProvider {
                schema: LoaderSchema::Custom,
                bake: Arc::new(|_, json, textures| {
                    let config: serde_json::Value = serde_json::from_str(json).unwrap();
                    assert!(textures.is_empty());
                    let sprite = config["elements"]["sprite"].as_str().unwrap();
                    Ok(vec![post(LoaderTexture::Sprite(sprite.to_owned()))])
                }),
            },
        )],
    );
    let quads = bake(&library, "t:custom", [0; 3]);
    assert_eq!(quads.len(), 1);
    assert_eq!(quads[0].material.sprite, "t:custom");
}

#[test]
fn only_explicit_cuboid_delegation_keeps_elements_alongside_custom_quads() {
    let json = |loader: &str| {
        format!(
            r##"{{"loader":"{loader}","parent":"t:base","elements":[{{"from":[0,0,0],"to":[16,16,16],"faces":{{"north":{{"texture":"#side"}}}}}}]}}"##
        )
    };
    let empty = json("asmtest:empty");
    let cuboids = json("asmtest:cuboids");
    let library = library(
        &[
            ("t:base", CUBOID),
            ("t:empty", &empty),
            ("t:cuboids", &cuboids),
        ],
        &[
            (
                "asmtest:empty",
                LoaderProvider {
                    schema: LoaderSchema::ModelProperties,
                    bake: Arc::new(|_, _, _| Ok(Vec::new())),
                },
            ),
            (
                "asmtest:cuboids",
                LoaderProvider {
                    schema: LoaderSchema::Cuboids,
                    bake: Arc::new(|_, _, _| Ok(vec![post(LoaderTexture::Slot("side".into()))])),
                },
            ),
        ],
    );
    let empty = library.resolve_required("t:empty").unwrap();
    assert!(matches!(empty.loader(), Some(ResolvedLoader::Registered(q)) if q.is_empty()));
    assert!(bake(&library, "t:empty", [0; 3]).is_empty());
    let cuboids = bake(&library, "t:cuboids", [0; 3]);
    assert_eq!(cuboids.len(), 2);
    assert_eq!(cuboids[0].facing, Direction::North);
    assert_eq!(cuboids[1].cull, Some(Direction::North));
}

#[test]
fn a_refusing_provider_bakes_once_for_shared_model_variants() {
    let calls = Arc::new(AtomicUsize::new(0));
    let library = library(
        &[(
            "t:refused",
            r#"{"loader":{"id":"asmtest:refuses","optional":true}}"#,
        )],
        &[(
            "asmtest:refuses",
            LoaderProvider {
                schema: LoaderSchema::Custom,
                bake: Arc::new({
                    let calls = Arc::clone(&calls);
                    move |_, _, _| {
                        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                            Err("invalid loader configuration".to_owned())
                        } else {
                            Ok(Vec::new())
                        }
                    }
                }),
            },
        )],
    );
    for turns in [[0; 3], [0, 1, 0]] {
        let missing = bake(&library, "t:refused", turns);
        assert_eq!(missing.len(), 6);
        assert!(
            missing
                .iter()
                .all(|quad| quad.material.sprite == "minecraft:missingno")
        );
    }
    assert!(library.resolve_required("t:refused").is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn inherited_loader_geometry_is_cached_per_model_identity() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let library = library(
        &[
            (
                "t:parent",
                r#"{"loader":"asmtest:named","textures":{"side":"block/post"}}"#,
            ),
            ("t:child", r#"{"parent":"t:parent"}"#),
        ],
        &[(
            "asmtest:named",
            LoaderProvider {
                schema: LoaderSchema::ModelProperties,
                bake: Arc::new({
                    let calls = Arc::clone(&calls);
                    move |name, _, textures| {
                        calls
                            .lock()
                            .unwrap()
                            .push((name.to_owned(), textures.clone()));
                        let mut quad = post(LoaderTexture::Slot("side".into()));
                        quad.tint_index = if name == "t:parent" { 1 } else { 7 };
                        Ok(vec![quad])
                    }
                }),
            },
        )],
    );
    for _ in 0..2 {
        assert_eq!(bake(&library, "t:parent", [0; 3])[0].tint_index, 1);
        assert_eq!(bake(&library, "t:child", [0; 3])[0].tint_index, 7);
    }
    let expected = BTreeMap::from([("side".to_owned(), "minecraft:block/post".to_owned())]);
    assert_eq!(
        *calls.lock().unwrap(),
        [
            ("t:parent".to_owned(), expected.clone()),
            ("t:child".to_owned(), expected)
        ]
    );
}

#[test]
fn resolve_with_textures_caches_loader_quads_per_effective_texture_map() {
    let make_override = |sprite: &str| {
        BTreeMap::from([(
            "side".to_owned(),
            TextureSlot::Material(Material {
                sprite: sprite.to_owned(),
                force_translucent: false,
            }),
        )])
    };
    let gold = make_override("minecraft:block/gold_block");
    let iron = make_override("minecraft:block/iron_block");

    // Test both resolution orders to prove neither leaks cached quads into the other.
    for (first, first_tint, second, second_tint) in [(&gold, 10, &iron, 20), (&iron, 20, &gold, 10)]
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let lib = library(
            &[(
                "t:tex",
                r#"{"loader":"asmtest:tex_tint","textures":{"side":"block/post"}}"#,
            )],
            &[(
                "asmtest:tex_tint",
                LoaderProvider {
                    schema: LoaderSchema::ModelProperties,
                    bake: Arc::new({
                        let calls = Arc::clone(&calls);
                        move |_, _, textures| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            let mut quad = post(LoaderTexture::Slot("side".into()));
                            quad.tint_index = if textures["side"] == "minecraft:block/gold_block" {
                                10
                            } else {
                                20
                            };
                            Ok(vec![quad])
                        }
                    }),
                },
            )],
        );
        for (overrides, tint) in [
            (first, first_tint),
            (second, second_tint),
            (first, first_tint),
        ] {
            let quads = bake_block_model(
                &lib.resolve_with_textures("t:tex", overrides).unwrap(),
                BlockModelTransform::new([0; 3], false),
            )
            .unwrap();
            assert_eq!(quads[0].tint_index, tint);
        }
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }
}
