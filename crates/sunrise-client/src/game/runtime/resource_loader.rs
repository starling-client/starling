//! What a start loads before the game takes over: the CPU-only bundle a loader generation builds
//! (`GameCpuResources`, `EntityModels`), what the background loader publishes (`Loaded`,
//! `LoaderMsg`), the world's rest loader and a pending world entry, the ordinary launch
//! (`run_loading`) and the smoke run's report (`write_report`).
use super::*;

/// CPU-only startup data, owned by one loader generation. No file reads or
/// model baking are needed when the event loop installs this bundle.
pub(super) struct GameCpuResources {
    pub(super) painting_variants: paintings::PaintingVariants,
    pub(super) frame_model: frames::FrameModel,
    pub(super) glow_frame_model: frames::FrameModel,
    pub(super) map_frame_model: frames::FrameModel,
    pub(super) glow_map_frame_model: frames::FrameModel,
    pub(super) armor_stand_assets: armor_stands::ArmorStandAssets,
    pub(super) waypoint_styles: sunrise_client::locator_bar::WaypointStyles,
    pub(super) boat_models: boats::BoatModels,
    pub(super) cart_models: minecarts::CartModels,
    pub(super) crystal_model: Option<sunrise_client::mobs::Part>,
    pub(super) cushion_model: Option<sunrise_client::mobs::Part>,
    pub(super) dragon_model: Option<sunrise_client::mobs::Part>,
    pub(super) arrow_model: Option<sunrise_client::mobs::Part>,
    pub(super) bee_stinger_model: Option<sunrise_client::mobs::Part>,
    pub(super) trident_model: Option<sunrise_client::mobs::Part>,
    pub(super) entity_models: EntityModels,
    pub(super) creative_screen: Creative,
}

/// Baked layers of entities that may first show up deep into a session, plus every
/// layer from `entity-models.json` indexed for `bake_layer`. They come from the
/// document parsed once at startup and pack reload: parsing entity-models.json (2.4 MB)
/// when the first one appeared stalled that frame by 40 ms of CPU and the file read.
#[derive(Default)]
pub(super) struct EntityModels {
    /// LeashKnotModel.
    pub(super) leash_knot: Option<sunrise_client::mobs::Part>,
    /// WindChargeModel.
    pub(super) wind_charge: Option<sunrise_client::mobs::Part>,
    /// ShulkerBulletModel.
    pub(super) shulker_bullet: Option<sunrise_client::mobs::Part>,
    /// SkullModel of the wither skull.
    pub(super) wither_skull: Option<sunrise_client::mobs::Part>,
    /// EvokerFangsModel.
    pub(super) evoker_fangs: Option<Arc<sunrise_client::mobs::Part>>,
    /// BannerModel's pole and flag (a raid captain's banner).
    pub(super) banner: Option<(sunrise_client::mobs::Part, sunrise_client::mobs::Part)>,
    /// All layers indexed from `entity-models.json` at client resource load and reload
    /// (matching vanilla `EntityModelSet.bakeLayer`).
    pub(super) layers: sunrise_client::fast_hash::FxHashMap<String, mod_api::ModelPart>,
}

impl EntityModels {
    pub(super) fn load(document: Option<&sunrise_client::mobs::EntityModelDocument>) -> Self {
        let model = |key| document.and_then(|document| document.model(key));
        let mut layers = sunrise_client::fast_hash::FxHashMap::default();
        if let Some(document) = document {
            for key in document.keys() {
                if let Some(part) = document.model(key) {
                    layers.insert(
                        key.to_owned(),
                        sunrise_client::mobs::model_layers::from_part(&part),
                    );
                }
            }
        }
        Self {
            leash_knot: model("minecraft:leash_knot#main"),
            wind_charge: model("minecraft:wind_charge#main"),
            shulker_bullet: model("minecraft:shulker_bullet#main"),
            wither_skull: model("minecraft:wither_skull#main"),
            evoker_fangs: model("minecraft:evoker_fangs#main").map(Arc::new),
            banner: model("minecraft:standing_banner#main")
                .zip(model("minecraft:standing_banner#flag")),
            layers,
        }
    }
}

impl GameCpuResources {
    pub(super) fn load(
        resources: &sunrise_assets::resource_source::ResourceRoot,
        document: Option<&sunrise_client::mobs::EntityModelDocument>,
        model_library: Option<&sunrise_assets::resolve::ModelLibrary>,
        draws: bool,
    ) -> Result<Self> {
        let root = resources.prepared();
        let owned_document;
        let document = if !draws {
            None
        } else if let Some(document) = document {
            Some(document)
        } else {
            owned_document = sunrise_client::mobs::EntityModelDocument::load(root).ok();
            owned_document.as_ref()
        };
        let visuals = draws.then(|| resources.source());
        let model = |key| document.and_then(|document| document.model(key));
        let (
            painting_variants,
            frames,
            armor_stand_assets,
            waypoint_styles,
            boat_models,
            cart_models,
            creative_screen,
        ) = std::thread::scope(|scope| {
            let painting_variants = named_scope(scope, "game_cpu.paintings", move || {
                paintings::PaintingVariants::load(root)
            });
            let frames = named_scope(scope, "game_cpu.frames", move || {
                if draws {
                    frames::bake_frames(model_library)
                } else {
                    Default::default()
                }
            });
            let armor_stand_assets = named_scope(scope, "game_cpu.armor_stands", move || {
                visuals
                    .map(|source| armor_stands::ArmorStandAssets::load(source, document))
                    .unwrap_or_default()
            });
            let waypoint_styles = named_scope(scope, "game_cpu.waypoints", move || {
                visuals
                    .map(sunrise_client::locator_bar::WaypointStyles::load)
                    .unwrap_or_default()
            });
            let boat_models = named_scope(scope, "game_cpu.boats", move || {
                boats::BoatModels::load(document)
            });
            let cart_models = named_scope(scope, "game_cpu.carts", move || {
                minecarts::CartModels::load(document)
            });
            let creative_screen = named_scope(scope, "game_cpu.creative", move || {
                let mut creative = Creative::load(root).map_err(|error| error.to_string())?;
                if draws {
                    let tab_modifiers =
                        sunrise_client::polytone_creative_tabs::TabModifier::read_all(root);
                    creative.set_pack_modifiers(tab_modifiers);
                }
                Ok::<_, String>(creative)
            });
            (
                painting_variants
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                frames
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                armor_stand_assets
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                waypoint_styles
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                boat_models
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                cart_models
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                creative_screen
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            )
        });
        let [
            frame_model,
            glow_frame_model,
            map_frame_model,
            glow_map_frame_model,
        ] = frames;
        Ok(Self {
            painting_variants,
            frame_model,
            glow_frame_model,
            map_frame_model,
            glow_map_frame_model,
            armor_stand_assets,
            waypoint_styles,
            boat_models,
            cart_models,
            crystal_model: model("minecraft:end_crystal#main"),
            cushion_model: model("minecraft:cushion#main"),
            dragon_model: model("minecraft:ender_dragon#main"),
            arrow_model: model("minecraft:arrow#main"),
            bee_stinger_model: model("minecraft:bee_stinger#main"),
            trident_model: model("minecraft:trident#main"),
            entity_models: EntityModels::load(document),
            creative_screen: creative_screen
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
        })
    }
}

/// The data the background loader produces for `Game::new`.
pub(super) struct Loaded {
    pub(super) game_cpu: Option<GameCpuResources>,
    /// The one parsed `entity-models.json`, for the handoff `Game::new` does
    /// itself on the synchronous launch.
    pub(super) entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
    pub(super) renderer_cpu: Option<sunrise_client::gfx::PreparedTextures>,
    pub(super) root: PathBuf,
    pub(super) pack_error: Option<String>,
    pub(super) resources: Arc<Resources>,
    pub(super) rest_loader: Option<WorldRestLoader>,
    /// The client reload listeners' prepared applies, staged by the loader
    /// thread; `start_game` runs them once the game exists.
    pub(super) client_applies: Vec<client_reload::PreparedClientReload>,
    pub(super) pack: PackResources,
    /// The menu bundle, for the synchronous launch; the loading screen receives
    /// it earlier through its own channel.
    pub(super) gui: Option<UiResources>,
}

/// What the background loader publishes: the menu bundle as soon as it is
/// ready, then the core bundle with a handle to the rest worker.
pub(super) enum LoaderMsg {
    Ui(
        UiResources,
        PathBuf,
        sunrise_assets::resource_source::ResourceSource,
    ),
    /// Clear the failed revision on the thread that owns settings, then acknowledge.
    ClearPackSelection {
        revision: u64,
        ack: std::sync::mpsc::Sender<()>,
    },
    Done(std::result::Result<Loaded, LoaderFailure>),
}

impl LoaderMsg {
    pub(super) fn done(result: Result<Loaded>) -> Self {
        Self::Done(result.map_err(
            |error| match error.downcast::<mods::DeclarationFailures>() {
                Ok(refusals) => LoaderFailure::Declarations(*refusals),
                Err(error) => LoaderFailure::Other(error.to_string()),
            },
        ))
    }
}

/// Only the startup report needs its error type preserved across threads.
pub(super) enum LoaderFailure {
    Declarations(mods::DeclarationFailures),
    Other(String),
}

impl From<LoaderFailure> for Box<dyn std::error::Error> {
    fn from(error: LoaderFailure) -> Self {
        match error {
            LoaderFailure::Declarations(refusals) => refusals.into(),
            LoaderFailure::Other(message) => message.into(),
        }
    }
}

pub(super) enum PendingWorldEntry {
    Local {
        folder: String,
        name: Option<String>,
        seed: Option<i64>,
        creative: bool,
        create: Option<Box<sunrise_client::title::WorldOptions>>,
    },
    Server {
        address: String,
        name: String,
        /// The saved server list row's name, when the join came from that row.
        list_name: Option<String>,
        lan: bool,
        realm: bool,
        /// A transfer's carried-over server cookies (`TransferState`).
        cookies: Option<sunrise_client::net::Cookies>,
    },
    StudioReplay(std::path::PathBuf),
}

pub(super) struct WorldRestLoader {
    pub(super) progress: Progress,
    pub(super) handle: JoinHandle<std::result::Result<(), String>>,
}

impl WorldRestLoader {
    pub(super) fn ready(&self) -> Result<bool> {
        // No time limit: the title stays usable while this loads, and a slow
        // disk or a debug build can take minutes. A failure ends the thread
        // and is reported by `finish`.
        Ok(self.handle.is_finished())
    }

    pub(super) fn finish(self) -> Result<()> {
        self.handle
            .join()
            .map_err(|_| "world resource loader panicked")?
            .map_err(Into::into)
    }
}

pub(super) fn write_report(game: &Game<'_>) -> Result<()> {
    if let Some(path) = &game.options.report {
        let report = serde_json::json!({
            "frames": game.frames,
            "elapsed_seconds": game.started.elapsed().as_secs_f64(),
            "chunks": game.world.chunks.len(),
            "generated": game.streaming.generated_total,
            "meshed_sections": game.streaming.meshed_total,
            "player": game.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]),
            "error": game.error.as_ref().map(|e| e.to_string()),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

/// The ordinary launch: load the resources on a worker while the window shows
/// the loading screen, then swap in the game on the main thread.
pub(super) fn run_loading(
    options: Options,
    settings: Settings,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
) -> Result<()> {
    #[cfg(target_os = "android")]
    let setup = if options.offscreen || options.world.is_some() {
        setup::Setup::default()
    } else {
        setup::Setup::for_launch(&vanilla_root)
    };
    #[cfg(not(target_os = "android"))]
    let setup = setup::Setup::default();
    let progress = Progress::new();
    let menu = startup_menu::StartupMenu::new(&options, pack_error.as_deref())?;
    let max_atlas_size = Arc::new(AtomicU32::new(sunrise_client::hud::MAX_ATLAS_SIZE));
    let waiting_for_choice = setup.onboarding_visible();
    let (receiver, loader) = if waiting_for_choice {
        // Setup finished on a previous run. Game starts its loader when chosen.
        (std::sync::mpsc::channel().1, None)
    } else {
        let (receiver, loader) = startup_menu::spawn_loader(
            &options,
            &settings,
            menu.pack_selection_revision,
            mods::LoadRole::startup(options.execution_role),
            &vanilla_root,
            started,
            &progress,
            &max_atlas_size,
        )?;
        (receiver, Some(loader))
    };
    let sound_registry = (!waiting_for_choice && options.dedicated.is_none()).then(|| {
        let root = vanilla_root.clone();
        std::thread::spawn(move || {
            // Starling: a phone's setup before 0.1.6 left the sounds out.
            sunrise_client::preparer::complete_sounds(&root);
            sunrise_client::sound_events::SoundRegistry::load(
                &sunrise_assets::resource_source::ResourceSource::directory(root),
            )
        })
    });
    let loading = Loading {
        window: None,
        renderer: None,
        boot_renderer: None,
        world_renderer: None,
        pick_hdr: Default::default(),
        max_atlas_size,
        boot: sunrise_client::hud::HudAtlas::boot(),
        gui: None,
        settings,
        fullscreen: sunrise_client::window::FullscreenState::new(false),
        video_modes: Default::default(),
        options: Some(options),
        vanilla_root,
        pack_error,
        started,
        panorama_rotation: Default::default(),
        progress,
        bar: 0.0,
        receiver,
        loaded: None,
        delivered: false,
        handle: loader,
        shown: false,
        ui_at: None,
        title_drawn: false,
        menu,
        error: None,
        failure: None,
        setup,
        touch_pointer: Default::default(),
        imports: Default::default(),
        audio: None,
        sound_registry,
        suspended: false,
    };
    let mut app = App::Loading(Box::new(loading));
    let result = event_loop()?.run_app(&mut app);
    narrator::stop();
    match &mut app {
        App::Ready(game) => leave_fullscreen_after(&result, &mut game.settings),
        App::Loading(loading) => leave_fullscreen_after(&result, &mut loading.settings),
    }
    result?;
    match app {
        App::Ready(mut game) => {
            write_report(&game)?;
            if let Some(e) = game.error.take() {
                return Err(e);
            }
        }
        App::Loading(loading) => {
            if let Some(e) = loading.error {
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_models_indexes_non_mob_layers_without_keeping_disk_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::fs::write(
            path.join("entity-models.json"),
            r#"{
                "minecraft:chest#main": {
                    "pose": [1.0, 2.0, 3.0, 0.25, -0.5, 0.75, 1.0, 2.0, 0.5],
                    "polygons": [
                        [
                            [0.0, 0.0, 0.0, 0.0, 0.0],
                            [14.0, 0.0, 0.0, 0.25, 0.0],
                            [14.0, 10.0, 0.0, 0.25, 0.5],
                            [0.0, 10.0, 0.0, 0.0, 0.5]
                        ]
                    ],
                    "normals": [[0.0, 0.0, 1.0]],
                    "children": {
                        "lid": { "pose": [1.0, 9.0, 1.0] },
                        "lock": { "pose": [7.0, 7.0, 15.0] }
                    }
                }
            }"#,
        )
        .unwrap();

        let document = sunrise_client::mobs::EntityModelDocument::load(&path).unwrap();
        let entity_models = EntityModels::load(Some(&document));
        drop(document);
        drop(dir);
        assert!(!path.exists());

        let mut root_pose = mod_api::PartPose::offset_and_rotation(1.0, 2.0, 3.0, 0.25, -0.5, 0.75);
        root_pose.x_scale = 1.0;
        root_pose.y_scale = 2.0;
        root_pose.z_scale = 0.5;
        let expected = mod_api::ModelPart::new(
            vec![mod_api::ModelPolygon::new(
                [
                    mod_api::ModelVertex::new([0.0, 0.0, 0.0], 0.0, 0.0),
                    mod_api::ModelVertex::new([14.0, 0.0, 0.0], 0.25, 0.0),
                    mod_api::ModelVertex::new([14.0, 10.0, 0.0], 0.25, 0.5),
                    mod_api::ModelVertex::new([0.0, 10.0, 0.0], 0.0, 0.5),
                ],
                [0.0, 0.0, 1.0],
            )],
            vec![
                (
                    "lid".to_owned(),
                    mod_api::ModelPart::new(
                        Vec::new(),
                        Vec::new(),
                        mod_api::PartPose::offset(1.0, 9.0, 1.0),
                    ),
                ),
                (
                    "lock".to_owned(),
                    mod_api::ModelPart::new(
                        Vec::new(),
                        Vec::new(),
                        mod_api::PartPose::offset(7.0, 7.0, 15.0),
                    ),
                ),
            ],
            root_pose,
        );

        assert_eq!(
            entity_models.layers.get("minecraft:chest#main"),
            Some(&expected)
        );
        assert_eq!(entity_models.layers.get("minecraft:unknown#main"), None);
        assert!(EntityModels::load(None).layers.is_empty());
    }
}
