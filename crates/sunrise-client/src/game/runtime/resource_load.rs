//! The startup's resource load: the stages the loading screen shows, the
//! scoped threads each step runs on, and the core, gameplay and pack tables
//! read from the prepared resources.
use super::*;

#[cfg(test)]
#[path = "incoming_tests/resource_load_scope.rs"]
mod scope_tests;

#[cfg(test)]
#[path = "resource_load/facts_memo.rs"]
mod facts_memo;

pub(super) const STAGE_LINES: [&str; 10] = [
    "Loading menu",
    "Loading assets",
    "Loading block facts",
    "Loading biomes",
    "Loading pack tables",
    "Loading collision",
    "Loading items",
    "Loading recipes",
    "Loading world resources",
    "World resources ready",
];

/// The background loader's checkpoint, read by the loading screen each frame.
#[derive(Clone)]
pub(super) struct Progress {
    pub(super) value: Arc<AtomicU32>,
    pub(super) lines: &'static [&'static str],
    /// A line in place of the stage's, while a step outside the stages runs (a content mods'
    /// build).
    note: Arc<std::sync::Mutex<Option<String>>>,
}
impl Progress {
    pub(super) fn new() -> Self {
        Self::with(&STAGE_LINES)
    }
    pub(super) fn with(lines: &'static [&'static str]) -> Self {
        Self {
            value: Arc::new(AtomicU32::new(0)),
            lines,
            note: Default::default(),
        }
    }
    pub(super) fn set(&self, stage: u32) {
        self.value.store(stage, Ordering::Relaxed);
    }
    pub(super) fn note(&self, line: Option<String>) {
        *self
            .note
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = line;
    }
    pub(super) fn noted(&self) -> Option<String> {
        self.note
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub(super) fn fraction(&self) -> f32 {
        self.value.load(Ordering::Relaxed) as f32 / (self.lines.len() - 1) as f32
    }
    pub(super) fn line(&self) -> String {
        self.noted().unwrap_or_else(|| {
            self.lines[(self.value.load(Ordering::Relaxed) as usize).min(self.lines.len() - 1)]
                .to_owned()
        })
    }
}

/// Harness and dedicated runs keep the fully-synchronous startup so their
/// output stays byte-identical; only an ordinary launch shows the window first.
pub(super) fn synchronous_startup(options: &Options) -> bool {
    // A headless game has no window for the loading screen.
    options.headless
        || options.frames.is_some()
        || options.world_frames.is_some()
        || options.benchmark.is_some()
        || options.control.is_some()
        || !options.execution_role.is_client()
        || std::env::var_os("SUNRISE_SCREEN_BUILD_FIXTURE").is_some()
        || std::env::var_os("SUNRISE_DISPENSE_FIXTURE").is_some()
        || std::env::var_os("SUNRISE_SCULK_FIXTURE").is_some()
        || std::env::var_os("SUNRISE_HOT_PATHS").is_some()
        || std::env::var_os("SUNRISE_COMMAND_SCRIPT").is_some()
        || std::env::var_os("SUNRISE_NET_REPORT").is_some()
}

/// The local world a client's launch opens at once (`--world`,
/// `--quickPlaySingleplayer`), whose generator data can load with the game's.
pub(super) fn launch_world(options: &Options) -> Option<&str> {
    (options.execution_role.is_client() && options.server.is_none() && !options.test_terrain)
        .then_some(options.world.as_deref())
        .flatten()
}

/// A scoped thread with a name, for one step of the resource load.
///
/// Named because the heap census and the startup trace both fall back to the
/// OS thread name: unnamed, every loader's retained memory and every span it
/// ran landed in one `unnamed thread` bucket. That bucket was 330 MiB of a
/// playing game's resident set -- the largest of all of them -- with nothing
/// to say which loader held it, and 13 of the trace's 17 lanes were
/// indistinguishable.
pub(super) fn named_scope<'scope, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    name: &str,
    f: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    #[cfg(test)]
    let inherited = mod_api::testing::inherit_mods(move |_: &()| f());
    #[cfg(test)]
    let f = move || inherited(&());
    std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        // Linux keeps 15 bytes of a thread name: a short prefix keeps them apart
        // ("sunrise-load-core" and "sunrise-load-collision" would both read
        // "sunrise-load-co").
        .name(format!("load-{name}"))
        .spawn_scoped(scope, f)
        // `Scope::spawn`, which this replaces, panics when the OS refuses a
        // thread. Keeping that: a resource load cannot go on without it.
        .expect("resource loader thread")
}

/// A step of the resource load: on its own scoped thread, or, when the
/// settings allow a single worker, run where it is joined.
pub(super) enum Step<'scope, T> {
    Thread(std::thread::ScopedJoinHandle<'scope, T>),
    Later(Box<dyn FnOnce() -> T + Send + 'scope>),
}
impl<'scope, T: Send + 'scope> Step<'scope, T> {
    /// `name` names the thread when one is spawned ([`named_scope`]).
    pub(super) fn spawn(
        scope: &'scope std::thread::Scope<'scope, '_>,
        parallel: bool,
        name: &str,
        f: impl FnOnce() -> T + Send + 'scope,
    ) -> Self {
        if parallel {
            Self::Thread(named_scope(scope, name, f))
        } else {
            Self::Later(Box::new(f))
        }
    }
    pub(super) fn join(self) -> T {
        match self {
            Self::Thread(handle) => handle.join().unwrap(),
            Self::Later(f) => f(),
        }
    }
}

/// The world bundle's core tables from one resource root, before
/// `load_resources` binds them into the pack tables and `WorldCore`.
pub(super) struct CoreTables {
    pub(super) item_data: Arc<sunrise_world::item_defaults::ItemData>,
    pub(super) entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
    pub(super) item_defaults: Arc<sunrise_world::item_defaults::ItemDefaults>,
    pub(super) assets: Assets,
    pub(super) render: RenderAssets,
    pub(super) item_renderer: sunrise_client::item_render::ItemRenderer,
    pub(super) visibility: sunrise_world::block_render::FaceVisibility,
    pub(super) gameplay: GameplayTables,
    pub(super) biomes: Arc<Biomes>,
    pub(super) environment: Environment,
    pub(super) collision: Arc<BlockCollisionRegistry<'static>>,
}
// Distinguish a failed pack (which startup can roll back) from a failed
// load or mod prepare (neither may publish a partly prepared generation).
enum CoreLoadError {
    Pack(String),
    Fatal(String),
}
impl From<mod_api::__private::reload::ReloadStageError> for CoreLoadError {
    fn from(error: mod_api::__private::reload::ReloadStageError) -> Self {
        Self::Fatal(error.diagnostic())
    }
}

/// The block facts and the tables read from them.
pub(super) struct GameplayTables {
    pub(super) behaviour_json: serde_json::Value,
    pub(super) facts: BlockFacts,
    pub(super) gameplay_projection: Vec<sunrise_client::blocks::GameplayProjection>,
    pub(super) light: LightTables,
    pub(super) motion: MotionResources,
}

/// [`CoreTables`] at `root`. Every table reads the block registry; past it,
/// the block models and sprites (then the item models, which share their
/// parsed models), the face visibility, the collisions, the biomes and the
/// environment need no block facts and load beside them. The outer error
/// fails the load; the inner one is the packs' (the registry or the block
/// models), which [`load_resources_for_mods`] rolls back from.
pub(super) fn load_core(
    resources: &sunrise_assets::resource_source::ResourceRoot,
    additional_data: &[Arc<sunrise_client::datapack::DataPack>],
    conditions: &sunrise_world::conditions::LoadConditions,
    load_role: mods::LoadRole,
    parallel: bool,
    progress: &Progress,
    launch_world: Option<&str>,
) -> std::result::Result<std::result::Result<CoreTables, String>, String> {
    let role = load_role.execution;
    let root = resources.prepared();
    let headless = !role.is_client();
    // The model library reads every file under `assets/` and needs no
    // block registry, so it is parsed beside `Assets::load` rather than
    // after it. It used to sit inside `reload` behind both the registry
    // read and the block facts; on a cold cache that read was most of a
    // second on the critical path, and warm it is still ~30 ms.
    // A server draws nothing and parses no models, so it skips this.
    let (assets, parsed_models, entity_models) = std::thread::scope(|scope| {
        let models = (parallel && !headless).then(|| {
            scope.spawn(|| {
                let _step = startup::step("core.models_parse");
                sunrise_client::model_library::ParsedModels::load(resources.source())
            })
        });
        let entity_models = (!headless).then(|| {
            Step::spawn(scope, parallel, "entity_models", || {
                startup::time("core.entity_models", || {
                    sunrise_client::mobs::EntityModelDocument::load(root).ok()
                })
            })
        });
        let assets = startup::time("core.assets", || Assets::load(root));
        let models = models.map(|m| m.join().unwrap_or_else(|e| std::panic::resume_unwind(e)));
        let entity_models = entity_models.and_then(|step| step.join()).map(Arc::new);
        (assets, models, entity_models)
    });
    let assets = match assets {
        Ok(assets) => assets,
        Err(e) => return Ok(Err(e.to_string())),
    };
    progress.set(1);
    let item_data =
        Arc::new(mods::load_item_data(root, role, load_role.owning).map_err(|e| e.to_string())?);
    let registry = assets.registry;
    let entity_models_for_render = entity_models.clone();
    let (render, gameplay, visibility, biomes, environment, collision) = std::thread::scope(
        |scope| {
            let assets = &assets;
            let item_data = &item_data;
            let render = Step::spawn(scope, parallel, "render", move || {
                // A server reads no block sprites' pixels, which only the
                // renderer draws (32 MB).
                let parsed_models = parsed_models;
                let render = startup::time("core.block_models_sprites", || {
                    if headless {
                        assets.reload_for_server(resources)
                    } else if let Some(parsed) = parsed_models {
                        assets.reload_with_models(resources, parsed)
                    } else {
                        assets.reload(resources)
                    }
                })
                .map_err(|e| e.to_string())?;
                let library = render.model_library.clone();
                let item_defaults = startup::time("core.item_defaults", || {
                    sunrise_world::item_defaults::shared(root)
                })
                .map_err(|e| e.to_string())?;
                let item_renderer = startup::time("core.item_models", || {
                    if headless {
                        sunrise_client::item_render::ItemRenderer::load_for_server(
                            resources, library, item_data,
                        )
                    } else if let Some(document) = entity_models_for_render.as_deref() {
                        sunrise_client::item_render::ItemRenderer::load_with_library_and_entity_models(
                            resources, library, document, item_data,
                        )
                    } else {
                        sunrise_client::item_render::ItemRenderer::load_with_library(
                            resources, library, item_data,
                        )
                    }
                })
                .map_err(|e| e.to_string());
                Ok((render, item_renderer, item_defaults))
            });
            // Nor the face visibility the mesher and the block entity
            // renderer cull with (19 MB).
            let visibility = Step::spawn(scope, parallel, "visibility", || {
                if headless {
                    return Ok(sunrise_world::block_render::FaceVisibility::empty());
                }
                startup::time("core.visibility", || {
                    models::load_visibility(root, registry)
                })
                .map_err(|e| e.to_string())
            });
            let biomes = Step::spawn(scope, parallel, "biomes", || {
                let biomes = Arc::new(
                    startup::time("core.biomes", || {
                        if headless {
                            Biomes::load_for_server(resources)
                        } else {
                            Biomes::load(resources)
                        }
                    })
                    .map_err(|e| e.to_string())?,
                );
                // A launch that names a world: its data preload needs only the
                // block registry and the biomes, so it starts here rather than
                // after the block models and facts (~250 ms later).
                if let Some(folder) = launch_world {
                    world_loading::preload_launch_world(
                        folder,
                        root,
                        resources.data_changes(),
                        registry,
                        biomes.clone(),
                        additional_data,
                    );
                }
                Ok::<_, String>(biomes)
            });
            let environment = Step::spawn(scope, parallel, "environment", || {
                startup::time("core.environment", || Environment::load(resources.source()))
                    .map_err(|e| e.to_string())
            });
            let collision = Step::spawn(scope, parallel, "collision", || {
                startup::time("core.collision", || collisions::load(root, registry))
                    .map_err(|e| e.to_string())
            });
            let gameplay = gameplay_tables(root, assets, conditions, parallel, progress);
            (
                render.join(),
                gameplay,
                visibility.join(),
                biomes.join(),
                environment.join(),
                collision.join(),
            )
        },
    );
    // The packs' failure first: it rolls back whatever else failed.
    let (render, item_renderer, item_defaults) = match render {
        Ok(render) => render,
        Err(e) => return Ok(Err(e)),
    };
    let gameplay = gameplay?;
    progress.set(3);
    Ok(Ok(CoreTables {
        item_data,
        item_defaults,
        entity_models,
        gameplay,
        biomes: biomes?,
        environment: environment?,
        visibility: visibility?,
        item_renderer: item_renderer?,
        collision: Arc::new(collision?),
        assets,
        render,
    }))
}
/// The block facts, then the light tables beside the motion tables, which
/// both read them.
pub(super) fn gameplay_tables(
    root: &std::path::Path,
    assets: &Assets,
    conditions: &sunrise_world::conditions::LoadConditions,
    parallel: bool,
    progress: &Progress,
) -> std::result::Result<GameplayTables, String> {
    let facts_step = startup::step("core.block_facts");
    // Read whole, then parsed from the buffer: `serde_json::from_reader`
    // pulls through a `BufReader` and is measurably slower on a file this
    // size than one read plus `from_slice`, which is how `BlockFacts` next
    // door already reads its own. Same value either way.
    let behaviour_json: serde_json::Value =
        sunrise_world::json_file::read(&root.join("block-behaviours.json"))
            .map_err(|e| e.to_string())?;
    #[cfg(not(test))]
    let (facts, gameplay_projection) =
        BlockFacts::load_with_projection(root, assets.registry, &behaviour_json, parallel)
            .map_err(|e| e.to_string())?;
    #[cfg(test)]
    let (facts, gameplay_projection) =
        facts_memo::load(root, assets.registry, &behaviour_json, parallel)?;
    drop(facts_step);
    progress.set(2);
    let (light, motion) = std::thread::scope(|scope| {
        let light = Step::spawn(scope, parallel, "light", || {
            startup::time("core.light", || {
                LightTables::load(root, assets.registry, &facts)
            })
            .map_err(|e| e.to_string())
        });
        let motion = startup::time("core.motion", || {
            let mut motion =
                MotionResources::load_with_conditions(root, assets, &facts, conditions)?;
            // #812: the attribute types mods declare come first, appended
            // after every vanilla attribute in the sorted catalog order, so
            // the rows below may name a mod's own new attribute and the
            // network ids stay the same on every side running these mods.
            let declared = mod_api::__private::attribute_declarations();
            if !declared.is_empty() {
                motion.attributes =
                    std::sync::Arc::new(with_mod_attributes(&motion.attributes, declared)?);
            }
            // #3728: then the bounds mods widen, on vanilla's attributes and
            // the declared ones alike, before anything reads a definition.
            let widenings = mod_api::__private::attribute_bounds_widenings();
            if !widenings.is_empty() {
                motion.attributes =
                    std::sync::Arc::new(with_mod_widened_bounds(&motion.attributes, widenings)?);
            }
            // #845: a mod's declared attribute sets
            // (`EntityAttributeCreationEvent`) become their entity types' rows
            // in the live attribute map's catalog — the one record the mob
            // logic and the attribute sync read (#1493) — built from the
            // template's row, as the type's content row is.
            for declaration in mod_api::__private::entity_attribute_declarations() {
                motion.attributes = std::sync::Arc::new(
                    motion
                        .attributes
                        .with_entity_row(
                            &declaration.id,
                            &declaration.template,
                            &declaration.attributes,
                        )
                        .map_err(|error| {
                            format!("mod entity attributes of {}: {error}", declaration.id)
                        })?,
                );
            }
            // #846: then the attributes mods add to existing types
            // (`EntityAttributeModificationEvent`, fired after the creation
            // event), on the same catalog rows.
            let modifications = mod_api::__private::entity_attribute_modifications();
            if !modifications.is_empty() {
                motion.attributes = std::sync::Arc::new(with_mod_attribute_additions(
                    &motion.attributes,
                    modifications,
                )?);
            }
            Ok::<_, Box<dyn std::error::Error>>(motion)
        })
        .map_err(|e| e.to_string());
        (light.join(), motion)
    });
    Ok(GameplayTables {
        light: light?,
        motion: motion?,
        behaviour_json,
        facts,
        gameplay_projection,
    })
}

/// Publishes `catalog`'s entity type suppliers, the ones mods declared and added to included,
/// as `ItemsHost::entity_type_default_attribute` reads them (#3944): each attribute's supplier
/// value (`AttributeSupplier.getValue`, the base inside its range; the spawn and reset bases
/// stay the raw ones). The read answers only a registered entity type's: a declaration for a
/// type no mod registered stays unread.
pub(in crate::game) fn publish_entity_type_defaults(
    catalog: &sunrise_world::attributes::entity::Catalog,
) {
    mod_api::__private::publish_entity_type_defaults(
        catalog
            .entities()
            .filter_map(|entity| Some((entity.to_owned(), catalog.supplier_values(entity)?))),
    );
}

/// The catalog with the attribute types mods declared (#812,
/// `Registrar::declare_attribute`), appended after every vanilla attribute:
/// `Catalog::with_attributes` sorts them by declaring mod id, then attribute
/// id, and refuses a duplicate, naming it.
pub(super) fn with_mod_attributes(
    catalog: &sunrise_world::attributes::entity::Catalog,
    declared: &[mod_api::__private::AttributeDeclaration],
) -> std::result::Result<sunrise_world::attributes::entity::Catalog, String> {
    let added = declared
        .iter()
        .map(|declared| {
            Ok(sunrise_world::attributes::entity::AddedAttribute {
                mod_id: declared.mod_id.clone(),
                id: declared.id.clone(),
                definition: sunrise_world::attributes::Definition::ranged(
                    declared.default,
                    declared.min,
                    declared.max,
                )
                .map_err(|error| format!("mod attribute {}: {error}", declared.id))?,
                syncable: declared.syncable,
                tooltip: sunrise_world::attributes::Tooltip {
                    sentiment: match declared.sentiment {
                        mod_api::Sentiment::Positive => {
                            sunrise_world::attributes::Sentiment::Positive
                        }
                        mod_api::Sentiment::Negative => {
                            sunrise_world::attributes::Sentiment::Negative
                        }
                        mod_api::Sentiment::Neutral => {
                            sunrise_world::attributes::Sentiment::Neutral
                        }
                    },
                    display: match declared.tooltip.display {
                        mod_api::AttributeDisplay::Ranged => {
                            sunrise_world::attributes::Display::Ranged
                        }
                        mod_api::AttributeDisplay::Percentage => {
                            sunrise_world::attributes::Display::Percentage
                        }
                        mod_api::AttributeDisplay::Boolean => {
                            sunrise_world::attributes::Display::Boolean
                        }
                    },
                    base_modifier: declared.tooltip.base_modifier.clone(),
                },
            })
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    catalog
        .with_attributes(&added)
        .map_err(|error| format!("mod attribute declarations: {error}"))
}

/// The catalog with the attribute bounds mods widened (#3728,
/// `Registrar::widen_attribute_bounds`): the union of every ask, whatever the
/// load order. An unknown attribute, or a range without the attribute's
/// default, refuses the load, naming the mod.
pub(super) fn with_mod_widened_bounds(
    catalog: &sunrise_world::attributes::entity::Catalog,
    widenings: &[mod_api::__private::AttributeBoundsWidening],
) -> std::result::Result<sunrise_world::attributes::entity::Catalog, String> {
    let asks: Vec<_> = widenings
        .iter()
        .map(|ask| sunrise_world::attributes::entity::WidenedBounds {
            mod_id: ask.mod_id.clone(),
            id: ask.id.clone(),
            min: ask.min,
            max: ask.max,
        })
        .collect();
    catalog
        .with_widened_bounds(&asks)
        .map_err(|error| format!("mod attribute bounds: {error}"))
}

/// The catalog with the attributes mods add to existing entity types
/// (#846, `EntityAttributeModificationEvent`), in registration order, so a
/// later addition of the same attribute wins. An unknown attribute or a type
/// without a row refuses the load, naming the type.
pub(super) fn with_mod_attribute_additions(
    catalog: &sunrise_world::attributes::entity::Catalog,
    modifications: &[mod_api::__private::EntityAttributeModification],
) -> std::result::Result<sunrise_world::attributes::entity::Catalog, String> {
    let mut catalog = catalog.clone();
    for modification in modifications {
        catalog = catalog
            .with_entity_additions(&modification.entity, &modification.attributes)
            .map_err(|error| {
                format!(
                    "mod entity attribute additions to {}: {error}",
                    modification.entity
                )
            })?;
    }
    Ok(catalog)
}

/// Each mob type mods added attributes to takes its grown catalog row
/// (`with_mod_attribute_additions`) as its attribute map and spawn fields, as
/// a declared set's type does (#845), so a new mob of the type spawns with
/// what the live map answers. A type with no mob type (the player) builds
/// its map from the catalog alone.
pub(super) fn take_modified_attribute_rows(
    types: &mut HashMap<String, sunrise_client::mobs::MobType>,
    catalog: &sunrise_world::attributes::entity::Catalog,
    modifications: &[mod_api::__private::EntityAttributeModification],
) {
    let modified: std::collections::BTreeSet<&str> = modifications
        .iter()
        .map(|modification| modification.entity.as_str())
        .collect();
    for entity in modified {
        if let (Some(kind), Some(row)) = (types.get_mut(entity), catalog.entity_row(entity)) {
            kind.take_attribute_row(Arc::new(row), catalog);
        }
    }
}

/// Fixture helper for tests without a launch mod state.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(super) fn load_resources(
    roots: impl Into<packs::PackRoots>,
    additional_data: Vec<Arc<sunrise_client::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiResources, &std::path::Path),
) -> Result<Loaded> {
    load_resources_for_mods(
        roots.into(),
        additional_data,
        launch_world,
        vanilla_root,
        settings,
        started,
        menu,
        role,
        progress,
        publish,
        &mut || unreachable!("fixture without pack recovery"),
        &mods::ModsState::default(),
    )
}

/// The prepared resource location, optional world to preload, and launch start time.
pub(super) struct StartupLoadContext<'a> {
    pub(super) vanilla_root: &'a std::path::Path,
    pub(super) launch_world: Option<&'a str>,
    pub(super) started: Instant,
}

/// A launch without an interactive title owns its settings on the loading thread.
pub(super) fn load_resources_sync(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<sunrise_client::datapack::DataPack>>,
    context: StartupLoadContext<'_>,
    settings: &mut Settings,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    mod_sources: &mods::ModsState,
) -> Result<Loaded> {
    let snapshot = settings.clone();
    load_resources_for_mods(
        roots,
        additional_data,
        context.launch_world,
        context.vanilla_root,
        &snapshot,
        context.started,
        false,
        role,
        progress,
        &mut |_, _| {},
        &mut || {
            packs::clear_selection(settings);
            Ok(())
        },
        mod_sources,
    )
}

/// A synchronous launch loads both phases; an ordinary launch returns the
/// core bundle while the rest worker keeps loading. `launch_world` names the
/// world to open, whose data is preloaded during startup.
///
/// The owning mod state supplies the same cached condition table used to build
/// content roots and reload data. Recovery clears the settings owner's selection
/// before reopening the required resource packs over the accepted content root.
#[allow(clippy::too_many_arguments)]
pub(super) fn load_resources_for_mods(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<sunrise_client::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiResources, &std::path::Path),
    clear_pack_selection: &mut dyn FnMut() -> Result<()>,
    mod_sources: &mods::ModsState,
) -> Result<Loaded> {
    let load_role: mods::LoadRole = role.into();
    let role = load_role.execution;
    let packs::PackRoots {
        base,
        selected,
        cleared,
    } = roots;
    let mut recover = |failure: &str, required: &[sunrise_client::packs::Pack]| -> Result<_> {
        log::warn!("Sunrise: resource packs failed to load: {failure}");
        clear_pack_selection()?;
        packs::layer_client_packs(&base, required).map_err(Into::into)
    };
    let (resolved, mut rollback, mut pack_error) = match (selected, cleared) {
        (Ok(selected), cleared) => (selected, cleared, None),
        (Err(error), Some(required)) => {
            let failure = error.to_string();
            (recover(&failure, &required)?, None, Some(failure))
        }
        (Err(error), None) => return Err(error.into()),
    };
    let root = base.prepared().to_path_buf();
    let headless = !role.is_client();
    let texture_atlases = if headless {
        Vec::new()
    } else {
        client_reload::collect_texture_atlases()
    };
    let mut resolved = client_reload::stage_texture_atlases(resolved, &texture_atlases);
    let threads = settings.sunrise.worker_threads();
    // One pool of workers for every level's streaming, the client's and the
    // server thread's (sunrise_threads::jobs), sized before any world opens.
    sunrise_threads::jobs::configure(threads);
    sunrise_client::sprites::set_texture_budget(settings.sunrise.texture_budget_mib);
    sunrise_client::sprites::set_labpbr_maps(shader_packs::picks_material_pack(settings));
    // Stage A: the menu bundle (atlas and panorama). With a loading screen it is
    // published at once so the title screen can appear while the world bundle
    // loads; the synchronous launch instead carries it in `Loaded`.
    // A dedicated server builds only the font (UiResources::server), as the
    // integrated server's game does: the full atlas and panorama are 144 MB.
    let load_ui = |source: &sunrise_assets::resource_source::ResourceSource| {
        let _step = startup::step("ui.load");
        if headless {
            UiResources::server(source)
        } else {
            UiResources::load(source, vanilla_root).map(|ui| {
                ui.with_splashes(
                    source,
                    vanilla_root,
                    settings.extras.skip_modded_splashes_if_vanilla_modified,
                )
            })
        }
    };
    // The menu bundle loads on this thread while workers build the world
    // bundle's core tables from the same root (`load_core`). A pack failure in
    // either clears the client selection, not required mod resources (NeoForge's
    // `Minecraft.rollbackResourcePacks`).
    let parallel = threads > 1;
    let mut offer = |ui: UiResources, root: &std::path::Path| {
        if menu {
            log::info!(
                "Sunrise: UI bundle ready after {} ms",
                started.elapsed().as_millis()
            );
            publish(ui, root);
            None
        } else {
            Some(ui)
        }
    };
    let inventory = sunrise_mod_package::enabled::EnabledMods::read(&root)?;
    let conditions = super::load_conditions::conditions_for_mods(mod_sources, &inventory)?;
    let (core, loaded_gui, resolved, client_applies) = loop {
        let listeners = (!headless)
            .then(|| client_reload::register_startup_client_reload_listeners(Some(settings)));
        let (ui, core) = std::thread::scope(|scope| {
            let core = |_unit: &()| {
                // A failed pack starts core again from the game's own
                // resources, which the world's preload must read: with packs
                // it waits for the core that loaded.
                let early = launch_world.filter(|_| root == vanilla_root);
                let build = || {
                    startup::time("core", || {
                        load_core(
                            &resolved,
                            &additional_data,
                            &conditions,
                            load_role,
                            parallel,
                            progress,
                            early,
                        )
                    })
                    .map_err(CoreLoadError::Fatal)?
                    .map_err(CoreLoadError::Pack)
                };
                if let Some(listeners) = listeners {
                    client_reload::stage_client_reload(resolved.source(), listeners, build)
                } else {
                    build().map(|core| (core, Vec::new()))
                }
            };
            #[cfg(test)]
            let core = mod_api::testing::inherit_mods(core);
            let core = Step::spawn(scope, parallel, "core", || core(&()));
            let ui = load_ui(resolved.source()).map(|ui| offer(ui, &root));
            (ui, core.join())
        });
        // A cloud texture vanilla would read past fails its reload (`CloudRenderer.prepare`),
        // and so this load: checked here, where the packs can still be dropped, before the
        // pack tables read it again. A server draws no clouds.
        let clouds = match headless {
            true => Ok(()),
            false => sunrise_client::clouds::load(resolved.source()).map(drop),
        };
        // Core data is provisional: a selected world pack may replace an entry.
        let (failure, required) = match (ui, core, clouds, rollback.take()) {
            (Ok(gui), Ok((core, applies)), Ok(()), _) => {
                break (core, gui, resolved, applies);
            }
            (_, Err(CoreLoadError::Fatal(e)), _, _) => return Err(e.into()),
            (Err(e), _, _, Some(required)) => (e.to_string(), required),
            (_, Err(CoreLoadError::Pack(e)), _, Some(required)) => (e, required),
            (_, _, Err(e), Some(required)) => (e.to_string(), required),
            (_, Err(CoreLoadError::Pack(e)), _, None) => {
                return Err(format!(
                    "{e}\nPrepare resources with the launcher, or: sunrise-prepare --prepare"
                )
                .into());
            }
            (Err(e), Ok(_), _, None) => return Err(e),
            (Ok(_), Ok(_), Err(e), None) => return Err(e.into()),
        };
        resolved =
            client_reload::stage_texture_atlases(recover(&failure, &required)?, &texture_atlases);
        pack_error = Some(failure);
    };
    // The same prepared gameplay root supplies entity keys and spawner facts,
    // regardless of which client resource layers loaded successfully.
    let spawner_types = mods::register_iterated_registries(&root)?;
    log::info!("load core tables: {:.2}s", started.elapsed().as_secs_f32());
    let CoreTables {
        item_data,
        item_defaults,
        entity_models,
        assets,
        render,
        item_renderer,
        visibility,
        gameplay:
            GameplayTables {
                behaviour_json,
                facts,
                gameplay_projection,
                light,
                motion,
            },
        biomes,
        environment,
        collision,
    } = core;
    // The accepted suppliers and resolved entity keys belong to the same Parts installation.
    publish_entity_type_defaults(&motion.attributes);
    let assets = Arc::new(assets);
    let render = Arc::new(render);
    // The namespaces of the mod block states registered now: what every level.dat saved from
    // here on records, and what a world is checked against before it opens (`level_mods`). A
    // launch world's data preload, which may already have started, reads no mod records; its
    // open, which checks them, waits for these resources.
    {
        use sunrise_client::save::level_mods;
        let registry = assets.registry;
        let names = (0..registry.len() as u32)
            .filter_map(|id| registry.state(sunrise_world::registry::BlockStateId(id)))
            .map(|state| state.name.as_str());
        level_mods::set_block_namespaces(Some(level_mods::mod_namespaces(names)));
    }
    if let Some(folder) = launch_world {
        world_loading::preload_launch_world(
            folder,
            &root,
            resolved.data_changes(),
            assets.registry,
            biomes.clone(),
            &additional_data,
        );
    }
    api_values::resolve_block_definitions(assets.registry)?;
    mods::extend_poi_types(assets.registry);
    mods::register_cauldron_fluid_contents(assets.registry);
    let tables = Arc::new(startup::time("pack.mesh_tables", || {
        MeshTables::new(
            &assets,
            &render,
            &facts,
            visibility,
            api_values::mod_tint_values(assets.registry)?,
        )
        .map(|tables| tables.with_polytone_colormaps(resolved.source(), assets.registry))
    })?);
    let pack_step = startup::step("pack.assemble");
    let mut pack = PackResources::assemble(
        &resolved,
        assets.clone(),
        render,
        tables,
        item_renderer,
        biomes.clone(),
        Arc::new(light),
    )?;
    pack.additional_data = additional_data.into();
    drop(pack_step);
    log::info!("load pack tables: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(4);
    if !menu {
        use sunrise_client::lang::{self, Language};
        // An integrated server shares the client's loaded resources and
        // language; it doesn't run this loader in production.
        lang::set(if role.is_dedicated_server() {
            Language::load_default(&vanilla_root.to_path_buf())
        } else {
            Language::load(&[&pack.source], &settings.language)
        });
    }
    progress.set(5);
    let suffocation =
        sunrise_client::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation)?;
    let resources = Resources {
        core: WorldCore {
            item_data,
            facts,
            biomes: biomes.clone(),
            environment,
            collision,
            motion,
            suffocation,
            spawn_registries: sunrise_client::mobs::finalize::SpawnRegistries::load(&root),
            threads,
        },
        rest: std::sync::OnceLock::new(),
    };
    let resources = Arc::new(resources);
    if menu {
        log::info!(
            "Sunrise: core-ready after {} ms",
            started.elapsed().as_millis()
        );
    }
    // The block entity renderer's face visibility: the meshes' own, whose
    // shapes already hold their faces, not a second load of the same tables.
    let visibility = pack.context.tables.visibility.clone();
    let rest_loader = if menu {
        let resources = resources.clone();
        let resource_root = resolved.clone();
        let vanilla_root = vanilla_root.to_owned();
        let entity_models = entity_models.clone();
        let rest_progress = progress.clone();
        let progress = progress.clone();
        let handle = std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("world-rest-loader".into())
            .spawn(move || {
                let _step = startup::step("rest");
                let rest = world_rest::load_world_rest(
                    &resource_root,
                    &conditions,
                    &resources,
                    spawner_types,
                    &assets,
                    &behaviour_json,
                    &gameplay_projection,
                    entity_models.as_deref(),
                    item_defaults.as_ref(),
                    visibility,
                    &progress,
                    started,
                    true,
                )
                .map_err(|error| error.to_string())?;
                resources
                    .rest
                    .set(rest)
                    .map_err(|_| "world resources already loaded".to_owned())?;
                progress.set(9);
                log::info!(
                    "Sunrise: rest-ready after {} ms",
                    started.elapsed().as_millis()
                );
                write_prepared_files(vanilla_root);
                Ok(())
            })?;
        Some(WorldRestLoader {
            progress: rest_progress,
            handle,
        })
    } else {
        let rest = world_rest::load_world_rest(
            &resolved,
            &conditions,
            &resources,
            spawner_types,
            &assets,
            &behaviour_json,
            &gameplay_projection,
            entity_models.as_deref(),
            item_defaults.as_ref(),
            visibility,
            progress,
            started,
            !headless,
        )?;
        resources
            .rest
            .set(rest)
            .map_err(|_| "world resources already loaded")?;
        None
    };
    // The world tables load (above) while the handoff's CPU work is done: the
    // world renderer's textures beside the game's own tables. A failure here
    // waits for the tail, so no tail worker outlives its pack-loader generation.
    let prepared_at = Instant::now();
    let (game_cpu, renderer_cpu) = if menu {
        let (game_cpu, renderer_cpu) = std::thread::scope(|scope| {
            // A server read no sprite pixels (`reload_for_server`), so it has no window textures to mip.
            let textures = (!headless).then(|| {
                Step::spawn(scope, parallel, "textures", || {
                    startup::time("handoff.renderer_textures", || {
                        prepare_renderer_textures(&pack)
                    })
                })
            });
            let game_cpu = startup::time("handoff.game_cpu", || {
                GameCpuResources::load(
                    &resolved,
                    entity_models.as_deref(),
                    pack.render.model_library.as_deref().ok(),
                )
            });
            (game_cpu, textures.map(Step::join))
        });
        let game_cpu = match game_cpu {
            Ok(game_cpu) => game_cpu,
            Err(error) => {
                if let Some(tail) = rest_loader {
                    let _ = tail.handle.join();
                }
                return Err(error);
            }
        };
        log::info!(
            "Sunrise: handoff CPU resources prepared on worker in {} ms",
            prepared_at.elapsed().as_millis()
        );
        (Some(game_cpu), renderer_cpu)
    } else {
        (None, None)
    };
    Ok(Loaded {
        game_cpu,
        entity_models,
        renderer_cpu,
        root,
        pack_error,
        resources,
        rest_loader,
        client_applies,
        pack,
        gui: loaded_gui,
    })
}

/// Write the prepared folder's `sunrise-data.bin` when it has none the game
/// can use: on a thread of its own, at idle priority, once the start is done.
/// This start has just read the folder's files (they are in the disk cache);
/// the next starts read them from one file (#1961).
pub(super) fn write_prepared_files(root: PathBuf) {
    use sunrise_core::prepared_files;
    if prepared_files::open(&root).is_some() || !root.join("sunrise-resources.json").is_file() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .name("Prepared data file".into())
        .spawn(move || {
            sunrise_threads::idle_current();
            let started = Instant::now();
            match prepared_files::write(&root) {
                Ok(()) => log::info!(
                    "Sunrise: wrote {} in {} ms",
                    prepared_files::FILE_NAME,
                    started.elapsed().as_millis()
                ),
                Err(e) => log::info!("Sunrise: {} not written: {e}", prepared_files::FILE_NAME),
            }
        });
    if let Err(e) = spawned {
        log::info!(
            "Sunrise: {} not written: {e}",
            sunrise_core::prepared_files::FILE_NAME
        );
    }
}
