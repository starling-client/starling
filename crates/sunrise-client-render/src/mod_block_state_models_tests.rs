use super::*;
use crate::{
    model_library::ParsedModels, models::StateModels, resource_source::ResourceSource,
    sprites::Sprites,
};
use mod_api::{
    __private::{game::RegistrationContextNew, take_hook_panics},
    RegistrationContext,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use sunrise_core::{nbt::Tag, position::BlockPos};
use sunrise_world::{
    DATA_VERSION,
    chunk::BlockState,
    registry::{BlockRegistry, BlockStateId},
};

fn pick(state: u32, model: &str) -> BlockStateModelPick {
    BlockStateModelPick::Model {
        state,
        model: model.into(),
        turns: [0; 3],
        uvlock: false,
    }
}

#[test]
fn picks_fill_each_states_slot_and_turns_wrap() {
    let picks = vec![
        BlockStateModelPick::Model {
            state: 2,
            model: "lamps:block/on".into(),
            turns: [4, 7, 255],
            uvlock: true,
        },
        pick(0, "block/off"),
    ];
    let models = picked("lamps:t", 3, picks).unwrap();
    assert_eq!(models[0].as_ref().unwrap().model, "minecraft:block/off");
    assert!(models[1].is_none());
    let on = models[2].as_ref().unwrap();
    assert_eq!((on.rotations, on.uv_lock), ([0, 3, 3], true));
}

/// Every malformed answer refuses the file with a reason, never a panic.
#[test]
fn a_malformed_answer_refuses_the_file_with_a_reason() {
    let cases = [
        (
            vec![pick(u32::MAX, "block/a")],
            "picked state 4294967295, but the block has 2 states",
        ),
        (
            vec![pick(2, "block/a")],
            "picked state 2, but the block has 2 states",
        ),
        (
            vec![pick(1, "block/a"), pick(1, "block/b")],
            "picked state 1 twice",
        ),
        (vec![pick(0, "Not A Model")], "picked model \"Not A Model\""),
        (Vec::new(), "picked no model"),
        (
            vec![
                pick(0, "block/a"),
                BlockStateModelPick::Refuse {
                    reason: "needs \"on\"".into(),
                },
            ],
            "refused it: needs \"on\"",
        ),
    ];
    for (picks, reason) in cases {
        let error = picked("lamps:t", 2, picks).unwrap_err();
        assert!(error.contains(reason), "{error:?} names {reason:?}");
        assert!(error.contains("lamps:t"), "{error:?} names the type");
    }
}

/// A lamp's definition type: state by state, `on` when `powered` is true, else `off`.
struct Lamps;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Lamps {
    fn register_block_state_models(
        &self,
        _: &RegistrationContext,
        event: &mut mod_api::RegisterBlockStateModelsEvent<'_>,
    ) {
        event.register(Identifier::parse("rendtest:by_power").unwrap(), |input| {
            let (Some(Tag::String(on)), Some(Tag::String(off))) =
                (input.config.get("on"), input.config.get("off"))
            else {
                return Err("needs \"on\" and \"off\"".into());
            };
            Ok(input
                .states
                .iter()
                .enumerate()
                .map(|(state, properties)| {
                    let lit = properties.get("powered").is_some_and(|v| v == "true");
                    let model = if lit { on } else { off };
                    pick(state as u32, &model.to_string_lossy())
                })
                .collect())
        });
    }
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let fixture =
            Self(std::env::temp_dir().join(format!("sunrise-bsm-{name}-{}", std::process::id())));
        let cube = r##"{"textures":{"all":"rendtest:block/lamp"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}"##;
        for model in ["on", "off", "dim", "post"] {
            fixture.write(&format!("assets/rendtest/models/block/{model}.json"), cube);
        }
        // Vanilla's multipart beside the typed file.
        fixture.write(
            "assets/rendtest/blockstates/post.json",
            r#"{"multipart":[{"apply":{"model":"rendtest:block/post"}},{"apply":{"model":"rendtest:block/dim"}}]}"#,
        );
        fixture
    }
    fn write(&self, file: &str, json: &str) {
        let path = self.0.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, json).unwrap();
    }
    fn lamp(&self, json: &str) {
        self.write("assets/rendtest/blockstates/lamp.json", json);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `rendtest:lamp` has two states (`powered` false, then true), `rendtest:post` one.
fn registry() -> BlockRegistry {
    let state = |name: &str, powered: Option<&str>| BlockState {
        name: name.into(),
        properties: powered
            .map(|p| BTreeMap::from([("powered".to_owned(), p.to_owned())]))
            .unwrap_or_default()
            .into(),
    };
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(vec![
                state("rendtest:lamp", Some("false")).to_nbt(),
                state("rendtest:lamp", Some("true")).to_nbt(),
                state("rendtest:post", None).to_nbt(),
            ]),
        ),
        (
            "defaults",
            Tag::compound([
                ("rendtest:lamp", Tag::Int(0)),
                ("rendtest:post", Tag::Int(2)),
            ]),
        ),
    ]))
    .unwrap()
}

/// The models each state of the registry draws, as a load of `fixture` bakes them: what a
/// resource load or reload builds.
fn load(fixture: &Fixture, definitions: Option<&BlockStateModelTable>) -> Vec<Vec<String>> {
    let facts: BTreeMap<String, serde_json::Value> = serde_json::from_str(
        r#"{"rendtest:lamp":{"schema":{"powered":{"named":["false","true"]}},
            "states":[{"powered":"false"},{"powered":"true"}]},
            "rendtest:post":{"schema":{},"states":[{}]}}"#,
    )
    .unwrap();
    let source = ResourceSource::directory(fixture.0.clone());
    let mut sprites = Sprites::new(source.clone());
    let catalog = StateModels::from_facts_with_definitions(
        &source,
        &registry(),
        &["rendtest:lamp", "rendtest:post"],
        &mut sprites,
        &facts,
        &ParsedModels::default(),
        definitions,
    )
    .unwrap();
    (0..3)
        .map(|state| {
            catalog
                .collect(BlockStateId(state), BlockPos::new(0, 0, 0))
                .map(|models| models.iter().map(|(v, _)| v.model.clone()).collect())
                .unwrap_or_default()
        })
        .collect()
}

fn names(models: &[&str]) -> Vec<String> {
    models.iter().map(|m| (*m).to_owned()).collect()
}

/// A two-state block picks each state's model through the mod's bound provider; a reload over the
/// edited file picks again; a vanilla multipart block beside it keeps its parts; a file the
/// provider refuses, or one naming a type nobody registered, leaves the block without a model.
#[test]
fn a_registered_type_picks_each_states_model_and_again_on_reload() {
    let fixture = Fixture::new("reload");
    let post = names(&["rendtest:block/post", "rendtest:block/dim"]);
    let none = Vec::new();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("lamps", Box::new(Lamps))].into();
    let (definitions, kept) = mod_platform::testing::with_mods(mods, || {
        let mut definitions = Vec::new();
        mod_api::dispatch_register_block_state_models(
            &RegistrationContext::new(),
            &mut definitions,
        );
        freeze(definitions)
    });
    assert_eq!(kept, ["rendtest:by_power"]);
    fixture.lamp(
        r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/on","off":"rendtest:block/off"}"#,
    );
    assert_eq!(
        load(&fixture, Some(&definitions)),
        [
            names(&["rendtest:block/off"]),
            names(&["rendtest:block/on"]),
            post.clone()
        ]
    );
    // A pack edit, then a reload: the powered lamp is dim now.
    fixture.lamp(
        r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/dim","off":"rendtest:block/off"}"#,
    );
    assert_eq!(
        load(&fixture, Some(&definitions)),
        [
            names(&["rendtest:block/off"]),
            names(&["rendtest:block/dim"]),
            post.clone()
        ]
    );
    // Malformed for the type: refused.
    fixture.lamp(r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/on"}"#);
    assert_eq!(
        load(&fixture, Some(&definitions)),
        [none.clone(), none.clone(), post.clone()]
    );
    // A type no mod registered: refused.
    fixture.lamp(
        r#"{"neoforge:definition_type":"rendtest:never","on":"rendtest:block/on","off":"rendtest:block/off"}"#,
    );
    assert_eq!(
        load(&fixture, Some(&definitions)),
        [none.clone(), none.clone(), post.clone()]
    );
    // With no bound provider installed: refused.
    fixture.lamp(
        r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/on","off":"rendtest:block/off"}"#,
    );
    assert_eq!(
        load(&fixture, installed_definitions()),
        [none.clone(), none, post]
    );
}

/// A pack's typed layer over vanilla's variants replaces the states it picks; the others keep the
/// layer below, as stacked vanilla definitions do.
#[test]
fn a_typed_layer_over_vanilla_replaces_only_the_states_it_picks() {
    struct PoweredOnly;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PoweredOnly {
        fn register_block_state_models(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterBlockStateModelsEvent<'_>,
        ) {
            event.register(Identifier::parse("rendtest:powered_only").unwrap(), |_| {
                Ok(vec![pick(1, "rendtest:block/on")])
            });
        }
    }
    let fixture = Fixture::new("layers");
    let stacked = serde_json::json!({ crate::packs::BLOCKSTATE_LAYERS: [
        {"variants":{"":{"model":"rendtest:block/off"}}},
        {"neoforge:definition_type":"rendtest:powered_only"}
    ]});
    fixture.lamp(&stacked.to_string());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("on", Box::new(PoweredOnly))].into();
    let (definitions, kept) = mod_platform::testing::with_mods(mods, || {
        let mut definitions = Vec::new();
        mod_api::dispatch_register_block_state_models(
            &RegistrationContext::new(),
            &mut definitions,
        );
        freeze(definitions)
    });
    assert_eq!(kept, ["rendtest:powered_only"]);
    let picked = load(&fixture, Some(&definitions));
    assert_eq!(picked[0], ["rendtest:block/off"]);
    assert_eq!(picked[1], ["rendtest:block/on"]);
}

#[test]
fn minecraft_namespace_is_accepted_and_last_writer_wins() {
    struct Earlier;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Earlier {
        fn register_block_state_models(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterBlockStateModelsEvent<'_>,
        ) {
            event.register(
                Identifier::parse("minecraft:rendtest_override").unwrap(),
                |_| Ok(vec![pick(0, "rendtest:block/off")]),
            );
        }
    }
    struct Later;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Later {
        fn register_block_state_models(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterBlockStateModelsEvent<'_>,
        ) {
            event.register(
                Identifier::parse("minecraft:rendtest_override").unwrap(),
                |_| Ok(vec![pick(0, "rendtest:block/on")]),
            );
        }
    }
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("earlier_bsm", Box::new(Earlier)),
        mod_api::ModEntry::new("later_bsm", Box::new(Later)),
    ]
    .into();
    let (definitions, kept) = mod_platform::testing::with_mods(mods, || {
        let mut definitions = Vec::new();
        mod_api::dispatch_register_block_state_models(
            &RegistrationContext::new(),
            &mut definitions,
        );
        freeze(definitions)
    });
    assert_eq!(kept, ["minecraft:rendtest_override"]);
    let states = [BTreeMap::new()];
    let models = instantiate(
        Some(&definitions),
        "minecraft:rendtest_override",
        "rendtest:lamp",
        &serde_json::json!({}),
        &states,
    )
    .unwrap();
    assert_eq!(
        models[0].as_ref().unwrap().model,
        "rendtest:block/on",
        "last registration replaces earlier one"
    );
}

#[test]
fn panic_disables_provider_with_notice_and_non_panic_err_does_not_latch() {
    let panic_calls = Arc::new(AtomicUsize::new(0));
    let flaky_calls = Arc::new(AtomicUsize::new(0));

    struct Panicker(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Panicker {
        fn register_block_state_models(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterBlockStateModelsEvent<'_>,
        ) {
            let calls = Arc::clone(&self.0);
            event.register(
                Identifier::parse("rendtest:bsm_panics").unwrap(),
                move |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    panic!("bsm boom");
                },
            );
        }
    }

    struct Flaky(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Flaky {
        fn register_block_state_models(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterBlockStateModelsEvent<'_>,
        ) {
            let calls = Arc::clone(&self.0);
            event.register(
                Identifier::parse("rendtest:bsm_flaky").unwrap(),
                move |_| {
                    if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                        Err("missing key".into())
                    } else {
                        Ok(vec![pick(0, "rendtest:block/on")])
                    }
                },
            );
        }
    }

    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("bsm_panicker", Box::new(Panicker(Arc::clone(&panic_calls)))),
        mod_api::ModEntry::new("bsm_flaky", Box::new(Flaky(Arc::clone(&flaky_calls)))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut definitions = Vec::new();
        mod_api::dispatch_register_block_state_models(
            &RegistrationContext::new(),
            &mut definitions,
        );
        let (definitions, kept) = freeze(definitions);
        assert_eq!(kept, ["rendtest:bsm_panics", "rendtest:bsm_flaky"]);

        let states = [BTreeMap::new()];
        let cfg = serde_json::json!({});
        let load_type =
            |kind| instantiate(Some(&definitions), kind, "rendtest:lamp", &cfg, &states);

        let _ = take_hook_panics();
        let err = load_type("rendtest:bsm_panics").unwrap_err();
        assert!(err.contains("bsm_panicker"), "{err:?}");
        assert_eq!(panic_calls.load(Ordering::Relaxed), 1);
        let notices = take_hook_panics();
        assert!(
            notices.iter().any(|notice| notice.mod_id == "bsm_panicker"
                && notice.hook == "instantiate_block_state_definition"),
            "expected panic notice, got {notices:?}"
        );
        // Disabled after panic: second call does not invoke the callback again.
        let err2 = load_type("rendtest:bsm_panics").unwrap_err();
        assert!(err2.contains("disabled"), "{err2:?}");
        assert_eq!(panic_calls.load(Ordering::Relaxed), 1);

        // Non-panic Err does not latch or queue a panic notice.
        let err = load_type("rendtest:bsm_flaky").unwrap_err();
        assert!(
            err.contains("bsm_flaky") && err.contains("missing key"),
            "{err:?}"
        );
        assert_eq!(flaky_calls.load(Ordering::Relaxed), 1);
        assert!(take_hook_panics().is_empty());
        let recovered = load_type("rendtest:bsm_flaky").unwrap();
        assert_eq!(flaky_calls.load(Ordering::Relaxed), 2);
        assert_eq!(recovered[0].as_ref().unwrap().model, "rendtest:block/on");
    });
}
