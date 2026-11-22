//! The model loaders native mods register (NeoForge's `ModelEvent.RegisterLoaders`), bound at
//! registration time so a resource load resolves each model's loader directly.
use mod_api::{
    __private::game::{CallbackOwner, CallbackValue, ModelLoaderRegistration},
    Answered, ModelLoaderInput, ModelLoaderQuad, ModelLoaderSchema, ModelLoaderTexture,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
use sunrise_assets::{
    model::{CuboidModel, Direction as FaceDirection},
    model_loaders::{LoaderProvider, LoaderQuad, LoaderSchema, LoaderTexture},
};
use sunrise_core::geometry::Direction;

pub(crate) type ModelLoaderTable = BTreeMap<String, LoaderProvider>;

static LOADERS: OnceLock<ModelLoaderTable> = OnceLock::new();

fn freeze(loaders: Vec<Answered<ModelLoaderRegistration>>) -> (ModelLoaderTable, Vec<String>) {
    let mut bound = BTreeMap::new();
    let mut kept = Vec::new();
    for Answered { answer, mod_id, .. } in loaders {
        let key = answer.id.to_string();
        let owner = if matches!(
            key.as_str(),
            "neoforge:composite" | "neoforge:obj" | "fusion:model"
        ) {
            log::error!("Mod {mod_id}: model loader {key} collides with a built-in model loader");
            None
        } else if bound.contains_key(&key) {
            log::error!(
                "Mod {mod_id}: model loader {key} is already registered; the first registration is kept"
            );
            None
        } else {
            let owner = CallbackOwner::new(mod_id);
            if owner.is_none() {
                log::error!("Model loader {key} has no installed owner {mod_id}");
            }
            owner
        };
        let Some(owner) = owner else {
            CallbackOwner::for_mod(mod_id).dispose("model loader", "drop_baker", answer.bake);
            continue;
        };
        let schema = match answer.schema {
            ModelLoaderSchema::Custom => LoaderSchema::Custom,
            ModelLoaderSchema::ModelProperties => LoaderSchema::ModelProperties,
            ModelLoaderSchema::Cuboids => LoaderSchema::Cuboids,
        };
        let loader = key.clone();
        let callback = CallbackValue::new(owner, "model loader", "drop_baker", answer.bake);
        let bake = Arc::new(
            move |model: &str,
                  config: &str,
                  textures: &BTreeMap<String, String>|
                  -> Result<Vec<LoaderQuad>, String> {
                let input = ModelLoaderInput::new(model, config, textures);
                let owner = callback.owner();
                let quads = owner
                    .call("model loader", "bake_model_loader", || {
                        (callback.value())(&input)
                    })
                    .map_err(|reason| format!("loader {loader}: {reason}"))?
                    .map_err(|reason| {
                        format!("mod {} loader {loader} refused it: {reason}", owner.id())
                    })?;
                Ok(checked(&loader, model, quads))
            },
        );
        bound.insert(key.clone(), LoaderProvider { schema, bake });
        kept.push(key);
    }
    (bound, kept)
}

/// Binds the model loaders native mods registered once at start, before resources load. Any valid
/// `Identifier` (including `minecraft:` and empty-path ids) is accepted; built-in loader
/// collisions (`neoforge:composite`, `neoforge:obj`, `fusion:model`) and duplicates are logged as errors and left
/// out. Returns the ids kept, in registration order.
pub fn register(loaders: Vec<Answered<ModelLoaderRegistration>>) -> Vec<String> {
    let (bound, kept) = freeze(loaders);
    if LOADERS.set(bound).is_err() {
        log::warn!("model loaders were registered already; these are left out");
        return Vec::new();
    }
    kept
}

pub(crate) fn installed_loaders() -> Option<&'static ModelLoaderTable> {
    LOADERS.get()
}

/// The client selects a provider before decoding the model's fields.
pub(crate) fn decode_model(text: &str) -> Result<CuboidModel, sunrise_assets::DecodeError> {
    decode_model_with_loaders(text, installed_loaders())
}

pub(crate) fn decode_model_with_loaders(
    text: &str,
    loaders: Option<&ModelLoaderTable>,
) -> Result<CuboidModel, sunrise_assets::DecodeError> {
    CuboidModel::from_json_with_loaders(text, &|id| loaders?.get(id).cloned())
}

/// The quads a mod answered, as the baker takes them: a quad with a position or UV that isn't
/// finite is skipped by itself.
fn checked(loader: &str, model: &str, quads: Vec<ModelLoaderQuad>) -> Vec<LoaderQuad> {
    quads
        .into_iter()
        .filter_map(|quad| {
            let finite = quad
                .positions
                .iter()
                .flatten()
                .chain(quad.uvs.iter().flatten())
                .all(|value| value.is_finite());
            if !finite {
                log::warn!("model {model}: loader {loader}: a quad that isn't finite; skipped");
                return None;
            }
            Some(LoaderQuad {
                positions: quad.positions,
                uvs: quad.uvs,
                texture: match quad.texture {
                    ModelLoaderTexture::Slot(slot) => LoaderTexture::Slot(slot),
                    ModelLoaderTexture::Sprite(sprite) => LoaderTexture::Sprite(sprite.to_string()),
                },
                cull: quad.cull.map(face),
                tint_index: quad.tint_index,
                shade: quad.shade,
            })
        })
        .collect()
}

fn face(direction: Direction) -> FaceDirection {
    match direction {
        Direction::Down => FaceDirection::Down,
        Direction::Up => FaceDirection::Up,
        Direction::North => FaceDirection::North,
        Direction::South => FaceDirection::South,
        Direction::West => FaceDirection::West,
        Direction::East => FaceDirection::East,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_library::ParsedModels;
    use mod_api::{
        __private::{game::RegistrationContextNew, take_hook_panics},
        Identifier, RegistrationContext,
    };
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_assets::{
        bake::{BlockModelTransform, bake_block_model},
        resource_source::{Resource, ResourceSource},
    };

    fn quad(texture: &str, x: f32) -> ModelLoaderQuad {
        ModelLoaderQuad {
            positions: [
                [x, 0.0, 0.5],
                [0.0, 0.0, 0.5],
                [0.0, 1.0, 0.5],
                [x, 1.0, 0.5],
            ],
            uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
            texture: ModelLoaderTexture::Slot(texture.into()),
            cull: Some(Direction::Up),
            tint_index: -1,
            shade: true,
        }
    }

    #[test]
    fn large_quad_counts_over_65536_are_accepted() {
        let many = vec![quad("side", 1.0); 70_000];
        assert_eq!(checked("l", "m", many).len(), 70_000);
    }

    #[test]
    fn a_quad_that_isnt_finite_is_skipped_alone() {
        let mut uv_nan = quad("side", 1.0);
        uv_nan.uvs[2][1] = f32::NAN;
        let kept = checked(
            "l",
            "m",
            vec![quad("side", f32::INFINITY), uv_nan, quad("#side", 0.5)],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].positions[0][0], 0.5);
        assert_eq!(kept[0].texture, LoaderTexture::Slot("#side".into()));
        assert_eq!(kept[0].cull, Some(FaceDirection::Up));
    }

    /// A native mod's loader: a quad as wide as the model's `width`, in sixteenths.
    struct Panel;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Panel {
        fn register_model_loaders(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterModelLoadersEvent<'_>,
        ) {
            event.register(
                Identifier::parse("rendtest:panel").unwrap(),
                ModelLoaderSchema::ModelProperties,
                |input| {
                    #[derive(serde::Deserialize)]
                    struct PanelConfig {
                        elements: PanelElements,
                    }
                    #[derive(serde::Deserialize)]
                    struct PanelElements {
                        width: u8,
                    }
                    let config: PanelConfig =
                        serde_json::from_str(input.config).map_err(|e| e.to_string())?;
                    assert_eq!(
                        input.textures.get("side").map(String::as_str),
                        Some("minecraft:block/panel")
                    );
                    Ok(vec![quad("#side", f32::from(config.elements.width) / 16.0)])
                },
            );
        }
    }

    /// The provider travels with the load, even on another thread. It is not visible to an
    /// ordinary load of the same file. A reload with the provider reads the edited model.
    #[test]
    fn a_mods_loader_bakes_its_quads_and_answers_again_on_reload() {
        let source = |width: u8| {
            let json = format!(
                r#"{{"loader":"rendtest:panel","elements":{{"width":{width}}},"textures":{{"side":"block/panel"}}}}"#
            );
            ResourceSource::indexed(BTreeMap::from([(
                "assets/t/models/panel.json".into(),
                Resource::bytes(json.into_bytes()),
            )]))
        };
        let widths = |parsed: ParsedModels, source: &ResourceSource| {
            let library = parsed
                .block_library(source, vec!["t:panel".into()])
                .unwrap();
            bake_block_model(
                &library.resolve("t:panel").unwrap(),
                BlockModelTransform::new([0; 3], false),
            )
            .unwrap()
            .iter()
            .map(|q| (q.positions[0][0], q.material.sprite.clone()))
            .collect::<Vec<_>>()
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("panel", Box::new(Panel))].into();
        let (loaders, kept) = mod_platform::testing::with_mods(mods, || {
            let mut loaders = Vec::new();
            mod_api::dispatch_register_model_loaders(&RegistrationContext::new(), &mut loaders);
            freeze(loaders)
        });
        assert_eq!(kept, ["rendtest:panel"]);

        let initial = source(8);
        let start = Barrier::new(2);
        let (custom, missing) = std::thread::scope(|scope| {
            let custom = scope.spawn(|| {
                start.wait();
                widths(
                    ParsedModels::load_with_loaders(&initial, Some(&loaders)),
                    &initial,
                )
            });
            start.wait();
            let missing = widths(ParsedModels::load(&initial), &initial);
            (custom.join().unwrap(), missing)
        });
        assert_eq!(custom, [(0.5, "minecraft:block/panel".to_owned())]);
        assert_eq!(missing.len(), 6);
        assert!(
            missing
                .iter()
                .all(|(_, sprite)| sprite == "minecraft:missingno")
        );

        let reloaded = source(4);
        assert_eq!(
            widths(
                ParsedModels::load_with_loaders(&reloaded, Some(&loaders)),
                &reloaded
            ),
            [(0.25, "minecraft:block/panel".to_owned())]
        );
    }

    #[test]
    fn minecraft_namespace_is_accepted_and_duplicates_and_builtins_are_rejected() {
        struct First;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for First {
            fn register_model_loaders(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterModelLoadersEvent<'_>,
            ) {
                event.register(
                    Identifier::parse("minecraft:rendtest_first").unwrap(),
                    ModelLoaderSchema::ModelProperties,
                    |_| Ok(vec![quad("side", 0.25)]),
                );
                event.register(
                    Identifier::parse("rendtest:").unwrap(),
                    ModelLoaderSchema::ModelProperties,
                    |_| Ok(vec![quad("side", 0.5)]),
                );
                event.register(
                    Identifier::parse("neoforge:composite").unwrap(),
                    ModelLoaderSchema::ModelProperties,
                    |_| Ok(vec![quad("side", 1.0)]),
                );
                event.register(
                    Identifier::parse("neoforge:obj").unwrap(),
                    ModelLoaderSchema::ModelProperties,
                    |_| Ok(vec![quad("side", 1.0)]),
                );
            }
        }
        struct Second;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Second {
            fn register_model_loaders(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterModelLoadersEvent<'_>,
            ) {
                event.register(
                    Identifier::parse("minecraft:rendtest_first").unwrap(),
                    ModelLoaderSchema::ModelProperties,
                    |_| Ok(vec![quad("side", 0.75)]),
                );
            }
        }
        let mods: mod_api::Mods = vec![
            mod_api::ModEntry::new("first_loader", Box::new(First)),
            mod_api::ModEntry::new("second_loader", Box::new(Second)),
        ]
        .into();
        let (loaders, kept) = mod_platform::testing::with_mods(mods, || {
            let mut loaders = Vec::new();
            mod_api::dispatch_register_model_loaders(&RegistrationContext::new(), &mut loaders);
            freeze(loaders)
        });
        assert_eq!(kept, ["minecraft:rendtest_first", "rendtest:"]);
        let textures = BTreeMap::from([("side".to_owned(), "minecraft:block/panel".to_owned())]);
        let baked =
            (loaders["minecraft:rendtest_first"].bake)("t:vanilla_ns", "{}", &textures).unwrap();
        assert_eq!(baked.len(), 1);
        assert_eq!(baked[0].positions[0][0], 0.25, "first registration wins");
        let empty_path = (loaders["rendtest:"].bake)("t:empty_path", "{}", &textures).unwrap();
        assert_eq!(empty_path.len(), 1);
        assert_eq!(empty_path[0].positions[0][0], 0.5);
        assert!(!loaders.contains_key("rendtest:unregistered"));
    }

    #[test]
    fn panic_disables_loader_with_notice_and_non_panic_err_does_not_latch() {
        let panic_calls = Arc::new(AtomicUsize::new(0));
        let flaky_calls = Arc::new(AtomicUsize::new(0));

        struct Panicker(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Panicker {
            fn register_model_loaders(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterModelLoadersEvent<'_>,
            ) {
                let calls = Arc::clone(&self.0);
                event.register(
                    Identifier::parse("rendtest:panics").unwrap(),
                    ModelLoaderSchema::Custom,
                    move |_| {
                        calls.fetch_add(1, Ordering::Relaxed);
                        panic!("boom");
                    },
                );
            }
        }

        struct Flaky(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Flaky {
            fn register_model_loaders(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterModelLoadersEvent<'_>,
            ) {
                let calls = Arc::clone(&self.0);
                event.register(
                    Identifier::parse("rendtest:flaky").unwrap(),
                    ModelLoaderSchema::Custom,
                    move |_| {
                        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                            Err("bad config".into())
                        } else {
                            Ok(vec![quad("side", 0.5)])
                        }
                    },
                );
            }
        }

        let mods: mod_api::Mods = vec![
            mod_api::ModEntry::new(
                "loader_panicker",
                Box::new(Panicker(Arc::clone(&panic_calls))),
            ),
            mod_api::ModEntry::new("loader_flaky", Box::new(Flaky(Arc::clone(&flaky_calls)))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut loaders = Vec::new();
            mod_api::dispatch_register_model_loaders(&RegistrationContext::new(), &mut loaders);
            let (loaders, kept) = freeze(loaders);
            assert_eq!(kept, ["rendtest:panics", "rendtest:flaky"]);

            let textures =
                BTreeMap::from([("side".to_owned(), "minecraft:block/panel".to_owned())]);
            let panics = &loaders["rendtest:panics"].bake;
            let flaky = &loaders["rendtest:flaky"].bake;
            let _ = take_hook_panics();
            assert!(panics("t:p", "{}", &textures).is_err());
            assert_eq!(panic_calls.load(Ordering::Relaxed), 1);
            let notices = take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.mod_id == "loader_panicker"
                        && notice.hook == "bake_model_loader"),
                "expected panic notice, got {notices:?}"
            );
            // Disabled after panic: second call does not invoke the callback again.
            assert!(panics("t:p", "{}", &textures).is_err());
            assert_eq!(panic_calls.load(Ordering::Relaxed), 1);

            // Non-panic Err does not latch or queue a panic notice.
            assert!(flaky("t:f", "{}", &textures).is_err());
            assert_eq!(flaky_calls.load(Ordering::Relaxed), 1);
            assert!(take_hook_panics().is_empty());
            let recovered = flaky("t:f", "{}", &textures).unwrap();
            assert_eq!(flaky_calls.load(Ordering::Relaxed), 2);
            assert_eq!(recovered.len(), 1);
            assert_eq!(recovered[0].positions[0][0], 0.5);
        });
    }
}
