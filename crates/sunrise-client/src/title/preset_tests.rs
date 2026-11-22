#[test]
fn bundled_mod_world_presets_do_not_require_a_repository_pack_selection() {
    let mut f = Fixture::new();
    let data = f.root.join("mod-data/data/example/worldgen/world_preset");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("passive.json"), r#"{"dimensions":{"minecraft:overworld":{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}}}"#).unwrap();
    let tags = f
        .root
        .join("mod-data/data/minecraft/tags/worldgen/world_preset");
    fs::create_dir_all(&tags).unwrap();
    fs::write(
        tags.join("normal.json"),
        r#"{"values":["example:passive"]}"#,
    )
    .unwrap();
    f.title
        .set_preset_data_source(crate::resource_source::ResourceSource::directory(
            f.root.join("mod-data"),
        ));
    f.title.enter_singleplayer();
    let Mode::Create { options, .. } = &mut f.title.mode else {
        panic!("create")
    };
    assert_eq!(options.data_packs, ["vanilla"]);
    options.cycle_preset(-1);
    assert_eq!(options.preset_id(), "example:passive");
    assert!(options.dimensions().is_some());
}

#[test]
fn an_empty_normal_world_preset_tag_falls_back_to_all_registry_presets() {
    let mut f = Fixture::new();
    let data = f.root.join("mod-data/data/example/worldgen/world_preset");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("islands.json"), r#"{"dimensions":{"minecraft:overworld":{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}}}"#).unwrap();
    let tags = f
        .root
        .join("mod-data/data/minecraft/tags/worldgen/world_preset");
    fs::create_dir_all(&tags).unwrap();
    fs::write(tags.join("normal.json"), r#"{"replace":true,"values":[]}"#).unwrap();
    f.title
        .set_preset_data_source(crate::resource_source::ResourceSource::directory(
            f.root.join("mod-data"),
        ));
    f.title.enter_singleplayer();
    let Mode::Create { options, .. } = &mut f.title.mode else {
        panic!("create")
    };
    assert_eq!(
        options.preset_order.len(),
        crate::save::WorldPreset::ALL.len() + 2
    );
    for preset in crate::save::WorldPreset::ALL {
        assert!(options.preset_order.iter().any(|id| id == preset.id()));
    }
    options.cycle_preset(-1);
    assert_eq!(options.preset_id(), "example:islands");
    options.cycle_preset(-1);
    assert_eq!(options.preset_id(), "minecraft:debug_all_block_states");
    assert_eq!(options.world_type, crate::save::WorldPreset::Debug);
    options.cycle_preset(1);
    assert_eq!(options.preset_id(), "example:islands");
    options.cycle_preset(1);
    assert_eq!(options.preset_id(), "minecraft:normal");
}

#[test]
fn recreating_a_debug_world_preset_keeps_its_hidden_type() {
    let mut f = Fixture::new();
    f.world("Debug", "Debug");
    crate::save::write_world_gen_settings(
        &f.title.saves.join("Debug"),
        123456,
        crate::save::WorldGen {
            preset: crate::save::WorldPreset::Debug,
            ..Default::default()
        },
    )
    .unwrap();
    f.select();
    f.click("Re-Create");
    let Mode::Create { options, .. } = &f.title.mode else {
        panic!("create")
    };
    assert_eq!(options.preset_id(), "minecraft:debug_all_block_states");
    assert!(
        !options
            .preset_order
            .iter()
            .any(|id| id == "minecraft:debug_all_block_states")
    );
    let Action::Create { options, .. } = f.title.create_action() else {
        panic!("create action")
    };
    assert_eq!(options.world_gen().preset, crate::save::WorldPreset::Debug);
}

#[test]
fn world_preset_narration_matches_the_cycle_and_only_hints_actual_amplified() {
    let mut options = WorldOptions {
        custom_preset: Some("example:islands".into()),
        world_type: crate::save::WorldPreset::Amplified,
        preset_order: vec![
            "minecraft:normal".into(),
            "example:islands".into(),
            "minecraft:flat".into(),
        ],
        tab: 1,
        ..Default::default()
    };
    let button = create_world::buttons(480.0, 262.0, false, &options, None)
        .into_iter()
        .find(|b| b.id == 25)
        .unwrap();
    let narration = create_world::button_narration(&button, false, &options);
    assert!(narration.hint.is_none());
    let crate::widgets::What::Cycle { next, .. } = narration.what else {
        panic!("cycle")
    };
    assert_eq!(next, lang::get("generator.minecraft.flat"));
    options.cycle_preset(1);
    assert_eq!(options.preset_id(), "minecraft:flat");
    options.custom_preset = None;
    options.world_type = crate::save::WorldPreset::Amplified;
    assert!(
        create_world::button_narration(&button, false, &options)
            .hint
            .is_some()
    );
}

#[test]
fn registered_world_preset_editor_commits_only_on_done_and_saves_its_dimensions() {
    use mod_api::{
        __private::game::RegistrationContextNew, Hooks, Identifier, ModEntry, PresetEditor,
        PresetEditorDraft, RegisterPresetEditorsEvent, RegistrationContext,
    };

    struct TitledEditor(&'static str);
    impl PresetEditor for TitledEditor {
        fn title(&self) -> Option<String> {
            Some(self.0.into())
        }
        fn key_pressed(
            &mut self,
            key: u32,
            _scancode: u32,
            _modifiers: u32,
            draft: &mut PresetEditorDraft<'_>,
        ) -> bool {
            if key != 44 {
                return false;
            }
            assert!(!draft.set_dimensions(Default::default()));
            let mut edited = draft.dimensions().clone();
            if let Some(Tag::Compound(overworld)) = edited.get_mut(&"minecraft:overworld".into()) {
                overworld.insert(
                    "type".into(),
                    Tag::String("minecraft:overworld_caves".into()),
                );
            }
            assert!(draft.set_dimensions(edited));
            true
        }
    }

    struct FirstMod;
    #[mod_api::__subscriptions]
    impl Hooks for FirstMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            event.register(Identifier::parse("example:islands").unwrap(), |_| {
                Ok(Box::new(TitledEditor("First Islands Editor")))
            });
        }
    }

    struct SecondMod;
    #[mod_api::__subscriptions]
    impl Hooks for SecondMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            event.register(Identifier::parse("example:islands").unwrap(), |_| {
                Ok(Box::new(TitledEditor("Winning Islands Editor")))
            });
        }
    }

    let mods: mod_api::Mods = vec![
        ModEntry::new("first", Box::new(FirstMod)),
        ModEntry::new("second", Box::new(SecondMod)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut f = Fixture::new();
        let data = f.root.join("pack/data/example/worldgen/world_preset");
        fs::create_dir_all(&data).unwrap();
        let tag_dir = f
            .root
            .join("pack/data/minecraft/tags/worldgen/world_preset");
        fs::create_dir_all(&tag_dir).unwrap();
        fs::write(
            tag_dir.join("normal.json"),
            r#"{"values":["example:islands"]}"#,
        )
        .unwrap();
        let dimensions = serde_json::json!({"minecraft:overworld": {
            "type": "minecraft:overworld", "generator": {
                "type": "minecraft:flat", "settings": {
                    "biome": "minecraft:plains", "layers": [{"block": "minecraft:stone", "height": 5}],
                    "features": false, "lakes": false
                }
            }
        }});
        fs::write(
            data.join("islands.json"),
            serde_json::json!({"dimensions": dimensions}).to_string(),
        )
        .unwrap();
        fs::write(
            data.join("hidden.json"),
            serde_json::json!({"dimensions": dimensions}).to_string(),
        )
        .unwrap();
        let pack = crate::datapack::DataPack::from_source(
            "example".into(),
            crate::resource_source::ResourceSource::directory(f.root.join("pack")),
        );
        f.title
            .set_additional_data_packs(vec![std::sync::Arc::new(pack)].into());
        f.title.enter_singleplayer();
        f.title
            .set_data_packs(vec!["vanilla".into(), "example".into()]);

        let mut editors = Vec::new();
        mod_api::dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
        f.title
            .set_preset_editor_factories(PresetEditorFactories::bind(editors));

        let Mode::Create { options, .. } = &mut f.title.mode else {
            panic!("create")
        };
        options.tab = 1;
        // Five vanilla presets then the pack's one custom preset.
        for _ in 0..5 {
            options.cycle_preset(1);
        }
        assert_eq!(options.preset_id(), "example:islands");
        assert!(!options.preset_order.iter().any(|id| id == "example:hidden"));
        let original = options.dimensions().unwrap();
        assert!(
            !f.title
                .buttons(480.0, 262.0)
                .iter()
                .find(|b| b.id == 26)
                .unwrap()
                .enabled
        );
        f.title
            .set_preset_data_source(crate::resource_source::ResourceSource::directory(
                f.root.join("no-mods"),
            ));
        let click = |title: &mut Title, id, size: (f32, f32)| {
            let button = title
                .buttons(size.0, size.1)
                .into_iter()
                .find(|b| b.id == id)
                .unwrap();
            assert!(button.enabled);
            title.click(button.x + 2.0, button.y + 2.0, size);
        };
        click(&mut f.title, 26, (480.0, 262.0));
        assert_eq!(f.title.mode.id(), "world_preset_editor");
        // Last writer wins: SecondMod overrode FirstMod.
        let session = f.title.preset_editor_session().unwrap();
        assert_eq!(session.preset, "example:islands");
        assert_eq!(session.title(), Some("Winning Islands Editor"));
        assert!(
            !f.title
                .set_preset_editor_dimensions(Tag::Compound(Default::default()))
        );
        let oversized = crate::save::json_tag(&serde_json::json!({"minecraft:overworld": {
            "type": "minecraft:overworld", "generator": {"type": "minecraft:flat", "settings": {
                "layers": [{"block": "minecraft:stone", "height": 1000000000}]
            }}
        }}));
        assert!(!f.title.set_preset_editor_dimensions(oversized));
        assert_eq!(f.title.preset_editor_dimensions(), Some(original.clone()));
        let mut edited = original.clone();
        let Tag::Compound(stems) = &mut edited else {
            unreachable!()
        };
        let Tag::Compound(overworld) = stems.get_mut(&"minecraft:overworld".into()).unwrap() else {
            unreachable!()
        };
        overworld.insert(
            "type".into(),
            Tag::String("minecraft:overworld_caves".into()),
        );
        // Edit through the bound PresetEditor callback and discard via Escape.
        assert!(
            f.title
                .preset_editor_session()
                .unwrap()
                .key_pressed(44, 0, 0)
        );
        assert_eq!(f.title.preset_editor_dimensions(), Some(edited.clone()));
        f.title.escape();
        let Mode::Create { options, .. } = &f.title.mode else {
            panic!("back")
        };
        assert_eq!(options.dimensions(), Some(original));
        assert!(f.title.preset_editor_session().is_none());
        assert!(!f.title.set_preset_editor_dimensions(edited.clone()));
        // Reopen, edit via Title::set_preset_editor_dimensions, and commit via Done.
        f.title.focused_button = Some(26);
        f.title.confirm_key_size((320.0, 240.0));
        assert_eq!(f.title.mode.id(), "world_preset_editor");
        assert!(f.title.set_preset_editor_dimensions(edited.clone()));
        f.title.focused_button = Some(94);
        f.title.confirm_key_size((320.0, 240.0));
        let Mode::Create { options, .. } = &f.title.mode else {
            panic!("back")
        };
        assert_eq!(options.dimensions(), Some(edited.clone()));
        let save = f.root.join("created");
        options.write_world_gen(&save, 123).unwrap();
        assert_eq!(
            crate::save::world_gen_settings(&save)
                .unwrap()
                .get("dimensions"),
            Some(&edited)
        );
        assert_eq!(
            crate::save::world_gen_settings(&save).unwrap().get("seed"),
            Some(&Tag::Long(123))
        );
        let saved = crate::save::world_gen_settings(&save);
        let mut refused = options.clone();
        refused.preset_dimensions = Some(Tag::Compound(Default::default()));
        assert!(refused.write_world_gen(&save, 999).is_err());
        assert_eq!(crate::save::world_gen_settings(&save), saved);
        f.title
            .set_preset_editor_factories(PresetEditorFactories::default());
        assert!(
            !f.title
                .buttons(320.0, 240.0)
                .iter()
                .find(|b| b.id == 26)
                .unwrap()
                .enabled
        );
        f.title.focused_button = Some(26);
        f.title.confirm_key_size((320.0, 240.0));
        assert_eq!(f.title.mode.id(), "world_create");
        f.title.set_data_packs(vec!["vanilla".into()]);
        let Mode::Create { options, .. } = &f.title.mode else {
            panic!("create")
        };
        assert_eq!(options.preset_id(), "minecraft:normal");
        assert!(options.dimensions().is_none());
        assert!(
            !f.title
                .buttons(480.0, 262.0)
                .iter()
                .find(|b| b.id == 26)
                .unwrap()
                .enabled
        );
    });
}

#[test]
fn minecraft_preset_editor_override_and_vanilla_fallback() {
    use mod_api::{
        __private::game::RegistrationContextNew, Hooks, Identifier, ModEntry, PresetEditor,
        RegisterPresetEditorsEvent, RegistrationContext,
    };

    struct CustomFlatEditor;
    impl PresetEditor for CustomFlatEditor {
        fn title(&self) -> Option<String> {
            Some("Custom Flat Override".into())
        }
    }

    struct OverrideMod;
    #[mod_api::__subscriptions]
    impl Hooks for OverrideMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            event.register(Identifier::parse("minecraft:flat").unwrap(), |_| {
                Ok(Box::new(CustomFlatEditor))
            });
        }
    }

    let mods: mod_api::Mods = vec![ModEntry::new("override_mod", Box::new(OverrideMod))].into();
    mod_platform::testing::with_mods(mods, || {
        let mut editors = Vec::new();
        mod_api::dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
        let mut f = Fixture::new();
        f.title
            .set_preset_editor_factories(PresetEditorFactories::bind(editors));
        f.title
            .set_preset_data_source(crate::resource_source::ResourceSource::directory(
                f.root.join("no-mods"),
            ));
        f.title.enter_singleplayer();
        {
            let Mode::Create { options, .. } = &mut f.title.mode else {
                panic!("create")
            };
            options.tab = 1;
            options.world_type = crate::save::WorldPreset::Flat;
        }
        f.title.focused_button = Some(26);
        f.title.confirm_key_size((480.0, 262.0));
        assert_eq!(f.title.mode.id(), "world_preset_editor");
        let session = f.title.preset_editor_session().unwrap();
        assert_eq!(session.preset, "minecraft:flat");
        assert_eq!(session.title(), Some("Custom Flat Override"));

        // Cancel returns to Create World; un-overridden single_biome_surface still uses vanilla.
        f.title.focused_button = Some(95);
        f.title.confirm_key_size((480.0, 262.0));
        assert_eq!(f.title.mode.id(), "world_create");
        {
            let Mode::Create { options, .. } = &mut f.title.mode else {
                panic!("create")
            };
            options.world_type = crate::save::WorldPreset::SingleBiomeSurface;
        }
        f.title.focused_button = Some(26);
        f.title.confirm_key_size((480.0, 262.0));
        assert_eq!(f.title.mode.id(), "world_create_buffet");
        f.title.escape();

        // Removing the minecraft:flat override restores vanilla's CreateFlat screen.
        f.title
            .set_preset_editor_factories(PresetEditorFactories::default());
        {
            let Mode::Create { options, .. } = &mut f.title.mode else {
                panic!("create")
            };
            options.world_type = crate::save::WorldPreset::Flat;
        }
        f.title.focused_button = Some(26);
        f.title.confirm_key_size((480.0, 262.0));
        assert_eq!(f.title.mode.id(), "world_create_flat");
    });
}

#[test]
fn preset_editor_factory_refusal_does_not_latch_and_panics_disable_callbacks() {
    use mod_api::{
        __private::game::RegistrationContextNew, Hooks, Identifier, ModEntry, PresetEditor,
        PresetEditorDraft, RegisterPresetEditorsEvent, RegistrationContext,
    };
    use std::sync::Arc;

    struct OkEditor;
    impl PresetEditor for OkEditor {
        fn title(&self) -> Option<String> {
            Some("Recovered".into())
        }
    }

    struct PanickingCallbackEditor {
        calls: Arc<AtomicUsize>,
    }
    impl PresetEditor for PanickingCallbackEditor {
        fn key_pressed(
            &mut self,
            _key: u32,
            _scancode: u32,
            _modifiers: u32,
            draft: &mut PresetEditorDraft<'_>,
        ) -> bool {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut edited = draft.dimensions().clone();
            if let Some(Tag::Compound(overworld)) = edited.get_mut(&"minecraft:overworld".into()) {
                overworld.insert(
                    "type".into(),
                    Tag::String("minecraft:overworld_caves".into()),
                );
            }
            let _ = draft.set_dimensions(edited);
            panic!("editor callback boom");
        }
    }

    struct RefusingMod {
        attempts: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for RefusingMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            let attempts = Arc::clone(&self.attempts);
            event.register(Identifier::parse("example:flaky").unwrap(), move |_| {
                if attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                    Err("temporary refusal".into())
                } else {
                    Ok(Box::new(OkEditor))
                }
            });
        }
    }

    struct PanickingFactoryMod {
        calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for PanickingFactoryMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            let calls = Arc::clone(&self.calls);
            event.register(
                Identifier::parse("example:factory_boom").unwrap(),
                move |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    panic!("factory boom");
                },
            );
        }
    }

    struct PanickingEditorMod {
        calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for PanickingEditorMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            let calls = Arc::clone(&self.calls);
            event.register(
                Identifier::parse("example:editor_boom").unwrap(),
                move |_| {
                    Ok(Box::new(PanickingCallbackEditor {
                        calls: Arc::clone(&calls),
                    }))
                },
            );
        }
    }

    let refusing_attempts = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let editor_calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![
        ModEntry::new(
            "refusing",
            Box::new(RefusingMod {
                attempts: Arc::clone(&refusing_attempts),
            }),
        ),
        ModEntry::new(
            "panicking_factory",
            Box::new(PanickingFactoryMod {
                calls: Arc::clone(&factory_calls),
            }),
        ),
        ModEntry::new(
            "panicking_editor",
            Box::new(PanickingEditorMod {
                calls: Arc::clone(&editor_calls),
            }),
        ),
    ]
    .into();

    mod_platform::testing::with_mods(mods, || {
        let mut editors = Vec::new();
        mod_api::dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
        let factories = PresetEditorFactories::bind(editors);

        let Tag::Compound(dimensions) = Tag::compound([(
            "minecraft:overworld",
            Tag::compound([
                ("type", Tag::String("minecraft:overworld".into())),
                (
                    "generator",
                    Tag::compound([("type", Tag::String("minecraft:debug".into()))]),
                ),
            ]),
        )]) else {
            unreachable!()
        };

        // Non-panic factory errors do not latch.
        assert_eq!(
            factories
                .open_session("example:flaky", dimensions.clone(), "7")
                .unwrap_err(),
            "temporary refusal"
        );
        let recovered = factories
            .open_session("example:flaky", dimensions.clone(), "7")
            .unwrap()
            .unwrap();
        assert_eq!(recovered.title(), Some("Recovered"));
        assert_eq!(refusing_attempts.load(Ordering::Relaxed), 2);

        // Check the local owner's result and call count, not the shared notice queue.
        assert_eq!(
            factories
                .open_session("example:factory_boom", dimensions.clone(), "7")
                .unwrap_err(),
            "mod panicking_factory: preset editor callback panicked; it is disabled: factory boom"
        );
        assert_eq!(
            factories
                .open_session("example:factory_boom", dimensions.clone(), "7")
                .unwrap_err(),
            "mod panicking_factory: preset editor owner is disabled"
        );
        assert_eq!(factory_calls.load(Ordering::Relaxed), 1);

        // Editor callback panic disables the bound owner and preserves the prior draft.
        let session = factories
            .open_session("example:editor_boom", dimensions.clone(), "7")
            .unwrap()
            .unwrap();
        assert!(!session.key_pressed(44, 0, 0));
        assert_eq!(session.dimensions(), Tag::Compound(dimensions.clone()));
        assert!(!session.key_pressed(44, 0, 0));
        assert_eq!(editor_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            factories
                .open_session("example:editor_boom", dimensions, "7")
                .unwrap_err(),
            "mod panicking_editor: preset editor owner is disabled"
        );
    });
}

#[test]
fn preset_editor_destructor_is_contained_on_last_session_drop_and_title_failure() {
    use mod_api::{
        __private::game::RegistrationContextNew, Hooks, Identifier, ModEntry, PresetEditor,
        RegisterPresetEditorsEvent, RegistrationContext,
    };
    use std::sync::Arc;

    struct PanickingDropEditor {
        drops: Arc<AtomicUsize>,
        fail_title: bool,
    }
    impl PresetEditor for PanickingDropEditor {
        fn title(&self) -> Option<String> {
            assert!(!self.fail_title, "title boom");
            Some("Live editor".into())
        }
    }
    impl Drop for PanickingDropEditor {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
            panic!("editor destructor boom");
        }
    }

    struct DroppingMod {
        preset: &'static str,
        drops: Arc<AtomicUsize>,
        fail_title: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for DroppingMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            let drops = Arc::clone(&self.drops);
            let fail_title = self.fail_title;
            event.register(Identifier::parse(self.preset).unwrap(), move |_| {
                Ok(Box::new(PanickingDropEditor {
                    drops: Arc::clone(&drops),
                    fail_title,
                }))
            });
        }
    }

    struct HealthyEditor;
    impl PresetEditor for HealthyEditor {
        fn title(&self) -> Option<String> {
            Some("Healthy editor".into())
        }
    }

    struct HealthyMod;
    #[mod_api::__subscriptions]
    impl Hooks for HealthyMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            event.register(Identifier::parse("example:healthy").unwrap(), |_| {
                Ok(Box::new(HealthyEditor))
            });
        }
    }

    let live_drops = Arc::new(AtomicUsize::new(0));
    let failed_drops = Arc::new(AtomicUsize::new(0));
    let mods = vec![
        ModEntry::new(
            "editor_drop_panics",
            Box::new(DroppingMod {
                preset: "example:drop_panics",
                drops: Arc::clone(&live_drops),
                fail_title: false,
            }),
        ),
        ModEntry::new(
            "editor_title_panics",
            Box::new(DroppingMod {
                preset: "example:title_panics",
                drops: Arc::clone(&failed_drops),
                fail_title: true,
            }),
        ),
        ModEntry::new("editor_healthy", Box::new(HealthyMod)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut editors = Vec::new();
        mod_api::dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
        let factories = PresetEditorFactories::bind(editors);
        let open = |preset: &str| factories.open_session(preset, Default::default(), "7");

        let session = open("example:drop_panics").unwrap().unwrap();
        drop(session.clone());
        assert_eq!(live_drops.load(Ordering::Relaxed), 0);
        drop(session);
        assert_eq!(live_drops.load(Ordering::Relaxed), 1);
        assert_eq!(
            open("example:drop_panics").unwrap_err(),
            "mod editor_drop_panics: preset editor owner is disabled"
        );

        assert_eq!(
            open("example:title_panics").unwrap_err(),
            "mod editor_title_panics: preset editor callback panicked; it is disabled: title boom"
        );
        assert_eq!(failed_drops.load(Ordering::Relaxed), 1);
        assert_eq!(
            open("example:title_panics").unwrap_err(),
            "mod editor_title_panics: preset editor owner is disabled"
        );
        assert_eq!(failed_drops.load(Ordering::Relaxed), 1);

        let healthy = open("example:healthy").unwrap().unwrap();
        assert_eq!(healthy.title(), Some("Healthy editor"));
    });
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn finish_backup(&mut self) {
        self.title.backup_shown.set(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while matches!(self.title.mode, Mode::Backup { .. }) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.title.poll_backup();
        }
    }
}
