//! Prepare local worlds without blocking the window's event loop.
use super::*;
use sunrise_client::vanillagen::{
    TerrainResources, shared_biome_source_data, shared_resource_data,
};
use sunrise_core::identifier::Identifier;
use sunrise_world::worldgen::{
    biome_source::{BiomeSourceError, ResolvedBiomeSource},
    generator::{BiomeSourceData, GeneratorData},
    mod_generators::{ModChunkGenerator, ModChunkGenerators, mod_chunk_generators},
};

pub(super) struct WorldRequest {
    pub save_root: PathBuf,
    pub folder: String,
    pub name: Option<String>,
    pub seed: Option<i64>,
    pub creative: bool,
    pub create: Option<sunrise_client::title::WorldOptions>,
    pub rule_registry: Arc<sunrise_client::gamerules::RuleRegistry>,
    /// Where the server's region workers send their notices.
    pub(super) chunk_reports: Arc<sunrise_client::chunk_io::Reports>,
    /// The account the world opens under, for an integrated server
    /// (`IntegratedServer.getSingleplayerProfile`): its name and UUID.
    owner: Option<(String, u128)>,
    /// A fixture's effective registry traversal, before shape projection.
    #[cfg(test)]
    pub(super) stem_order: Option<&'static [&'static str]>,
}

pub(super) struct PreparedWorld {
    pub save_root: PathBuf,
    pub level: Level,
    pub session_lock: sunrise_client::save::SessionLock,
    pub rand_sequences: sunrise_client::random_sequences::RandomSequences,
    /// The owner's offline UUID and the account's, when an older world moved its
    /// owner at open (`playerdata::migrate_owner`): entities migrate as they load.
    pub owner_migration: Option<(u128, u128)>,
    created: bool,
    save_reporter: Arc<sunrise_client::chunk_io::SaveFailureReporter>,
    chunk_reports: Arc<sunrise_client::chunk_io::Reports>,
    #[cfg(test)]
    stem_order: Option<&'static [&'static str]>,
}

pub(super) struct PreparedTerrain {
    pub shapes: sunrise_client::chunk::LevelShapes,
    pub configured_levels: Vec<Dim>,
    pub world: World,
    pub streaming: Streaming,
    pub save: Arc<SaveDir>,
    pub locator: Option<Arc<VanillaTerrain>>,
}

/// The dedicated server's `Main` takes level.dat_old without asking
/// (`getUnfixedDataTagWithFallback`): there is no world list to recover from. The integrated
/// server's world opens from that list, which offers the recovery instead, whatever the launch
/// configuration says.
fn fall_back_to_level_dat_old_without_asking(
    role: execution_role::ExecutionRole,
    save_root: &std::path::Path,
    folder: &str,
) {
    if role.is_dedicated_server() && sunrise_client::save::fall_back_to_level_dat_old(save_root) {
        log::warn!("Sunrise: level.dat of \"{folder}\" did not read; level.dat_old took its place");
    }
}

/// Settle level.dat before applying any on-disk choice for the world's generator.
fn prepare_level_data(
    role: execution_role::ExecutionRole,
    save_root: &std::path::Path,
    folder: &str,
    game_dir: &std::path::Path,
) -> Result<()> {
    // openWorldLoadLevelData: level data that does not read is RecoverWorldDataScreen's to
    // handle (the title screen shows it); a world opened another way must not become a
    // new one with a random seed under the same name.
    fall_back_to_level_dat_old_without_asking(role, save_root, folder);
    if let Some(issue) = sunrise_client::save::level_data_needs_recovery(save_root) {
        let why = match issue {
            sunrise_client::save::DatIssue::Missing => "level.dat is missing".to_owned(),
            sunrise_client::save::DatIssue::Unreadable(why) => why,
        };
        return Err(format!(
            "Failed to load world \"{folder}\": its level data does not read ({why}). \
             Open it from the world list to try restoring level.dat_old."
        )
        .into());
    }
    // LevelSummary.isCompatible, which the dedicated server's `Main` checks before it fixes
    // a world: a world from another series is refused, never upgraded in place.
    if let Some(version) = sunrise_client::save::incompatible_version(save_root) {
        let version = if version.is_empty() {
            "unknown".to_owned()
        } else {
            version
        };
        return Err(format!(
            "Failed to load world \"{folder}\": it was last played in version {version}, \
             which cannot be opened in this version."
        )
        .into());
    }
    if role.is_dedicated_server() && save_root.join("level.dat").exists() {
        super::dedicated::biome_regions::choose(save_root, &game_dir.join("server.toml"))?;
    }
    Ok(())
}

/// `MinecraftServer.getServerDirectory` and its integrated override.
fn chunk_report_root<'a>(
    role: execution_role::ExecutionRole,
    game_dir: Option<&'a std::path::Path>,
    working_dir: &'a std::path::Path,
) -> &'a std::path::Path {
    if role == execution_role::ExecutionRole::IntegratedServer {
        // `Options::read` may already have made the configured game directory the cwd.
        game_dir.unwrap_or(working_dir)
    } else {
        working_dir
    }
}

impl WorldRequest {
    pub fn new(
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&sunrise_client::title::WorldOptions>,
        rule_registry: Arc<sunrise_client::gamerules::RuleRegistry>,
    ) -> Result<Self> {
        Ok(Self {
            save_root: std::env::current_dir()?.join("saves").join(folder),
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned(),
            rule_registry,
            chunk_reports: sunrise_client::chunk_io::PROCESS_REPORTS.clone(),
            owner: None,
            #[cfg(test)]
            stem_order: None,
        })
    }

    /// The account this world opens under (`IntegratedServer.getSingleplayerProfile`):
    /// a world older builds made under the offline profile moves its owner to it at
    /// open. A dedicated server has no single-player owner and leaves it unset.
    pub fn owned_by(mut self, player: &str, owner: u128) -> Self {
        self.owner = Some((player.to_owned(), owner));
        self
    }

    pub fn prepare(
        self,
        res: &Resources,
        pack: &PackResources,
        options: &Options,
        upgrade: &sunrise_world::datafix::progress::UpgradeProgress,
    ) -> Result<PreparedWorld> {
        let _prepare_step = startup::step("world.prepare");
        let Self {
            save_root,
            folder,
            name,
            seed,
            creative,
            create,
            rule_registry,
            chunk_reports,
            owner,
            #[cfg(test)]
            stem_order,
        } = self;
        let name = name.as_deref();
        sunrise_client::biome::Biomes::forget_missing_warnings();
        let create = create.map(|mut create| {
            create.set_additional_data_packs(pack.additional_data.clone());
            create
        });
        let create = create.as_ref();
        prepare_level_data(
            options.execution_role,
            &save_root,
            &folder,
            options
                .game_dir
                .as_deref()
                .unwrap_or(std::path::Path::new(".")),
        )?;
        sunrise_client::save::upgrade_older_world_with(&save_root, upgrade)?;
        sunrise_client::save::migrate_legacy(&save_root)?;
        let session_lock = sunrise_client::save::SessionLock::acquire(&save_root)?;
        // RandomSequences: the world's data/minecraft/random_sequences.dat.
        let rand_sequences = super::trading::read_random_sequences(&save_root);
        let existing = Level::read_with_registry(&save_root, rule_registry.clone());
        let created = existing.is_none();
        let level = existing.unwrap_or_else(|| Level {
            name: name.unwrap_or(&folder).to_owned(),
            // WorldOptions.randomSeed when no seed was given.
            seed: seed.unwrap_or_else(|| {
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
                (t ^ (t >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64
            }),
            creative,
            day_time: 0,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            rule_registry,
            ..Level::default()
        });
        let mut level = level;
        // `IntegratedServer.setSingleplayerProfile`: a world older Sunrise builds made
        // under the offline profile moves its owner to the account at open — the player
        // file, its `UUID` and level.dat's `singleplayer_uuid`. Its entities follow as
        // their chunks load (`owner_migration`, `entity_nbt::migrate_player_uuid`).
        let owner_migration = owner.and_then(|(player, account)| {
            if created {
                // The profile exists before `IntegratedServer.initServer` saves; the player does not.
                level.player_uuid = Some(sunrise_client::lan::playerdata::uuid_ints(account));
            }
            let old = sunrise_protocol::crypto::offline_uuid(&player);
            (old != account).then(|| {
                if !created
                    && sunrise_client::lan::playerdata::migrate_owner(
                        &save_root,
                        &mut level,
                        old,
                        account,
                        res.items(),
                    )
                {
                    log::info!(
                        "Sunrise: \"{folder}\" was made under the offline profile; \
                         its owner moved to the account's UUID"
                    );
                }
                (old, account)
            })
        });
        // CreateWorldScreen's choices, before streaming reads the generator.
        if created && let Some(o) = create {
            level.hardcore = o.hardcore;
            level.difficulty = sunrise_client::title::DIFFICULTIES[o.difficulty()].into();
            level.allow_commands = Some(o.commands(creative));
            o.write_world_gen(&save_root, level.seed)?;
            // The DataPackConfig the screen chose: the picked packs on,
            // everything else disabled — the feature packs left out, and the
            // world packs the Data Packs screen held back (`install` copies
            // every pack of the folder, selected or not).
            let packs = sunrise_client::datapack::feature_packs(&options.resources);
            level.data_packs = Some({
                let mut disabled: Vec<String> = packs
                    .iter()
                    .map(|p| p.id.clone())
                    .filter(|f| !o.data_packs.contains(f))
                    .collect();
                if let Some(dir) = o.pack_dir.as_deref() {
                    disabled.extend(
                        sunrise_client::packs::folder(
                            std::path::Path::new(dir),
                            sunrise_core::version::DATA_PACK_FORMAT,
                        )
                        .into_iter()
                        .map(|p| p.id)
                        .filter(|id| !o.data_packs.contains(id)),
                    );
                }
                (o.data_packs.clone(), disabled)
            });
            // WorldDataConfiguration's `enabled_features`: the flags of the
            // picked feature packs over vanilla's (`FeatureFlags.REGISTRY`'s
            // codec writes the whole set, `minecraft:vanilla` included, and
            // omits the tag when it is the default).
            level.enabled_features = o.enabled_features(&options.resources).into();
            // CreateWorldScreen copies the edited rules with the final flags,
            // even when the player never opened the rules screen.
            level.game_rules = o
                .rules
                .copy_with_features(&level.enabled_features)
                .by_name();
        }
        // Worlds made without the Create New World screen (--world, the
        // control socket) take its default: commands on for Creative only.
        if created {
            level.allow_commands.get_or_insert(creative);
        }
        // The biome regions are fixed before any chunk generates: a new world records the
        // installed mods' regions; an existing one opens only with a record that reads and a
        // recorded choice for each missing mod's region.
        if created {
            sunrise_client::save::biome_regions::create(&save_root)?;
        } else {
            sunrise_client::save::biome_regions::open(&save_root)
                .map_err(|why| format!("Failed to load world \"{folder}\": {why}"))?;
        }
        if let Some(d) = &options.dimension {
            level.dimension = d.clone();
            level.position = None;
        }
        let integrated = options.execution_role == execution_role::ExecutionRole::IntegratedServer;
        // Resolving the report path must not prevent the world from opening.
        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let server_root = chunk_report_root(
            options.execution_role,
            options.game_dir.as_deref(),
            &working_dir,
        );
        // Dedicated requests carry a storage path as `folder` and a level id as `name`.
        let level_id = if options.execution_role.is_dedicated_server() {
            name.unwrap_or(&folder)
        } else {
            &folder
        };
        let save_reporter = Arc::new(sunrise_client::chunk_io::SaveFailureReporter::new(
            integrated,
            server_root,
            level_id,
        ));
        Ok(PreparedWorld {
            save_root,
            level,
            session_lock,
            rand_sequences,
            owner_migration,
            created,
            save_reporter,
            chunk_reports,
            #[cfg(test)]
            stem_order,
        })
    }
}

impl PreparedWorld {
    /// Materialize the level only once the world's registry winners have been admitted.
    pub(super) fn prepare_terrain(
        &mut self,
        res: &Resources,
        pack: &PackResources,
        options: &Options,
        settings: &Settings,
        biomes: &WorldBiomeRegistry,
    ) -> Result<PreparedTerrain> {
        let save_root = &self.save_root;
        let level = &mut self.level;
        let mut overrides = biomes.overrides.clone();
        if self.created {
            level.generator = sunrise_client::save::WorldGenerator::read_new(save_root);
        } else if options.execution_role.is_dedicated_server() {
            level.recover_world_generator(&overrides, || {
                sunrise_client::random_source::create().next_long()
            })?;
        }
        growth::install_live_features(res, pack, options, &overrides);
        let (shapes, configured_levels) = level_shapes(
            &mut overrides,
            &options.resources,
            level,
            #[cfg(test)]
            self.stem_order,
        );
        Dim::set_extra(shapes.extra.iter().map(|level| level.id));
        // The bounded registry-loader leaf slice: fail world preparation before streaming,
        // rather than letting the flat-generator fallback swallow a selected pack's codec error.
        overrides.validate_fill_layers(pack.assets.registry)?;
        // PrepareSpawnTask: resolve the player's dimension against this world's stems,
        // without changing the saved position or adding a stem for the player.
        let dim = configured_levels
            .iter()
            .find(|dim| dim.id() == level.dimension)
            .or_else(|| {
                let spawn = level.world_spawn_dimension.as_deref()?;
                configured_levels.iter().find(|dim| dim.id() == spawn)
            })
            .copied()
            .unwrap_or(Dim::Overworld);
        level.dimension = dim.id().to_owned();
        let shape = shapes.get(dim.id());
        let streaming_step = startup::step("world.make_streaming");
        let (mut streaming, save, locator, generator_type) = make_streaming(
            res,
            pack,
            options,
            settings,
            save_root,
            dim,
            shape,
            level.seed,
            level,
            biomes.table.clone(),
            &overrides,
            None,
            self.chunk_reports.clone(),
            self.save_reporter.clone(),
        )?;
        drop(streaming_step);
        streaming.radius = settings.render_distance;
        let mut world = Game::authoritative_world_for_mods(res, level.seed);
        world.attachments = save.load_attachments();
        world.shape = shape;
        world.generator_type = Some(generator_type);
        world.fast_lava = sunrise_world::dimension_type::DimensionFacts::of_level(
            sunrise_core::LevelKey::parse(dim.id()).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
        )
        .fast_lava;
        world.game_time = level.time;
        world.day_time = options.time.unwrap_or(level.day_time);
        world.end_clock = level.end_clock.unwrap_or(level.time);
        world.weather = level.weather;
        if let Some(w) = options.weather.as_deref() {
            // /weather: clear, rain or thunder for a long duration.
            let (rain, thunder) = (w != "clear", w == "thunder");
            world.weather = sunrise_client::world::Weather {
                clear_time: if w == "clear" { 6000 } else { 0 },
                rain_time: 6000,
                thunder_time: 6000,
                raining: rain,
                thundering: thunder,
                rain_level: if rain { (1.0, 1.0) } else { (0.0, 0.0) },
                thunder_level: if thunder { (1.0, 1.0) } else { (0.0, 0.0) },
            };
        }

        // IntegratedServer.initServer writes the first level.dat after preparing the levels.
        // The worker must not write an initialized world or the unplaced owner's player file.
        if options.execution_role != execution_role::ExecutionRole::IntegratedServer
            && !save_root.join("level.dat").exists()
        {
            level.write(save_root, res.items())?;
        }
        Ok(PreparedTerrain {
            shapes,
            configured_levels,
            world,
            streaming,
            save,
            locator,
        })
    }
}

pub(super) struct WorldLoader {
    handle: JoinHandle<std::result::Result<PreparedWorld, String>>,
}

impl WorldLoader {
    /// Prepare the world on its own thread. `upgrade` is what its upgrade has done
    /// (`UpgradeProgress`), which the window's `FileFixerProgressScreen` shows while the
    /// file fixers run, and where its Cancel stops them.
    pub fn start(
        request: WorldRequest,
        res: Arc<Resources>,
        pack: PackResources,
        options: Options,
        upgrade: Arc<sunrise_world::datafix::progress::UpgradeProgress>,
    ) -> Result<Self> {
        let handle = std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("World initialization".into())
            .spawn(move || {
                request
                    .prepare(&res, &pack, &options, &upgrade)
                    .map_err(|e| e.to_string())
            })?;
        Ok(Self { handle })
    }

    pub fn finish(self) -> Result<PreparedWorld> {
        self.handle
            .join()
            .map_err(|_| "world initialization worker panicked")?
            .map_err(Into::into)
    }
}

/// How long the preloaded world data outlives the world screens: a player
/// who goes back to the title or to Multiplayer and opens no world for this
/// long gets the memory back.
const PRELOAD_KEPT: std::time::Duration = std::time::Duration::from_secs(60);

enum Preload {
    Idle,
    /// Read, or being read, on the "World data preload" thread. `left`: when
    /// the player last left the world screens.
    Started {
        thread: std::thread::JoinHandle<()>,
        left: Option<std::time::Instant>,
    },
    /// A world opened: the data is the world's now, and stays.
    Used,
}

static PRELOAD: std::sync::Mutex<Preload> = std::sync::Mutex::new(Preload::Idle);

/// Each frame of the title: preload the shared worldgen bundle on a helper
/// thread while the player is on the world screens
/// (`on_world_screens`: Singleplayer, Create World and their
/// dialogs), so it overlaps the time spent choosing a world. A world opened
/// meanwhile waits for the part being read instead of reading it again. Not
/// at the title screen itself: many players only go to Multiplayer, and
/// small machines need the memory. Dropped after `PRELOAD_KEPT` away from
/// the world screens without a world opening.
pub(super) fn preload_world_data(
    on_world_screens: bool,
    resources: &std::path::Path,
    data_source: &sunrise_assets::resource_source::ResourceSource,
    registry: &'static sunrise_world::registry::BlockRegistry,
) {
    let mut preload = PRELOAD.lock().unwrap_or_else(|e| e.into_inner());
    match &mut *preload {
        Preload::Used => {}
        Preload::Idle if on_world_screens => {
            let resources = resources.to_owned();
            let data_source = data_source.clone();
            let spawned = std::thread::Builder::new()
                .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
                .name("World data preload".into())
                .spawn(move || {
                    sunrise_threads::behind_server();
                    // Errors are left for the world opening to report.
                    if let Ok(data) = sunrise_client::vanillagen::shared_data_from(
                        &resources.join("worldgen"),
                        &Default::default(),
                        registry,
                        &data_source,
                    ) {
                        // The Overworld's feature decoration, which every
                        // seed shares: about 140 ms off opening the world.
                        let _ = data.prepare_decoration("minecraft:normal", "minecraft:overworld");
                    }
                });
            match spawned {
                Ok(thread) => *preload = Preload::Started { thread, left: None },
                Err(e) => log::warn!("Sunrise: world data preload: {e}"),
            }
        }
        Preload::Idle => {}
        Preload::Started { left, .. } if on_world_screens => *left = None,
        Preload::Started { thread, left } => {
            let left = *left.get_or_insert_with(std::time::Instant::now);
            if left.elapsed() >= PRELOAD_KEPT && thread.is_finished() {
                sunrise_client::vanillagen::forget_shared_data();
                super::lan::sync::forget_sync_data();
                super::lan::sync::forget_server_sources();
                sunrise_threads::release_free_memory();
                *preload = Preload::Idle;
                log::info!("Sunrise: preloaded world data dropped, no world opened");
            }
        }
    }
}

/// A launch that names a world (`--world`, `--quickPlaySingleplayer`): the
/// same preload, started as soon as the block registry and the biomes are
/// loaded, so it overlaps the rest of the startup instead of the world's
/// opening. Read for the world's own data packs and preset, the bundle and
/// decoration `make_streaming` asks for; a world they don't fit (one its
/// upgrade rewrites) reads its own when it opens.
pub(super) fn preload_launch_world(
    folder: &str,
    resources: &std::path::Path,
    builtin_source: &sunrise_assets::resource_source::ResourceSource,
    data_source: &sunrise_assets::resource_source::ResourceSource,
    registry: &'static sunrise_world::registry::BlockRegistry,
    additional: &[Arc<sunrise_client::datapack::DataPack>],
    base_conditions: sunrise_world::conditions::LoadConditions,
) {
    let mut preload = PRELOAD.lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(*preload, Preload::Idle) {
        return;
    }
    let Ok(dir) = std::env::current_dir() else {
        return;
    };
    let world_root = dir.join("saves").join(folder);
    let resources = resources.to_owned();
    let builtin_source = builtin_source.clone();
    let data_source = data_source.clone();
    let additional = additional.to_vec();
    let spawned = std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .name("World data preload".into())
        .spawn(move || {
            // Not behind the server: the world's opening waits for it.
            let _step = startup::step("world.preload");
            let started = std::time::Instant::now();
            // Errors are left for the world opening to report. The world's
            // flags from its level.dat: the feature packs a flag forces on.
            let level = Level::read(&world_root);
            let enabled_features = level
                .as_ref()
                .map_or(&[][..], |l| l.enabled_features.as_ref());
            let stack = super::functions::pack_stack(
                &world_root,
                &resources,
                &builtin_source,
                &data_source,
                false,
                enabled_features,
                &additional,
            );
            let enabled: Vec<_> = stack.enabled_packs().collect();
            let conditions = super::load_conditions::worldgen_load_conditions(
                &base_conditions,
                &resources,
                &enabled,
            );
            let Ok((overrides, _)) =
                worldgen_overrides_of(&stack.data_packs(Vec::new()), &conditions)
            else {
                return;
            };
            if let Ok(data) = sunrise_client::vanillagen::shared_data_from(
                &resources.join("worldgen"),
                &overrides,
                registry,
                &data_source,
            ) {
                let generator = level
                    .as_ref()
                    .and_then(|l| l.generator.as_ref())
                    .map(|g| g.settings());
                let preset = sunrise_client::save::WorldGen::noise_preset(generator);
                let _ = data.prepare_decoration(preset, "minecraft:overworld");
            }
            log::info!(
                "Sunrise: world data preloaded for the launch's world in {} ms",
                started.elapsed().as_millis()
            );
        });
    match spawned {
        Ok(thread) => *preload = Preload::Started { thread, left: None },
        Err(e) => log::warn!("Sunrise: world data preload: {e}"),
    }
}

/// A world opens: what `preload_world_data` read is the world's from now on.
pub(super) fn world_data_used() {
    *PRELOAD.lock().unwrap_or_else(|e| e.into_inner()) = Preload::Used;
}

/// The world's frozen biome admission. Generation and the join packet project
/// these winners without running their conditions in a different load context.
pub(super) struct WorldBiomeRegistry {
    pub(super) table: Arc<Biomes>,
    pub(super) overrides: sunrise_world::worldgen::overrides::WorldgenOverrides,
    differs_from_preload: bool,
}

impl WorldBiomeRegistry {
    pub(super) fn load(
        base: &Arc<Biomes>,
        root: &std::path::Path,
        blocks: &'static sunrise_world::registry::BlockRegistry,
        stack: &super::functions::PackStack,
        data: &sunrise_client::datapack::DataPacks,
        overlay: &sunrise_world::loot::PackOverlay<'_>,
        read_errors: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self> {
        use serde_json::Value;
        use std::{
            borrow::Cow,
            collections::{BTreeMap, BTreeSet},
        };
        use sunrise_world::worldgen::biome::{
            AdmittedBiomeGeneration, BiomeDefinition, BiomeGenerationRegistries,
        };
        const KIND: &str = "worldgen/biome";
        if let Some((file, error)) = read_errors
            .iter()
            .find(|(key, _)| key.starts_with("worldgen/biome/"))
        {
            return Err(format!("{file}: {error}").into());
        }
        // Only vanilla holders come from the boot table. Passive mod biomes
        // come from the prepared root selected for this world, not the previous mod inventory.
        let mut winners: BTreeMap<_, _> = base
            .registry_order()
            .into_iter()
            .filter(|id| id.starts_with("minecraft:"))
            .filter_map(|id| {
                base.builtin_file(id)
                    .map(|file| (id.to_owned(), Cow::Borrowed(file)))
            })
            .collect();
        let prepared_mods = sunrise_client::biome::mod_biome_files(root);
        let mut prepared_only: BTreeSet<_> =
            prepared_mods.iter().map(|(id, _)| id.clone()).collect();
        winners.extend(
            prepared_mods
                .into_iter()
                .map(|(id, file)| (id, Cow::Owned(file))),
        );
        for (id, files) in overlay.kind(KIND) {
            let value: Value = serde_json::from_slice(sunrise_world::loot::PackOverlay::top(files))
                .map_err(|error| format!("biome {id}: {error}"))?;
            prepared_only.remove(id);
            winners.insert(id.to_owned(), Cow::Owned(value));
        }
        let modified = sunrise_client::mod_content::biomes::modified(root);
        let mut payloads = BTreeMap::new();
        let mut omitted: BTreeSet<_> = base
            .registry_order()
            .into_iter()
            .filter(|id| !winners.contains_key(*id))
            .map(str::to_owned)
            .collect();
        let mut candidates = BTreeSet::new();
        for (id, winner) in winners {
            if !overlay.conditions.allows_id(&winner, KIND, &id) {
                omitted.insert(id);
                continue;
            }
            candidates.insert(id.clone());
            let payload = match winner {
                Cow::Borrowed(winner)
                    if !modified.contains(&id) && winner.get("neoforge:conditions").is_none() =>
                {
                    // Unchanged prepared vanilla definitions are already in both base readers.
                    continue;
                }
                winner => sunrise_world::conditions::into_payload(winner.into_owned()),
            };
            payloads.insert(id, payload);
        }
        let (mut inputs, warnings) =
            sunrise_client::datapack::worldgen_overrides(data, overlay.conditions);
        for warning in warnings {
            log::warn!("Sunrise: data pack worldgen: {warning}");
        }
        // `worldgen_overrides` has already run the winning registry files' conditions.
        sunrise_client::vanillagen::add_passive_structure_templates(
            &mut inputs,
            stack.content.source(),
        );
        let dir = root.join("worldgen");
        let mut resolved = BTreeMap::new();
        let mut overrides = inputs.clone();
        let mut generation = None;
        let mut files = BTreeMap::new();
        for (id, payload) in payloads {
            let generation = match &mut generation {
                Some(generation) => generation,
                slot @ None => slot.insert(BiomeGenerationRegistries::load(
                    &dir,
                    &inputs,
                    blocks,
                    candidates.clone(),
                )?),
            };
            let (file, decoded) = BiomeDefinition::decode(payload, generation)
                .map_err(|error| format!("biome {id}: {error}"))?
                .into_parts();
            files.insert(id.clone(), file.to_string());
            resolved.insert(id, decoded);
        }
        // Prepared-only mod files can already contain modifier output. Let the
        // modifier pass recover their original inputs from the prepared metadata.
        overrides.set_biome_winners(
            files
                .iter()
                .filter(|(id, _)| !prepared_only.contains(*id))
                .map(|(id, file)| (id.clone(), file.clone()))
                .collect(),
            omitted.clone(),
        );
        let mut overrides =
            sunrise_client::vanillagen::biome_modifiers_over_packs(&dir, &overrides)?
                .unwrap_or(overrides);
        let mut final_files = files.clone();
        final_files.extend(
            overrides
                .entries("biomes")
                .map(|(id, json)| (id.to_owned(), json.to_owned())),
        );
        let mut packed = Vec::new();
        for (id, json) in &final_files {
            let file = serde_json::from_str::<Value>(json)
                .map_err(|error| format!("biome {id}: {error}"))?;
            if files.get(id).is_some_and(|original| original == json) {
                packed.push((id.to_owned(), file));
                continue;
            }
            let generation = match &mut generation {
                Some(generation) => generation,
                slot @ None => slot.insert(BiomeGenerationRegistries::load(
                    &dir,
                    &inputs,
                    blocks,
                    candidates.clone(),
                )?),
            };
            let (file, decoded) = BiomeDefinition::decode(file, generation)
                .map_err(|error| format!("biome {id}: {error}"))?
                .into_parts();
            packed.push((id.to_owned(), file));
            resolved.insert(id.to_owned(), decoded);
        }
        let differs_from_preload = !packed.is_empty() || !omitted.is_empty();
        let table = if differs_from_preload {
            overrides.set_biome_winners(
                packed
                    .iter()
                    .map(|(id, file)| (id.clone(), file.to_string()))
                    .collect(),
                omitted.clone(),
            );
            overrides.bind_biome_generation(AdmittedBiomeGeneration::new(resolved));
            Arc::new(base.with_admitted_packs(
                packed.iter().map(|(id, file)| (id.as_str(), file)),
                &omitted,
            )?)
        } else {
            base.clone()
        };
        Ok(Self {
            table,
            overrides,
            differs_from_preload,
        })
    }

    pub(super) fn differs_from_preload(&self) -> bool {
        self.differs_from_preload
    }
}

/// The worldgen entries of the merged view `data` (the Create screen's chosen packs, or the
/// world's enabled ones: `applyNewPackConfig` reloads with the new pack config), and what the
/// generator leaves out of them. A file whose `neoforge:conditions` fail to decode refuses
/// the load (`RegistryDataLoader` fails on a decode error).
pub(super) fn worldgen_overrides_of(
    data: &sunrise_client::datapack::DataPacks,
    conditions: &sunrise_world::conditions::LoadConditions,
) -> std::result::Result<
    (
        sunrise_world::worldgen::overrides::WorldgenOverrides,
        Vec<String>,
    ),
    String,
> {
    let loaded = sunrise_client::datapack::worldgen_overrides(data, conditions);
    conditions.check_errors()?;
    Ok(loaded)
}

/// [`worldgen_overrides_of`] over the view of the world a test's fixture folder holds.
#[cfg(test)]
fn worldgen_overrides_in_world(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    enabled_features: &[String],
    additional: &[Arc<sunrise_client::datapack::DataPack>],
    base_conditions: &sunrise_world::conditions::LoadConditions,
) -> std::result::Result<
    (
        sunrise_world::worldgen::overrides::WorldgenOverrides,
        Vec<String>,
    ),
    String,
> {
    let empty = sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
    let stack = super::functions::pack_stack(
        world_root,
        resources,
        &empty,
        &empty,
        false,
        enabled_features,
        additional,
    );
    let enabled: Vec<_> = stack.enabled_packs().collect();
    let conditions =
        super::load_conditions::worldgen_load_conditions(base_conditions, resources, &enabled);
    worldgen_overrides_of(&stack.data_packs(Vec::new()), &conditions)
}

/// The shapes of the world's levels: their dimension types, the world's
/// enabled packs' stems, then saved preset/editor stems, over vanilla's. A refused type keeps the
/// vanilla shape, with a warning.
fn level_shapes(
    overrides: &mut sunrise_world::worldgen::overrides::WorldgenOverrides,
    resources: &std::path::Path,
    level: &Level,
    #[cfg(test)] stem_order: Option<&[&str]>,
) -> (sunrise_client::chunk::LevelShapes, Vec<Dim>) {
    if let Some(generator) = &level.generator {
        generator.merge_dimensions(overrides);
    } else {
        overrides.include_implicit_level_stems();
    }
    let stems = overrides.level_stems();
    #[cfg(test)]
    let (shapes, stems) = match stem_order {
        Some(order) => {
            assert_eq!(order.len(), overrides.level_stems().count());
            shapes_of(
                overrides,
                order
                    .iter()
                    .map(|id| overrides.level_stems().find(|(key, _)| key == id).unwrap()),
            )
        }
        None => shapes_of(overrides, stems),
    };
    #[cfg(not(test))]
    let (shapes, stems) = shapes_of(overrides, stems);
    Dim::set_extra(shapes.extra.iter().map(|level| level.id));
    set_level_sea_levels(
        overrides,
        sunrise_world::worldgen::overrides::mod_dimensions(),
        resources,
    );
    (shapes, stems.into_iter().map(Dim::from_id).collect())
}

/// The sea levels of the levels whose stem the mods or the data packs give: a mod's own
/// (`DimensionSettings::sea_level`), else the `sea_level` of the noise settings their stem names,
/// the packs' over the game's. The game's spawning, weather and maps then use them. A level of a
/// generator that reads no noise settings has its built-in level's, and the three built-in levels
/// too unless the packs replace their stem or their noise settings.
fn set_level_sea_levels(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    mods: &[sunrise_world::worldgen::overrides::ModDimension],
    resources: &std::path::Path,
) {
    use sunrise_world::worldgen::overrides::{DIMENSIONS, NOISE_SETTINGS, stem_sea_level};
    let read = |id: &str| -> Option<serde_json::Value> {
        if let Some((_, json)) = overrides
            .entries(NOISE_SETTINGS)
            .find(|(entry, _)| *entry == id)
        {
            return serde_json::from_str(json).ok();
        }
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let file = resources.join(format!(
            "data/{namespace}/worldgen/noise_settings/{path}.json"
        ));
        serde_json::from_slice(&std::fs::read(file).ok()?).ok()
    };
    let mut levels: Vec<(sunrise_core::LevelKey, i32)> = Vec::new();
    for (id, stem) in overrides.entries(DIMENSIONS) {
        let own = mods.iter().find(|d| d.id == id).and_then(|d| d.sea_level);
        // The noise settings are what the terrain is built with; `sea_level` of the mod is for
        // a generator that has none.
        let stem = serde_json::from_str::<serde_json::Value>(stem).ok();
        // A superflat generator builds at -63 (`FlatLevelSource.getSeaLevel`) whatever its level
        // is told; a mod's explicit sea level still wins for it.
        let flat = stem
            .as_ref()
            .is_some_and(|stem| stem["generator"]["type"].as_str() == Some("minecraft:flat"));
        let sea = stem
            .and_then(|stem| stem_sea_level(&stem, &read))
            .or(own)
            .or(flat.then_some(-63));
        if let (Some(sea), Ok(key)) = (sea, sunrise_core::LevelKey::parse(id)) {
            levels.push((key, sea));
        }
    }
    // A built-in level the packs gave no stem: its default noise settings, which a pack may replace.
    for (id, settings) in [
        ("minecraft:overworld", "minecraft:overworld"),
        ("minecraft:the_nether", "minecraft:nether"),
        ("minecraft:the_end", "minecraft:end"),
    ] {
        let Ok(key) = sunrise_core::LevelKey::parse(id) else {
            continue;
        };
        if levels.iter().any(|(level, _)| *level == key) {
            continue;
        }
        let replaced = overrides
            .entries(NOISE_SETTINGS)
            .any(|(entry, _)| entry == settings);
        let sea = replaced
            .then(|| read(settings))
            .flatten()
            .and_then(|json| i32::try_from(json["sea_level"].as_i64()?).ok());
        if let Some(sea) = sea {
            levels.push((key, sea));
        }
    }
    sunrise_world::level::set_sea_levels(levels);
}

/// Resolve shapes in the effective registry's traversal, retaining it for level creation.
fn shapes_of<'a>(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    stems: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> (sunrise_client::chunk::LevelShapes, Vec<&'static str>) {
    use sunrise_client::chunk::{ExtraLevel, LevelShape, LevelShapes};
    let builtins = [Dim::Overworld.id(), Dim::Nether.id(), Dim::End.id()];
    let mut shapes = LevelShapes::VANILLA;
    let mut extra = Vec::new();
    let mut configured = Vec::new();
    for (level, stem) in stems {
        let shape = overrides.level_height(level).and_then(|h| {
            LevelShape::new(
                h.min_y,
                h.height,
                h.logical_height,
                h.has_skylight,
                h.has_ceiling,
            )
            .ok()
        });
        if let Some(id) = builtins.into_iter().find(|id| *id == level) {
            configured.push(id);
            if let Some(shape) = shape {
                *shapes.get_mut(id) = shape;
            } else {
                log::warn!(
                    "Sunrise: data pack worldgen: the dimension type of {id} is not one the \
                     game accepts; the level keeps the vanilla build height"
                );
            }
        } else if let Some(shape) = shape {
            // A stem's `type` is a dimension type's id, or the type itself.
            let Some(stem) = stem.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            else {
                continue;
            };
            let id: &'static str = level.to_owned().leak();
            let dimension_type = stem["type"].as_str().unwrap_or(level).to_owned().leak();
            extra.push(ExtraLevel {
                id,
                dimension_type,
                shape,
            });
            configured.push(id);
        } else {
            log::warn!(
                "Sunrise: data pack worldgen: the dimension type of {level} is not one the \
                 game accepts; the level is left out"
            );
        }
    }
    shapes.extra = sunrise_client::chunk::ExtraLevels::new(extra);
    set_level_types(overrides, &shapes);
    (shapes, configured)
}

/// The dimension types the mods and packs changed or added, for the levels `shapes` holds: what
/// `DimensionFacts::of_level` answers for them. A type the game refuses keeps the level's own.
fn set_level_types(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    shapes: &sunrise_client::chunk::LevelShapes,
) {
    use sunrise_core::LevelKey;
    let levels = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ]
    .into_iter()
    .chain(shapes.extra.iter().map(|level| level.id));
    let mut facts = Vec::new();
    for level in levels {
        match overrides.level_facts(level) {
            Some(Ok(found)) => facts.extend(LevelKey::parse(level).ok().map(|key| (key, found))),
            Some(Err(why)) => log::warn!(
                "Sunrise: data pack worldgen: the dimension type of {level} sets a field the game                  refuses ({why}); the level keeps the type it would have without it"
            ),
            None => {}
        }
    }
    sunrise_world::dimension_type::DimensionFacts::set_levels(facts);
}

/// A registered generator paired with a biome source bound to the world's registries.
pub(in crate::game) struct ModStem {
    /// Full terrain-stage worldgen resources, loaded only when `generator.vanilla_stages().any()`.
    data: Option<Arc<GeneratorData>>,
    json: serde_json::Value,
    generator_id: Identifier,
    generator: Arc<dyn ModChunkGenerator>,
    pub(super) source: Arc<ResolvedBiomeSource>,
}

impl ModStem {
    pub(super) fn bind(
        biomes: &BiomeSourceData,
        json: serde_json::Value,
        generator_id: Identifier,
        generator: Arc<dyn ModChunkGenerator>,
        seed: i64,
    ) -> std::result::Result<Self, BiomeSourceError> {
        let source = biomes
            .resolve_mod_biome_source(&json["generator"]["biome_source"].to_string(), seed)?;
        Ok(Self {
            data: None,
            json,
            generator_id,
            generator,
            source: Arc::new(source),
        })
    }

    pub(super) fn with_stage_data(mut self, data: Arc<GeneratorData>) -> Self {
        self.data = Some(data);
        self
    }
}

/// What generates a level whose stem names a generator a mod registered.
enum ModLevel {
    /// The generator's terrain alone.
    Terrain(sunrise_client::modgen::ModTerrain),
    /// Its terrain and the requested vanilla stages over the stem's noise settings.
    Staged(VanillaTerrain),
}

/// Resolve the world's pack or saved custom stem, or reuse the runtime stem bound at admission,
/// with the id of the registered generator that builds it.
fn mod_terrain(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    terrain_resources: TerrainResources<'_>,
    generators: &ModChunkGenerators,
    dim: Dim,
    shape: sunrise_client::chunk::LevelShape,
    seed: i64,
    structures: bool,
    runtime_stem: Option<&ModStem>,
    saved_stem: Option<&sunrise_core::nbt::Tag>,
) -> Option<std::result::Result<(ModLevel, Identifier), String>> {
    let bound;
    let stem = if let Some(stem) = runtime_stem {
        stem
    } else {
        if generators.is_empty() {
            return None;
        }
        let json: serde_json::Value = overrides
            .entries(sunrise_world::worldgen::overrides::DIMENSIONS)
            .find(|(level, _)| *level == dim.id())
            .and_then(|(_, stem)| serde_json::from_str(stem).ok())
            .or_else(|| saved_stem.map(sunrise_client::scoreboard::tag_to_json))?;
        let requested = Identifier::parse_namespaced(json["generator"]["type"].as_str()?)?;
        let (id, generator) = generators.get_key_value(&requested)?;
        let result = shared_biome_source_data(terrain_resources.dir, overrides)
            .map_err(|e| e.to_string())
            .and_then(|biomes| {
                ModStem::bind(&biomes, json, id.clone(), generator.clone(), seed)
                    .map_err(|e| format!("biome source: {e}"))
            })
            .and_then(|stem| {
                if generator.vanilla_stages().any() {
                    let data = shared_resource_data(terrain_resources, overrides)
                        .map_err(|e| e.to_string())?;
                    Ok(stem.with_stage_data(data))
                } else {
                    Ok(stem)
                }
            });
        bound = match result {
            Ok(stem) => stem,
            Err(e) => return Some(Err(format!("{}: {e}", dim.id()))),
        };
        &bound
    };
    let level = if stem.generator.vanilla_stages().any() {
        let Some(data) = stem.data.clone() else {
            return Some(Err(format!(
                "{}: missing worldgen data for vanilla stages",
                dim.id()
            )));
        };
        Some(
            VanillaTerrain::for_mod(
                terrain_resources,
                data,
                seed,
                dim.id(),
                &stem.json,
                stem.generator.clone(),
                stem.source.clone(),
                structures,
            )
            .map(ModLevel::Staged)
            .map_err(|e| format!("{}: {e}", dim.id())),
        )
    } else {
        let biomes = terrain_resources.biomes;
        Some(
            sunrise_client::modgen::ModTerrain::new(
                dim.id(),
                stem.generator.clone(),
                stem.source.clone(),
                terrain_resources.registry,
                biomes.index.clone(),
                shape,
                seed,
            )
            .map(ModLevel::Terrain),
        )
    };
    level.map(|level| level.map(|level| (level, stem.generator_id.clone())))
}

/// The file beside the world that keeps the structure references of the level `id`. An id with a
/// `/` puts it in a folder of `data/`.
pub(super) fn structure_references_file(
    world_root: &std::path::Path,
    id: &str,
) -> std::path::PathBuf {
    world_root.join("data").join(format!(
        "sunrise_structure_references_{}.json",
        id.replace(':', "_")
    ))
}

/// Where `inner` keeps the structure references an explorer map skips, and how it reads a
/// saved chunk's.
fn keep_structure_references(
    inner: &VanillaTerrain,
    world_root: &std::path::Path,
    dim: Dim,
    save: &Arc<SaveDir>,
) {
    inner.set_reference_file(structure_references_file(world_root, dim.id()));
    let save = save.clone();
    inner.set_saved_references(Arc::new(move |name, x, z| {
        save.start_references(x, z, name)
    }));
}

pub(super) fn make_streaming(
    res: &Resources,
    pack: &PackResources,
    options: &Options,
    settings: &Settings,
    world_root: &std::path::Path,
    dim: Dim,
    shape: sunrise_client::chunk::LevelShape,
    seed: i64,
    level: &Level,
    biomes: Arc<Biomes>,
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    runtime: Option<&super::runtime_levels::Generate>,
    chunk_reports: Arc<sunrise_client::chunk_io::Reports>,
    save_reporter: Arc<sunrise_client::chunk_io::SaveFailureReporter>,
) -> Result<(
    Streaming,
    Arc<SaveDir>,
    Option<Arc<VanillaTerrain>>,
    Identifier,
)> {
    use sunrise_client::save::WorldGen;
    let generator = level.generator.as_ref().map(|g| g.settings());
    let saved_stem = generator.and_then(|s| s.get("dimensions")?.get(dim.id()));
    let mut save = SaveDir::new(
        sunrise_client::save::dimension_dir(world_root, dim.id()),
        shape,
        Some(res.shared_items()),
    );
    save.mark_server_owned();
    save.set_save_reporting(
        world_root.to_path_buf(),
        dim.id(),
        save_reporter,
        chunk_reports,
    );
    let save = Arc::new(save);
    if dim == Dim::End {
        // Migrate before any chunk reader can cache the old block entities. A side file it
        // can't read stays where it is and costs only its old exits: the End still opens.
        if let Err(e) = save.migrate_end_gateway_exits() {
            log::warn!("Sunrise: End gateway exits not migrated: {e}");
        }
    }
    let worldgen_dir = options.resources.join("worldgen");
    let terrain_resources = TerrainResources {
        data: Some(&pack.data_changes),
        dir: &worldgen_dir,
        registry: pack.assets.registry,
        biomes: &biomes,
        item_data: &res.core.item_data,
    };
    let mut locator = None;
    // The generator the branch below builds the level with, as `BuiltInRegistries.CHUNK_GENERATOR`
    // names its codec: what `LevelRef::generator_type` reads.
    let generator_type;
    let overworld_settings = generator.filter(|_| dim == Dim::Overworld);
    use sunrise_client::flatgen::FlatTerrain;
    let flat = if runtime.is_none() {
        // Enabled packs own the resolved stem; saved presets fill in levels they don't define.
        let packed_stem = overrides
            .entries(sunrise_world::worldgen::overrides::DIMENSIONS)
            .find(|(id, _)| *id == dim.id())
            .map(|(_, json)| serde_json::from_str::<serde_json::Value>(json))
            .transpose()
            .map_err(|e| format!("{}: cannot decode dimension stem: {e}", dim.id()))?
            .as_ref()
            .map(sunrise_client::scoreboard::json_to_tag);
        packed_stem
            .as_ref()
            .or(saved_stem)
            .and_then(|stem| stem.get("generator"))
            .map(FlatTerrain::decode_generator)
            .transpose()
            .map_err(|e| format!("{}: cannot decode flat dimension: {e}", dim.id()))?
            .flatten()
    } else {
        None
    };
    // `DebugLevelSource`: the overworld of the Debug preset.
    let debug = overworld_settings.and_then(|s| {
        sunrise_client::debugworld::DebugTerrain::from_settings(
            s,
            pack.assets.registry,
            &biomes,
            shape,
        )
    });
    // PlaceCommand.placeJigsaw/placeStructure generate directly, independently
    // of WorldOptions.generateStructures. Keep their flat locator available
    // even when automatic structure starts are disabled.
    let automatic_structures = generator.is_none_or(|s| WorldGen::from_settings(s).structures);
    // The sea level the level's rules and the mods read is the one its generator builds with
    // (`ChunkGenerator.getSeaLevel`), so a pack's noise settings or a superflat world are seen:
    // the superflat generators' is -63 (`FlatLevelSource.getSeaLevel`).
    let register_sea_level = |sea_level: i32| {
        if let Ok(key) = sunrise_core::LevelKey::parse(dim.id()) {
            sunrise_world::level::register_sea_level(key, sea_level);
        }
    };
    let mut streaming = if let Some(super::runtime_levels::Generate::Layers { flat, biome }) =
        runtime
    {
        // A level a mod opened with the default layers or with air alone.
        let inner = sunrise_client::flatgen::FlatTerrain::of_layers(
            pack.assets.registry,
            shape,
            *flat,
            *biome,
        )
        .ok_or("a runtime level's layers use a block the game lacks")?;
        register_sea_level(-63);
        generator_type = Identifier::parse("minecraft:flat").unwrap();
        Streaming::new(
            Arc::new(Persistent {
                inner: Arc::new(inner),
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else if let Some(flat) = flat {
        // `adjustGenerationSettings` decorates independently of generateStructures.
        let staged = FlatTerrain::uses_pipeline(&flat)
            .then(|| {
                VanillaTerrain::for_flat_level(
                    terrain_resources,
                    overrides,
                    seed,
                    &flat,
                    automatic_structures,
                    dim.id(),
                    Some(shape),
                )
                .map(|mut terrain| {
                    terrain.add_mod_signs(res.facts());
                    Arc::new(terrain)
                })
            })
            .transpose()
            .map_err(|e| format!("{}: cannot build flat dimension: {e}", dim.id()))?;
        register_sea_level(-63);
        generator_type = Identifier::parse("minecraft:flat").unwrap();
        locator = staged.clone();
        if let Some(inner) = staged.filter(|_| automatic_structures || flat.features || flat.lakes)
        {
            if automatic_structures {
                inner.search_rings_behind();
            }
            keep_structure_references(&inner, world_root, dim, &save);
            Streaming::new(
                Arc::new(Persistent {
                    inner,
                    save: save.clone(),
                    registry: pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                pack.context_with(biomes.clone()),
                res.threads(),
            )
        } else {
            let inner = FlatTerrain::from_decoded(&flat, pack.assets.registry, &biomes, shape)
                .ok_or_else(|| format!("{}: flat layers use an unregistered block", dim.id()))?;
            Streaming::new(
                Arc::new(Persistent {
                    inner: Arc::new(inner),
                    save: save.clone(),
                    registry: pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                pack.context_with(biomes.clone()),
                res.threads(),
            )
        }
    } else if let Some(inner) = debug {
        generator_type = Identifier::parse("minecraft:debug").unwrap();
        Streaming::new(
            Arc::new(Persistent {
                inner: Arc::new(inner),
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else if options.test_terrain && dim == Dim::Overworld {
        // The test terrain stands in for the noise generator its stem names.
        generator_type = Identifier::parse("minecraft:noise").unwrap();
        let inner = Arc::new(TestTerrain::new(pack.assets.registry, &biomes)?);
        Streaming::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else if let Some((level, id)) = mod_terrain(
        overrides,
        terrain_resources,
        mod_chunk_generators(),
        dim,
        shape,
        seed,
        automatic_structures,
        match runtime {
            Some(super::runtime_levels::Generate::Stem(stem)) => Some(stem),
            _ => None,
        },
        saved_stem,
    )
    .transpose()?
    {
        generator_type = id;
        match level {
            ModLevel::Terrain(inner) => Streaming::new(
                Arc::new(Persistent {
                    inner: Arc::new(inner),
                    save: save.clone(),
                    registry: pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                pack.context_with(biomes.clone()),
                res.threads(),
            ),
            ModLevel::Staged(mut inner) => {
                inner.add_mod_signs(res.facts());
                register_sea_level(inner.sea_level());
                let inner = Arc::new(inner);
                inner.load_structure_mobs_behind();
                keep_structure_references(&inner, world_root, dim, &save);
                locator = Some(inner.clone());
                Streaming::new(
                    Arc::new(Persistent {
                        inner,
                        save: save.clone(),
                        registry: pack.assets.clone(),
                        biomes: biomes.clone(),
                    }),
                    pack.context_with(biomes.clone()),
                    res.threads(),
                )
            }
        }
    } else {
        if runtime.is_some() {
            return Err(format!("{}: no generator builds this runtime level", dim.id()).into());
        }
        generator_type = Identifier::parse("minecraft:noise").unwrap();
        // WorldOptions.generateStructures, from the world's settings.
        let structures = automatic_structures;
        let preset = WorldGen::noise_preset(generator);
        let saved_stem = saved_stem.map(sunrise_client::scoreboard::tag_to_json);
        let regions = sunrise_client::save::biome_regions::layout(world_root, dim.id())
            .map_err(|e| format!("Failed to read the world's biome regions: {e}"))?;
        let mut inner = VanillaTerrain::for_world_with_settings(terrain_resources, overrides, seed, preset, dim.id(), structures, saved_stem.as_ref(), regions).map_err(|e| {
                if overrides.is_empty() {
                    format!("{e}\nPrepare the worldgen bundle with the launcher, or: sunrise-prepare --prepare")
                } else {
                    format!("{e}\nThe world's data packs change its world generation: one of them may not be made for this version")
                }
            })?;
        inner.add_mod_signs(res.facts());
        register_sea_level(inner.sea_level());
        let inner = Arc::new(inner);
        // The first mob a fresh chunk generates needs the mob data: read it now,
        // off the server's tick (#13857). The End's chunks generate none.
        if dim != Dim::End {
            inner.load_structure_mobs_behind();
        }
        if dim == Dim::Overworld && structures {
            inner.search_rings_behind();
        }
        // Chunks an older version saved: the new chunks around them blend in.
        if dim == Dim::Overworld {
            let old = Arc::new(sunrise_client::old_world::SavedOldWorld::new(
                save.clone(),
                pack.assets.clone(),
                res.biomes().clone(),
                inner.generator_data().clone(),
            ));
            // Saves of this folder take the old chunks' calculated blending data when
            // they are requested; a weak link, as the old world holds the folder.
            let weak = Arc::downgrade(&old);
            save.set_blending_lookup(Box::new(move |x, z| {
                weak.upgrade().and_then(|old| old.calculated(x, z))
            }));
            inner.set_old_world(old);
        }
        keep_structure_references(&inner, world_root, dim, &save);
        locator = Some(inner.clone());
        Streaming::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    };
    if options.execution_role.is_client() {
        streaming.load_forced(
            save.load_forced_chunks(),
            save.load_mod_tickets(mod_api::__private::game::ticket_controller_slot_by_id),
        );
    }
    streaming.radius = settings.render_distance;
    streaming.biome_blend = settings.biome_blend;
    streaming.smooth_lighting = settings.smooth_lighting;
    streaming.cutout_leaves = settings.cutout_leaves;
    streaming.opaque_ice = settings.sunrise.opaque_ice;
    streaming.leaf_culling = settings
        .sunrise
        .leaf_culling_effective(&settings.resource_packs);
    streaming.biome_zoom = BiomeZoom::new(obfuscate_seed(seed));
    let facts = sunrise_world::dimension_type::DimensionFacts::of_builtin_level(
        sunrise_core::LevelKey::builtin_or_overworld(dim.id()),
    );
    streaming.sky_light = facts.height.has_skylight;
    streaming.nether_shading =
        facts.cardinal_light == sunrise_world::dimension_type::CardinalLight::Nether;
    // A server draws nothing: neither the integrated server nor the
    // dedicated one meshes sections.
    streaming.mesh = options.execution_role.is_client();
    Ok((streaming, save, locator, generator_type))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_root_follows_the_server_role() {
        use execution_role::ExecutionRole::{DedicatedServer, IntegratedServer};
        use std::path::Path;

        let cwd = Path::new("server");
        let game = Path::new("game");
        assert_eq!(chunk_report_root(DedicatedServer, Some(game), cwd), cwd);
        assert_eq!(chunk_report_root(IntegratedServer, Some(game), cwd), game);
        assert_eq!(chunk_report_root(IntegratedServer, None, cwd), cwd);
    }

    #[test]
    fn unreadable_level_backup_refuses_before_biome_adoption() {
        use sunrise_client::save;
        use sunrise_core::nbt::Tag;
        use sunrise_world::worldgen::biome_source::regions::SelectorParams;

        let game = tempfile::tempdir().unwrap();
        let world = game.path().join("world");
        let record = world.join("data/sunrise/biome_regions.dat");
        let backup = world.join("level.dat_old");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&backup, b"unreadable backup").unwrap();
        std::fs::write(
            game.path().join("server.toml"),
            "[biome_regions]\nadopt_installed = true\n",
        )
        .unwrap();
        let selector = SelectorParams::current();
        let layout = Tag::compound([
            (
                "selector",
                Tag::compound([
                    ("version", Tag::Int(selector.version as i32)),
                    ("cell", Tag::Int(selector.cell as i32)),
                    ("salt", Tag::String(selector.salt.as_str().into())),
                    ("vanilla_weight", Tag::Int(selector.vanilla_weight as i32)),
                ]),
            ),
            ("regions", Tag::List(Vec::new())),
        ]);
        save::write_dat(
            &record,
            Tag::compound([
                ("version", Tag::Int(1)),
                (
                    "dimensions",
                    Tag::compound([("minecraft:overworld", layout)]),
                ),
            ]),
        )
        .unwrap();
        assert!(save::biome_regions::adoptable(&world));
        let before = std::fs::read(&record).unwrap();
        let role = execution_role::ExecutionRole::DedicatedServer;
        assert!(prepare_level_data(role, &world, "world", game.path()).is_err());
        assert!(!world.join("level.dat").exists());
        assert_eq!(std::fs::read(&record).unwrap(), before);
        assert_eq!(std::fs::read(&backup).unwrap(), b"unreadable backup");

        // The explicit opt-in does change this layout when recovery can proceed.
        save::write_dat(
            &backup,
            Tag::compound([("Data", Tag::Compound(Default::default()))]),
        )
        .unwrap();
        prepare_level_data(role, &world, "world", game.path()).unwrap();
        assert_ne!(std::fs::read(&record).unwrap(), before);
    }

    #[test]
    #[ignore = "daily"]
    fn recovered_world_uses_the_pack_flat_generator_before_its_first_save() {
        use execution_role::ExecutionRole::DedicatedServer;
        use std::fs;
        use sunrise_client::save;

        let scratch = tempfile::tempdir().unwrap();
        let world = scratch.path();
        Level::default().write_without_player(world).unwrap();
        fs::rename(world.join("level.dat"), world.join("level.dat_old")).unwrap();
        let settings_path = world.join("data/minecraft/world_gen_settings.dat");
        fs::write(&settings_path, b"unreadable generator settings").unwrap();
        let pack = world.join("datapacks/recovery");
        let dimensions = pack.join("data/minecraft/dimension");
        fs::create_dir_all(&dimensions).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Recovery test","min_format":[121,0],"max_format":[121,0]}}"#,
        )
        .unwrap();
        fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/recovery"],"disabled":[]}"#,
        )
        .unwrap();
        fs::write(
            dimensions.join("overworld.json"),
            r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:flat","settings":{
                "biome":"minecraft:plains","features":false,"lakes":false,"structure_overrides":[],
                "layers":[{"height":1,"block":"minecraft:bedrock"},{"height":2,"block":"minecraft:diamond_block"}]}}}"#,
        ).unwrap();

        let resources = std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap().into();
        crate::game::native_entity_refs::tests::with_game_role(
            resources,
            DedicatedServer,
            |game| {
                let mut request = WorldRequest::new(
                    "recovery",
                    None,
                    Some(54_321),
                    false,
                    None,
                    Level::default().rule_registry,
                )
                .unwrap();
                request.save_root = world.to_owned();
                game.load_world(request).unwrap();
                assert_eq!(
                    fs::read(&settings_path).unwrap(),
                    b"unreadable generator settings"
                );
                let y = game.world.shape.min_y + 1;
                assert!(
                    game.streaming
                        .set_forced_loaded(&mut game.world, 32, 32, true)
                );
                let chunk = &game.world.chunks[&(32, 32)];
                assert_eq!(
                    Some(chunk.get(0, y, 0)),
                    game.pack
                        .assets
                        .registry
                        .default_state("minecraft:diamond_block")
                        .map(|id| id.0)
                );

                // The spawn search sees the pack's flat layers before the first save.
                assert_eq!(
                    game.spawn_origin().unwrap(),
                    ((0, 0), game.world.shape.min_y + 3)
                );

                game.level.write_without_player(world).unwrap();
                assert_eq!(
                    save::world_gen_settings(world).as_ref(),
                    game.level
                        .generator
                        .as_ref()
                        .map(|generator| generator.settings()),
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn custom_flat_stems_validate_and_generate_pack_decoration() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::fs;
        use sunrise_client::chunk::LevelShape;

        const CASE: &str = "SUNRISE_CUSTOM_FLAT_STEM_CASE";
        if run_fresh_cases(
            "game::world_loading::tests::custom_flat_stems_validate_and_generate_pack_decoration",
            CASE,
            &["invalid", "decorated"],
        ) {
            return;
        }
        let invalid = std::env::var(CASE).unwrap() == "invalid";
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pack = root.join("datapacks/flat");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Flat stem fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/flat"],"disabled":[]}"#,
        )
        .unwrap();
        // FlatLevelGeneratorSettings.validateHeight rejects one above the limit.
        let height = if invalid { 4065 } else { 1 };
        write(
            "data/test/dimension/flat.json",
            &format!(
                r#"{{"type":"minecraft:the_nether","generator":{{"type":"minecraft:flat","settings":{{
            "biome":"test:glade","features":true,"lakes":false,"structure_overrides":[],
            "layers":[{{"height":{height},"block":"minecraft:bedrock"}}]}}}}}}"#
            ),
        );
        write(
            "data/test/worldgen/biome/glade.json",
            r#"{"temperature":0.8,"downfall":0.4,"has_precipitation":true,"effects":{},"carvers":[],"features":[["test:decoration"]]}"#,
        );
        write(
            "data/test/worldgen/feature/decoration.json",
            r#"{"type":"minecraft:fill_layer","height":4,"state":{"id":"minecraft:gold_block"}}"#,
        );
        write(
            "data/test/worldgen/placed_feature/decoration.json",
            r#"{"feature":"test:decoration","placement":[]}"#,
        );

        with_game(|game| {
            let shape = LevelShape::NETHER;
            let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = super::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let enabled: Vec<_> = stack.enabled_packs().collect();
            let conditions = super::load_conditions::worldgen_load_conditions(
                &base_conditions,
                &game.options.resources,
                &enabled,
            );
            let (overrides, _) = worldgen_overrides_of(&data, &conditions).unwrap();
            let packed: Vec<(String, serde_json::Value)> = overrides
                .entries("biomes")
                .filter_map(|(id, json)| Some((id.to_owned(), serde_json::from_str(json).ok()?)))
                .collect();
            let biomes = if packed.is_empty() {
                game.res.biomes().clone()
            } else {
                Arc::new(
                    game.res
                        .biomes()
                        .with_packs(packed.iter().map(|(i, v)| (i.as_str(), v)))
                        .unwrap(),
                )
            };
            let built = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.settings,
                root,
                Dim::Extra(mod_api::__private::dimension_key("test:flat")),
                shape,
                29,
                &game.level,
                biomes.clone(),
                &overrides,
                None,
                game.save.chunk_reports().clone(),
                game.save.save_failure_reporter().clone(),
            );
            if invalid {
                let Err(error) = built else {
                    panic!("a height outside the flat codec must reject this stem");
                };
                assert!(
                    error
                        .to_string()
                        .starts_with("test:flat: cannot decode flat dimension:")
                );
                return;
            }
            let (mut streaming, _, locator, generator_type) = built.unwrap();
            assert_eq!(generator_type.as_str(), "minecraft:flat");
            assert_eq!(
                locator.unwrap().noise_biomes(&[[0, 0, 0]]),
                [Some("test:glade".into())]
            );
            let mut world = World::new(game.res.facts().air, 29);
            world.shape = shape;
            assert!(streaming.set_forced_loaded(&mut world, 0, 0, true));
            let chunk = &world.chunks[&(0, 0)];
            let gold = game
                .pack
                .assets
                .registry
                .default_state("minecraft:gold_block")
                .unwrap();
            let bedrock = game
                .pack
                .assets
                .registry
                .default_state("minecraft:bedrock")
                .unwrap();
            assert_eq!(chunk.get(0, 0, 0), bedrock.0);
            assert_eq!(chunk.get(0, 4, 0), gold.0);
            assert_eq!(biomes.saved_name(chunk.biome(0, 4, 0)), "test:glade");
        });
    }

    /// Report the generator actually built: saved Debug beats a pack's noise stem, an unknown
    /// generator cannot build a custom level, and a runtime level reports its bound mod generator.
    #[test]
    #[ignore = "daily"]
    fn a_level_reports_the_generator_it_was_built_with() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::fs;
        use sunrise_client::chunk::LevelShape;
        use sunrise_client::save::{WorldGen, WorldGenerator, WorldPreset};

        const CASE: &str = "SUNRISE_LEVEL_GENERATOR_TYPE_CASE";
        if run_fresh_cases(
            "game::world_loading::tests::a_level_reports_the_generator_it_was_built_with",
            CASE,
            &["debug", "unregistered", "runtime"],
        ) {
            return;
        }
        let case = std::env::var(CASE).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        if case == "runtime" {
            use mod_api::{RuntimeGenerator, RuntimeLevel, RuntimeLevelError};
            use sunrise_world::worldgen::mod_generators::ChunkAccess;

            struct Floor;
            impl ModChunkGenerator for Floor {
                fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                    let pos = chunk.pos();
                    let gold = chunk.state("minecraft:gold_block");
                    assert!(chunk.set_block([pos.x * 16, chunk.min_y(), pos.z * 16], gold));
                }
            }

            let id = Identifier::parse("minecraft:fixture_floor").unwrap();
            let generators = ModChunkGenerators::from([(
                id.clone(),
                Arc::new(Floor) as Arc<dyn ModChunkGenerator>,
            )]);
            with_game(|game| {
                game.world_root = root.to_owned();
                let open = |game: &mut Game<'_>, name: &str| {
                    game.runtime_level_queue_open(
                        "test:runtime_floor",
                        RuntimeLevel::new(
                            "minecraft:overworld",
                            RuntimeGenerator::Mod(name.into()),
                            29,
                        ),
                        &generators,
                    )
                };
                for name in [
                    "fixture_floor",
                    ":fixture_floor",
                    "minecraft:BAD",
                    "test:unregistered",
                ] {
                    assert_eq!(
                        open(game, name),
                        Err(RuntimeLevelError::UnknownGenerator),
                        "{name}"
                    );
                }
                let level = open(game, id.as_str()).unwrap();
                game.apply_runtime_levels();
                assert_eq!(level.generator_type(game), Some(id));
                level.load_chunk(game, [0, 0]).unwrap();
                let world = game.native_level_world(level).unwrap();
                let gold = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:gold_block")
                    .unwrap();
                assert_eq!(world.block(0, world.shape.min_y, 0), gold);
            });
            return;
        }
        let debug = case == "debug";
        let pack = root.join("datapacks/stems");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Generator stem fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/stems"],"disabled":[]}"#,
        )
        .unwrap();
        write(
            "data/minecraft/dimension/overworld.json",
            r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise","settings":"minecraft:overworld",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
        );
        write(
            "data/test/dimension/odd.json",
            r#"{"type":"minecraft:overworld","generator":{"type":"test:missing",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
        );

        with_game(|game| {
            let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = super::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let enabled: Vec<_> = stack.enabled_packs().collect();
            let conditions = super::load_conditions::worldgen_load_conditions(
                &base_conditions,
                &game.options.resources,
                &enabled,
            );
            let (overrides, _) = worldgen_overrides_of(&data, &conditions).unwrap();
            let settings = tempfile::tempdir().unwrap();
            let preset = if debug {
                WorldPreset::Debug
            } else {
                WorldPreset::Normal
            };
            let options = WorldGen {
                preset,
                ..WorldGen::default()
            };
            sunrise_client::save::write_world_gen_settings(settings.path(), 1, options).unwrap();
            game.level.generator = WorldGenerator::read_new(settings.path());
            assert!(game.level.generator.is_some());
            let dim = if debug {
                Dim::Overworld
            } else {
                Dim::Extra(mod_api::__private::dimension_key("test:odd"))
            };
            let built = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.settings,
                root,
                dim,
                LevelShape::OVERWORLD,
                29,
                &game.level,
                game.res.biomes().clone(),
                &overrides,
                None,
                game.save.chunk_reports().clone(),
                game.save.save_failure_reporter().clone(),
            );
            if debug {
                let (_, _, _, generator_type) = built.unwrap();
                assert_eq!(generator_type.as_str(), "minecraft:debug");
            } else {
                let Err(error) = built else {
                    panic!("no registered generator builds test:missing");
                };
                assert!(
                    error.to_string().contains("is not a noise generator"),
                    "{error}"
                );
            }
        });
    }

    /// A pack's new biome opens with the world: the packed table gives it an id past the
    /// built-in ones, and the overworld stem's fixed biome source generates a chunk of it
    /// (`RegistryDataLoader` loads every pack biome, none is left out).
    #[test]
    #[ignore = "daily"]
    fn a_pack_biome_no_built_in_has_generates_in_the_overworld() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::fs;
        use sunrise_client::chunk::LevelShape;

        const CASE: &str = "SUNRISE_NEW_BIOME_WORLD_CASE";
        if run_fresh_cases(
            "game::world_loading::tests::a_pack_biome_no_built_in_has_generates_in_the_overworld",
            CASE,
            &["overworld"],
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pack = root.join("datapacks/biome");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"New biome fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/biome"],"disabled":[]}"#,
        )
        .unwrap();
        write(
            "data/minecraft/dimension/overworld.json",
            r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise","settings":"minecraft:overworld",
            "biome_source":{"type":"minecraft:fixed","biome":"test:new_biome"}}}"#,
        );
        with_game(|game| {
            // A copy of the game's plains with its own water colour.
            let mut file = game
                .res
                .biomes()
                .builtin_file("minecraft:plains")
                .unwrap()
                .clone();
            file["effects"]["water_color"] = serde_json::json!("#123456");
            write("data/test/worldgen/biome/new_biome.json", &file.to_string());

            let builtin = game.res.biomes();
            assert!(!builtin.contains("test:new_biome"));
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = super::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let (overrides, _) =
                worldgen_overrides_of(&data, &sunrise_world::conditions::LoadConditions::vanilla())
                    .unwrap();
            let packed: Vec<(String, serde_json::Value)> = overrides
                .entries("biomes")
                .filter_map(|(id, json)| Some((id.to_owned(), serde_json::from_str(json).ok()?)))
                .collect();
            let biomes = Arc::new(
                builtin
                    .with_packs(packed.iter().map(|(i, v)| (i.as_str(), v)))
                    .unwrap(),
            );
            let id = biomes
                .try_id("test:new_biome")
                .expect("the packed table knows the biome");
            assert!(usize::from(id) >= builtin.registry_order().len());
            assert_eq!(biomes.name(id), "test:new_biome");

            let shape = LevelShape::OVERWORLD;
            let (mut streaming, _, locator, _) = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.settings,
                root,
                Dim::Overworld,
                shape,
                7,
                &game.level,
                biomes.clone(),
                &overrides,
                None,
                game.save.chunk_reports().clone(),
                game.save.save_failure_reporter().clone(),
            )
            .unwrap();
            if let Some(locator) = locator {
                assert_eq!(
                    locator.noise_biomes(&[[0, 0, 0]]),
                    [Some("test:new_biome".into())]
                );
            }
            let mut world = World::new(game.res.facts().air, 7);
            world.shape = shape;
            assert!(streaming.set_forced_loaded(&mut world, 0, 0, true));
            let chunk = &world.chunks[&(0, 0)];
            for y in [shape.min_y, 0, 64, 200] {
                assert_eq!(
                    biomes.saved_name(chunk.biome(0, y, 0)),
                    "test:new_biome",
                    "y {y}"
                );
            }
        });
    }

    /// The biome names of the first chunk of the overworld a world with the data pack `files`
    /// (path under the pack, text) generates, one per quarter of the chunk at sea level.
    fn overworld_biomes_with_pack(files: &[(&str, &str)]) -> Vec<String> {
        use crate::game::native_entity_refs::tests::with_game;
        use std::fs;
        use sunrise_client::chunk::LevelShape;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pack = root.join("datapacks/preset");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Preset fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/preset"],"disabled":[]}"#,
        )
        .unwrap();
        for (path, text) in files {
            write(path, text);
        }
        let mut names = Vec::new();
        with_game(|game| {
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = super::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let (overrides, _) =
                worldgen_overrides_of(&data, &sunrise_world::conditions::LoadConditions::vanilla())
                    .unwrap();
            let packed: Vec<(String, serde_json::Value)> = overrides
                .entries("biomes")
                .filter_map(|(id, json)| Some((id.to_owned(), serde_json::from_str(json).ok()?)))
                .collect();
            let biomes = Arc::new(
                game.res
                    .biomes()
                    .with_packs(packed.iter().map(|(i, v)| (i.as_str(), v)))
                    .unwrap(),
            );
            let shape = LevelShape::OVERWORLD;
            let (mut streaming, _, _, _) = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.settings,
                root,
                Dim::Overworld,
                shape,
                7,
                &game.level,
                biomes.clone(),
                &overrides,
                None,
                game.save.chunk_reports().clone(),
                game.save.save_failure_reporter().clone(),
            )
            .unwrap();
            let mut world = World::new(game.res.facts().air, 7);
            world.shape = shape;
            assert!(streaming.set_forced_loaded(&mut world, 0, 0, true));
            let chunk = &world.chunks[&(0, 0)];
            names = [0, 5, 10, 15]
                .into_iter()
                .map(|x| biomes.saved_name(chunk.biome(x, 64, x)).into_owned())
                .collect();
        });
        names
    }

    /// A pack's file for `minecraft:normal` is the world preset a world created with the default
    /// preset generates (`RegistryDataLoader` over `WORLD_REGISTRIES`: `world_preset`).
    #[test]
    #[ignore = "daily"]
    fn a_pack_world_preset_replaces_the_default_presets_overworld() {
        const CASE: &str = "SUNRISE_PRESET_WORLD_CASE";
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::world_loading::tests::a_pack_world_preset_replaces_the_default_presets_overworld",
            CASE,
            &["overworld"],
        ) {
            return;
        }
        let names = overworld_biomes_with_pack(&[(
            "data/minecraft/worldgen/world_preset/normal.json",
            r#"{"dimensions":{"minecraft:overworld":{"type":"minecraft:overworld","generator":{
            "type":"minecraft:noise","settings":"minecraft:overworld",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:cherry_grove"}}}}}"#,
        )]);
        assert_eq!(names, ["minecraft:cherry_grove"; 4]);
    }

    /// A level stem that names a pack's multi-noise parameter list takes the table of the
    /// built-in preset the list's file names (`MultiNoiseBiomeSourceParameterList`): here the
    /// Nether's, so the overworld generates Nether biomes.
    #[test]
    #[ignore = "daily"]
    fn a_level_stem_naming_a_pack_parameter_list_generates_per_that_list() {
        const CASE: &str = "SUNRISE_PARAMETER_LIST_WORLD_CASE";
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::world_loading::tests::a_level_stem_naming_a_pack_parameter_list_generates_per_that_list",
            CASE,
            &["overworld"],
        ) {
            return;
        }
        let names = overworld_biomes_with_pack(&[
            (
                "data/test/worldgen/multi_noise_biome_source_parameter_list/mine.json",
                r#"{"preset":"minecraft:nether"}"#,
            ),
            (
                "data/minecraft/dimension/overworld.json",
                r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise",
                "settings":"minecraft:overworld",
                "biome_source":{"type":"minecraft:multi_noise","preset":"test:mine"}}}"#,
            ),
        ]);
        let nether = [
            "minecraft:nether_wastes",
            "minecraft:soul_sand_valley",
            "minecraft:crimson_forest",
            "minecraft:warped_forest",
            "minecraft:basalt_deltas",
        ];
        assert!(
            names.iter().all(|name| nether.contains(&name.as_str())),
            "{names:?}"
        );
    }

    /// A world whose level.dat does not read but whose level.dat_old does: only the
    /// dedicated server restores it unasked. Every role here carries the launch configuration
    /// (`options.dedicated`) a dedicated server and the integrated server thread both have, so
    /// it cannot be what decides.
    #[test]
    fn only_the_dedicated_role_takes_level_dat_old_unasked() {
        use execution_role::ExecutionRole::{Client, DedicatedServer, IntegratedServer};
        for (role, restored) in [
            (DedicatedServer, true),
            (IntegratedServer, false),
            (Client, false),
        ] {
            let dir = std::env::temp_dir().join(format!(
                "sunrise-level-dat-old-{role:?}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Level {
                name: "Recovered".into(),
                ..Level::default()
            }
            .write(&dir, &Default::default())
            .unwrap();
            std::fs::copy(dir.join("level.dat"), dir.join("level.dat_old")).unwrap();
            std::fs::write(dir.join("level.dat"), b"not a level").unwrap();
            let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
            options.dedicated = Some(Vec::new());
            options.execution_role = role;

            fall_back_to_level_dat_old_without_asking(options.execution_role, &dir, "world");

            assert_eq!(
                sunrise_client::save::level_dat_issue(&dir, false).is_none(),
                restored,
                "{role:?}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_levels_sea_level_comes_from_its_mod_or_its_noise_settings() {
        use sunrise_client::datapack::worldgen_overrides_over;
        use sunrise_world::worldgen::overrides::ModDimension;
        let level = |id: &str, stem: &str, sea_level| ModDimension {
            id: id.into(),
            stem: stem.into(),
            dimension_type: "{}".into(),
            sea_level,
        };
        let mods = [
            level(
                "test:own",
                r#"{"type":"test:own","generator":{"type":"test:floor"}}"#,
                Some(10),
            ),
            level(
                "test:inline",
                r#"{"type":"test:inline","generator":{"type":"minecraft:noise","settings":{"sea_level":20}}}"#,
                None,
            ),
            level(
                "test:plain",
                r#"{"type":"test:plain","generator":{"type":"test:floor"}}"#,
                None,
            ),
        ];
        let (overrides, _) = worldgen_overrides_over(
            &mods,
            &Default::default(),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        set_level_sea_levels(&overrides, &mods, std::path::Path::new("."));
        let sea = |id: &str| {
            sunrise_world::level::sea_level_of(sunrise_core::LevelKey::parse(id).unwrap())
        };
        assert_eq!(sea("test:own"), 10);
        assert_eq!(sea("test:inline"), 20);
        assert_eq!(sea("test:plain"), 63);
        assert_eq!(sea("minecraft:the_nether"), 32);
        sunrise_world::level::set_sea_levels(Vec::new());
    }

    #[test]
    #[ignore = "daily"]
    fn creation_rule_selection_reaches_the_saved_world() {
        use super::super::native_entity_refs::tests::with_game;
        use sunrise_client::gamerules::{GameRules, Rule, RuleRegistry, Value};
        let rule = Rule {
            name: "example:enabled".into(),
            category: "example:machines".into(),
            default: Value::Bool(true),
            min: 0,
            max: 0,
        };
        let registry =
            Arc::new(RuleRegistry::with_mod_rules(&["example:machines".into()], &[rule]).unwrap());
        let mut create = sunrise_client::title::WorldOptions {
            rules: GameRules::with_registry(registry.clone(), &[]),
            ..Default::default()
        };
        assert!(create.rules.set("example:enabled", Value::Bool(false)));
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("creation-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        with_game(|game| {
            let request = WorldRequest {
                owner: None,
                chunk_reports: sunrise_client::chunk_io::PROCESS_REPORTS.clone(),
                stem_order: None,
                save_root: dir.clone(),
                folder: "test".into(),
                name: Some("test".into()),
                seed: Some(1),
                creative: true,
                create: Some(create.clone()),
                rule_registry: registry.clone(),
            };
            assert!(Arc::ptr_eq(
                request.create.as_ref().unwrap().rules.registry(),
                &request.rule_registry
            ));
            game.load_world(request).unwrap();
            assert_eq!(
                game.ui.rules.get("example:enabled"),
                Some(Value::Bool(false))
            );
            assert!(Arc::ptr_eq(&game.level.rule_registry, &registry));
            game.save_game_with(true).unwrap();
        });
        assert_eq!(
            GameRules::load_with_registry(&dir, &[], registry).get("example:enabled"),
            Some(Value::Bool(false))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A minimal direct-codec biome used by the world loader fixtures.
    const PACK_BIOME: &str = r##"{"temperature":0.8,"downfall":0.4,"has_precipitation":true,"effects":{"water_color":"#2301ef"},"carvers":[],"features":[]}"##;

    fn biome_registry_fixture() -> (
        tempfile::TempDir,
        Arc<Biomes>,
        &'static sunrise_world::registry::BlockRegistry,
        crate::game::functions::PackStack,
    ) {
        use sunrise_assets::resource_source::ResourceRoot;
        use sunrise_client::datapack::DataPack;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("data/minecraft/worldgen/biome");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(root.path().join("worldgen")).unwrap();
        for name in ["fixture", "plains"] {
            std::fs::write(dir.join(format!("{name}.json")), PACK_BIOME).unwrap();
        }
        let resources = ResourceRoot::directory(root.path().to_owned());
        let base = Arc::new(Biomes::load_for_server(&resources).unwrap());
        let blocks: &'static _ = Box::leak(Box::new(sunrise_world::registry::BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:air":0,"minecraft:cave_air":1,"minecraft:void_air":2},"states":[["a",true,false,"","minecraft:air"],["c",true,false,"","minecraft:cave_air"],["v",true,false,"","minecraft:void_air"]]}"#.as_slice(),
        ).unwrap()));
        let stack = crate::game::functions::PackStack {
            builtin: None,
            content: DataPack::from_source("sunrise-content".into(), resources.data().clone()),
            packs: Vec::new(),
            enabled: Vec::new(),
            features: Vec::new(),
        };
        (root, base, blocks, stack)
    }

    #[test]
    fn biome_loader_checks_winning_conditions_and_decodes_passing_payloads() {
        use serde_json::{Value, json};
        use sunrise_world::{conditions::LoadConditions, loot::PackOverlay};
        let (root, base, blocks, stack) = biome_registry_fixture();
        std::fs::write(
            root.path().join("worldgen/placed_features.json"),
            json!({"demo:malformed": "{}", "demo:dangling": r#"{"feature":"demo:missing","placement":[]}"#}).to_string(),
        ).unwrap();
        let tags = root.path().join("data/demo/tags/worldgen/placed_feature");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(
            tags.join("malformed.json"),
            r#"{"values":["demo:malformed"]}"#,
        )
        .unwrap();
        let unknown = |condition| json!({"neoforge:conditions":[{"type":condition}]});
        let load = |winner: Value, conditions: &LoadConditions| {
            let files = [(
                "worldgen/biome/minecraft:fixture".into(),
                vec![
                    unknown("absent:loser").to_string().into_bytes(),
                    winner.to_string().into_bytes(),
                ],
            )];
            WorldBiomeRegistry::load(
                &base,
                root.path(),
                blocks,
                &stack,
                &stack.data_packs(Vec::new()),
                &PackOverlay {
                    files: &files,
                    conditions,
                    ..PackOverlay::default()
                },
                &Default::default(),
            )
        };

        let conditions = LoadConditions::new([]);
        let refused = load(unknown("absent:wire_biome"), &conditions).unwrap();
        assert!(!refused.table.contains("minecraft:fixture"));
        assert_eq!(
            conditions.check_errors().unwrap_err(),
            "data/minecraft/worldgen/biome/fixture.json: unknown load conditions [absent:wire_biome]"
        );

        for (field, invalid, diagnostic) in [
            (
                "effects",
                Value::Null,
                "invalid world generation field: effects",
            ),
            (
                "attributes",
                json!({"minecraft:visual/ambient_particles":1}),
                "minecraft:visual/ambient_particles",
            ),
            ("carvers", json!({}), "carvers:"),
            (
                "carvers",
                json!({"type":"neoforge:and","values":[[{"type":"minecraft:missing"}],[]]}),
                "world carver type",
            ),
            (
                "features",
                json!([["demo:unknown"]]),
                "unknown placed feature demo:unknown",
            ),
            (
                "features",
                json!([["demo:malformed"]]),
                "placed feature demo:malformed",
            ),
            (
                "features",
                json!(["#demo:malformed"]),
                "placed feature demo:malformed",
            ),
            (
                "features",
                json!([["demo:dangling"]]),
                "unknown feature reference demo:missing",
            ),
            (
                "features",
                json!([{"type":"neoforge:any"}]),
                "placed feature demo:dangling",
            ),
            (
                "features",
                json!([{"type":"neoforge:and","values":[["demo:malformed"],[]]}]),
                "placed feature demo:malformed",
            ),
        ] {
            let mut payload: Value = serde_json::from_str(PACK_BIOME).unwrap();
            payload[field] = invalid;
            let conditions = LoadConditions::new([]);
            let error = load(
                json!({"neoforge:conditions":[{"type":"neoforge:always"}],"neoforge:value":payload}),
                &conditions,
            )
            .err()
            .expect("a passing condition does not excuse an invalid biome");
            let error = error.to_string();
            assert!(error.starts_with("biome minecraft:fixture:"), "{error}");
            assert!(error.contains(diagnostic), "{error}");
            assert_eq!(conditions.check_errors(), Ok(()));
        }
    }

    #[test]
    fn wrapped_biome_resolves_custom_holder_sets_and_inline_registry_predicates() {
        use serde_json::{Value, json};
        use sunrise_world::{conditions::LoadConditions, loot::PackOverlay};

        let (root, base, blocks, stack) = biome_registry_fixture();
        std::fs::write(
            root.path().join("worldgen/fluid_tags.json"),
            json!({"demo:wet": r#"{"values":["minecraft:water"]}"#}).to_string(),
        )
        .unwrap();
        let feature = json!({"type":"minecraft:no_op", "config": {}});
        std::fs::write(
            root.path().join("worldgen/placed_features.json"),
            json!({"demo:no_op": json!({"feature":feature.clone(), "placement":[]}).to_string()})
                .to_string(),
        )
        .unwrap();
        let mut payload: Value = serde_json::from_str(PACK_BIOME).unwrap();
        payload["features"] = json!([
            {"type":"neoforge:any"},
            [{"feature":feature,"placement":[
                {"type":"minecraft:block_predicate_filter", "predicate": {
                    "type":"minecraft:matching_biomes", "biomes":["minecraft:plains", "demo:incoming"]
                }},
                {"type":"minecraft:block_predicate_filter", "predicate": {
                    "type":"minecraft:matching_fluids", "fluids":"#demo:wet"
                }}
            ]}]
        ]);
        payload["carvers"] = json!({"type":"neoforge:any"});
        let files = [(
            "worldgen/biome/demo:incoming".into(),
            vec![
                json!({
                    "neoforge:conditions":[{"type":"neoforge:always"}], "neoforge:value":payload
                })
                .to_string()
                .into_bytes(),
            ],
        )];
        let conditions = LoadConditions::new([]);
        let admitted = WorldBiomeRegistry::load(
            &base,
            root.path(),
            blocks,
            &stack,
            &stack.data_packs(Vec::new()),
            &PackOverlay {
                files: &files,
                conditions: &conditions,
                ..PackOverlay::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert!(admitted.table.contains("demo:incoming"));
        assert_eq!(conditions.check_errors(), Ok(()));
    }

    #[test]
    fn the_saved_biome_tests_pack_biome_decodes() {
        let climate =
            sunrise_world::worldgen::biome_temperature::BiomeTemperatureSettings::from_json(
                PACK_BIOME,
            )
            .unwrap();
        assert!(climate.has_precipitation);
        assert_eq!(climate.base_temperature, 0.8);
        assert_eq!(climate.downfall, 0.4);
        let value: serde_json::Value = serde_json::from_str(PACK_BIOME).unwrap();
        assert_eq!(value["carvers"], serde_json::json!([]));
    }

    #[test]
    #[ignore = "daily"]
    fn a_loaded_missing_biome_survives_a_game_save_with_an_enabled_biome_pack() {
        use super::super::native_entity_refs::tests::with_game;
        let root = std::env::temp_dir().join(format!(
            "saved-biome-pack-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pack = root.join("datapacks/biomes");
        let entries = pack.join("data/test/worldgen/biome");
        std::fs::create_dir_all(&entries).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Biome fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        )
        .unwrap();
        std::fs::write(entries.join("present.json"), PACK_BIOME).unwrap();
        Level {
            data_packs: Some((vec!["vanilla".into(), "file/biomes".into()], vec![])),
            ..Level::default()
        }
        .write(&root, &Default::default())
        .unwrap();
        with_game(|game| {
            let before = game.res.biomes().with_packs([]).unwrap();
            let mut chunk = sunrise_client::chunk::Chunk::empty(8, 8);
            chunk.set_biome_cell(1, 20, 2, before.saved_id("gone_mod:highland"));
            let save = SaveDir::new(
                root.clone(),
                sunrise_client::chunk::LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            save.save(&[Arc::new(chunk)], game.pack.assets.registry, &before)
                .unwrap();
            drop(save);
            let mut request = WorldRequest::new(
                "test",
                Some("test"),
                Some(1),
                true,
                None,
                game.level.rule_registry.clone(),
            )
            .unwrap();
            request.save_root = root.clone();
            game.load_world(request).unwrap();
            assert!(world_biomes!(game).contains("test:present"));
            game.streaming
                .set_forced_loaded(&mut game.world, 8, 8, true);
            let chunk = &game.world.chunks[&(8, 8)];
            let id = chunk.biome(4, 16, 8);
            assert_eq!(world_biomes!(game).saved_name(id), "gone_mod:highland");
            game.world.modified.insert((8, 8));
            game.save_level(true).unwrap();
            let reloaded = game
                .save
                .load(8, 8, game.pack.assets.registry, world_biomes!(game))
                .unwrap();
            assert_eq!(
                world_biomes!(game).saved_name(reloaded.biome(4, 16, 8)),
                "gone_mod:highland"
            );
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    /// NeoForge's `ModMismatchEvent`: a world saved with version 1 of a mod and
    /// opened with version 2 fires `mod_mismatch` once with both versions, a
    /// mod no longer loaded with no current version; the mod that marks its
    /// own mismatch resolved gets no warning while the missing one does. The
    /// save records the loaded list, so the reopened world fires nothing, as a
    /// world without the list doesn't.
    #[test]
    #[ignore = "daily"]
    fn a_world_saved_with_other_mod_versions_fires_mod_mismatch() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        // The warning is read back from the process's logger and its last 50 lines. In the shared
        // test process another test may own the global logger, or flood those lines: this runs
        // in a process of its own.
        if run_fresh_cases(
            "game::world_loading::tests::a_world_saved_with_other_mod_versions_fires_mod_mismatch",
            "SUNRISE_MOD_MISMATCH_CHILD",
            &["alone"],
        ) {
            return;
        }
        use mod_api::{Hooks, HostSide, ModEntry, ModMismatch, ModMismatchEvent};
        use std::sync::{Arc, Mutex};
        use sunrise_client::save::{
            Level,
            level_mods::{ModRecord, set_loaded},
        };

        struct Migrating(Arc<Mutex<Vec<Vec<ModMismatch>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Migrating {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                self.0.lock().unwrap().push(event.mismatches.to_vec());
                event.mark_resolved("mismatch_test");
            }
        }
        struct Duplicate;
        #[mod_api::__subscriptions]
        impl Hooks for Duplicate {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert!(event.was_resolved("mismatch_test"));
                assert_eq!(event.resolver("mismatch_test"), Some("mismatch_test"));
                event.mark_resolved("mismatch_test");
            }
        }
        struct Observing;
        #[mod_api::__subscriptions]
        impl Hooks for Observing {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert_eq!(event.resolver("mismatch_test"), Some("mismatch_test"));
                assert_eq!(event.resolved()[0].resolver, "mismatch_test");
            }
        }
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("mod-mismatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sunrise_client::logging::init(&dir, "main");
        let record = |id: &str, version: &str| ModRecord {
            id: id.into(),
            version: version.into(),
        };
        let mismatch = |mod_id: &str, saved: &str, current: Option<&str>| ModMismatch {
            mod_id: mod_id.into(),
            saved_version: saved.into(),
            current_version: current.map(str::to_owned),
        };
        let request = |root: &std::path::Path| {
            let mut request = WorldRequest::new(
                "test",
                Some("test"),
                Some(1),
                true,
                None,
                Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = root.to_owned();
            request
        };
        let warnings = || -> Vec<String> {
            sunrise_client::logging::recent_lines()
                .into_iter()
                .filter(|line| line.contains("Sunrise: mod "))
                .collect()
        };
        let heard = Arc::new(Mutex::new(Vec::new()));
        let level = || Level {
            name: "test".into(),
            ..Level::default()
        };
        // The loaded list is the whole process's: the test's goes away when it ends, even
        // when it fails, so the tests after it save no list.
        struct Restore(Option<Vec<ModRecord>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_loaded(self.0.take());
            }
        }
        // Before the loaded mods are known: a world without the list.
        let _restore = Restore(set_loaded(None));
        let plain = dir.join("plain");
        level().write(&plain, &Default::default()).unwrap();
        assert_eq!(Level::read(&plain).unwrap().saved_mods, None);
        // Saved with version 1 of the mod, and with one since gone.
        let changed = dir.join("changed");
        set_loaded(Some(vec![
            record("mismatch_test", "1.0.0"),
            record("gone", "0.5.0"),
        ]));
        level().write(&changed, &Default::default()).unwrap();
        set_loaded(Some(vec![record("mismatch_test", "2.0.0")]));
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new("mismatch_test", Box::new(Migrating(heard.clone()))),
                ModEntry::new("duplicate", Box::new(Duplicate)),
                ModEntry::new("observing", Box::new(Observing)),
            ]
            .into(),
            || {
                with_game(|game| {
                    game.load_world(request(&changed)).unwrap();
                    let lines = warnings();
                    assert_eq!(
                        *heard.lock().unwrap(),
                        vec![vec![
                            mismatch("mismatch_test", "1.0.0", Some("2.0.0")),
                            mismatch("gone", "0.5.0", None),
                        ]]
                    );
                    assert!(
                        lines.iter().any(|line| line.contains(
                            "Sunrise: mod gone was 0.5.0 when the world was saved, now missing"
                        )),
                        "{lines:?}"
                    );
                    assert!(
                        !lines.iter().any(|line| line.contains("mod mismatch_test")),
                        "{lines:?}"
                    );
                    game.save_game_with(true).unwrap();
                });
                assert_eq!(
                    Level::read(&changed).unwrap().saved_mods,
                    Some(vec![record("mismatch_test", "2.0.0")])
                );
                // An unchanged list still fires.
                with_game(|game| {
                    game.load_world(request(&changed)).unwrap();
                    assert_eq!(heard.lock().unwrap().len(), 2);
                });
                // A world without the list fires nothing and gets the list on its save.
                with_game(|game| {
                    game.load_world(request(&plain)).unwrap();
                    assert_eq!(heard.lock().unwrap().len(), 2);
                    game.save_game_with(true).unwrap();
                });
                assert_eq!(
                    Level::read(&plain).unwrap().saved_mods,
                    Some(vec![record("mismatch_test", "2.0.0")])
                );
            },
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mods_level_opens_with_no_data_packs() {
        use sunrise_client::chunk::{LevelShape, LevelShapes};
        use sunrise_client::datapack::worldgen_overrides_over;
        let (mut none, _) = worldgen_overrides_over(
            &[],
            &Default::default(),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        none.include_implicit_level_stems();
        assert_eq!(shapes_of(&none, none.level_stems()).0, LevelShapes::VANILLA);

        let pocket = sunrise_world::worldgen::overrides::ModDimension {
            id: "test:pocket".into(),
            stem: r#"{"type":"test:pocket","generator":{"type":"minecraft:noise"}}"#.into(),
            dimension_type:
                r#"{"min_y":-128,"height":448,"logical_height":320,"has_skylight":false,"has_ceiling":true}"#
                    .into(),
            sea_level: None,
        };
        let (mut overrides, _) = worldgen_overrides_over(
            &[pocket],
            &Default::default(),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        overrides.include_implicit_level_stems();
        let registry: std::collections::HashMap<_, _> = overrides.level_stems().collect();
        let traversal = [
            "minecraft:the_nether",
            "test:pocket",
            "minecraft:overworld",
            "minecraft:the_end",
        ];
        assert_eq!(registry.len(), traversal.len());
        let (shapes, configured) = shapes_of(
            &overrides,
            traversal.into_iter().map(|id| (id, registry[id])),
        );
        assert_eq!(configured, traversal);
        assert!(shapes.has("test:pocket"));
        assert_eq!(
            shapes.get("test:pocket"),
            LevelShape::new(-128, 448, 320, false, true).unwrap()
        );
        assert_eq!(shapes.dimension_type("test:pocket"), "test:pocket");
        assert_eq!(shapes.overworld, LevelShape::OVERWORLD);
    }

    /// A data-pack stem can pair a registered generator with a biome holder set without any
    /// NoiseGeneratorSettings. The generator table is scoped to this fixture.
    #[test]
    #[ignore = "daily"]
    fn a_stem_naming_a_mods_generator_opens_on_mod_terrain() {
        use std::fs;
        use sunrise_client::streaming::ChunkGenerator as _;
        use sunrise_world::{
            native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
            worldgen::mod_generators::ChunkAccess,
        };
        struct Floor;
        impl ModChunkGenerator for Floor {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let stone = chunk.state("stone");
                chunk.set_block([0, chunk.min_y(), 0], stone);
            }
        }
        let generators = ModChunkGenerators::from([(
            Identifier::parse("test:floor").unwrap(),
            Arc::new(Floor) as Arc<dyn ModChunkGenerator>,
        )]);
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let biomes = sunrise_client::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        )
        .unwrap();
        let registry = sunrise_client::assets::Assets::load(&root)
            .unwrap()
            .registry;
        let scratch = tempfile::tempdir().unwrap();
        let world = scratch.path();
        let pack = world.join("datapacks/checkerboard");
        let dimensions = pack.join("data/test/dimension");
        fs::create_dir_all(&dimensions).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Checkerboard test","min_format":[121,0],"max_format":[121,0]}}"#,
        ).unwrap();
        fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/checkerboard"],"disabled":[]}"#,
        )
        .unwrap();
        fs::write(
            dimensions.join("pocket.json"),
            r##"{
            "type":"minecraft:overworld", "generator":{"type":"test:floor", "biome_source":{
                "type":"minecraft:checkerboard", "biomes":"#minecraft:is_forest", "scale":0
            }}
        }"##,
        )
        .unwrap();
        let inputs = LoadInputs::new(Default::default(), root.join("settings"));
        let item_data = Arc::new(
            sunrise_world::item_defaults::ItemData::load(
                &root,
                LoadContext::new(DataLoadOrigin::ServerDataLoad, Arc::new(inputs), []),
            )
            .unwrap(),
        );
        let worldgen = root.join("worldgen");
        let resources = TerrainResources {
            data: None,
            dir: &worldgen,
            registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
        let (overrides, _) =
            worldgen_overrides_in_world(world, &root, &[], &[], &base_conditions).unwrap();
        let open = |dim, overrides: &_, saved: Option<&_>| {
            mod_terrain(
                overrides,
                resources,
                &generators,
                dim,
                sunrise_client::chunk::LevelShape::OVERWORLD,
                0,
                true,
                None,
                saved,
            )
        };
        let pocket = Dim::Extra(mod_api::__private::dimension_key("test:pocket"));
        let Some(Ok((ModLevel::Terrain(terrain), generator))) = open(pocket, &overrides, None)
        else {
            panic!("the pack's checkerboard needs no noise settings");
        };
        assert_eq!(
            generator.as_str(),
            "test:floor",
            "the read names the generator that built it"
        );
        let chunk = terrain.finish(terrain.proto(&mut (), 0, 0).unwrap());
        assert_eq!(
            chunk.block(0, -64, 0),
            registry.default_state("minecraft:stone").unwrap()
        );
        assert_eq!(biomes.name(chunk.biome(0, 0, 0)), "minecraft:forest");
        assert!(open(Dim::Overworld, &overrides, None).is_none());

        let empty = Default::default();
        let mut stem = serde_json::json!({
            "type": "minecraft:overworld",
            "generator": {
                "type": "test:floor",
                "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:plains"},
            },
        });
        let saved = sunrise_client::scoreboard::json_to_tag(&stem);
        let Some(Ok((_, generator))) = open(pocket, &empty, Some(&saved)) else {
            panic!("the registered generator should also bind from a saved stem");
        };
        assert_eq!(generator.as_str(), "test:floor");

        for name in ["test:BAD", "test:unregistered"] {
            stem["generator"]["type"] = name.into();
            let invalid = sunrise_client::scoreboard::json_to_tag(&stem);
            let mut packed = empty.clone();
            packed.set(
                sunrise_world::worldgen::overrides::DIMENSIONS,
                pocket.id(),
                stem.to_string(),
            );
            assert!(
                open(pocket, &empty, Some(&invalid)).is_none(),
                "saved {name}"
            );
            // A valid pack stem wins before lookup even when its name does not bind.
            assert!(open(pocket, &packed, Some(&saved)).is_none(), "pack {name}");
        }
    }

    /// A no-stage checkerboard stem opens when only biome and biome-tag data exist, without
    /// loading terrain-stage bundles (`settings.json`, `noises.json`, `block_flags.json`).
    #[test]
    fn no_stage_checkerboard_opens_without_noise_settings_bundle() {
        use std::fs;
        use sunrise_client::streaming::ChunkGenerator as _;
        use sunrise_world::{
            native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
            registry::BlockRegistry,
            worldgen::{
                generator::GeneratorData, mod_generators::ChunkAccess, overrides::WorldgenOverrides,
            },
        };
        struct Floor;
        impl ModChunkGenerator for Floor {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let stone = chunk.state("stone");
                chunk.set_block([0, chunk.min_y(), 0], stone);
            }
        }
        let generators = ModChunkGenerators::from([(
            Identifier::parse("test:floor").unwrap(),
            Arc::new(Floor) as Arc<dyn ModChunkGenerator>,
        )]);
        let scratch = tempfile::tempdir().unwrap();
        let resources_root = scratch.path().join("resources");
        let biome_dir = resources_root.join("data/minecraft/worldgen/biome");
        let worldgen_dir = resources_root.join("worldgen");
        fs::create_dir_all(&biome_dir).unwrap();
        fs::create_dir_all(&worldgen_dir).unwrap();
        fs::write(resources_root.join("items.json"), "{}").unwrap();
        fs::write(biome_dir.join("plains.json"), "{}").unwrap();
        fs::write(biome_dir.join("desert.json"), "{}").unwrap();
        fs::write(
            worldgen_dir.join("biomes.json"),
            r#"{"minecraft:plains":"{}","minecraft:desert":"{}"}"#,
        )
        .unwrap();
        fs::write(
            worldgen_dir.join("biome_tags.json"),
            r#"{"minecraft:is_dry":"{\"values\":[\"minecraft:desert\",\"minecraft:plains\"]}"}"#,
        )
        .unwrap();
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            BlockRegistry::from_facts_json(
                &br#"{"states":[["minecraft:air",true,false,"","minecraft:air"],["minecraft:stone",false,false,"","minecraft:stone"]],"defaults":{"minecraft:air":0,"minecraft:stone":1}}"#[..],
            )
            .unwrap(),
        ));
        assert!(
            GeneratorData::load_with(&worldgen_dir, registry, &WorldgenOverrides::default())
                .is_err(),
            "no noise-settings or terrain-stage bundles exist in this fixture"
        );
        let biomes = sunrise_client::biome::Biomes::load_for_server(
            &sunrise_assets::resource_source::ResourceRoot::directory(resources_root.clone()),
        )
        .unwrap();
        let inputs = LoadInputs::new(Default::default(), resources_root.join("settings"));
        let item_data = Arc::new(
            sunrise_world::item_defaults::ItemData::load(
                &resources_root,
                LoadContext::new(DataLoadOrigin::ServerDataLoad, Arc::new(inputs), []),
            )
            .unwrap(),
        );
        let world = scratch.path().join("world");
        let pack = world.join("datapacks/checkerboard");
        let dimensions = pack.join("data/test/dimension");
        fs::create_dir_all(&dimensions).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Checkerboard test","min_format":[121,0],"max_format":[121,0]}}"#,
        )
        .unwrap();
        fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/checkerboard"],"disabled":[]}"#,
        )
        .unwrap();
        fs::write(
            dimensions.join("pocket.json"),
            r##"{
                "type":"minecraft:overworld",
                "generator":{
                    "type":"test:floor",
                    "biome_source":{
                        "type":"minecraft:checkerboard",
                        "biomes":"#minecraft:is_dry",
                        "scale":0
                    }
                }
            }"##,
        )
        .unwrap();
        let resources = TerrainResources {
            data: None,
            dir: &worldgen_dir,
            registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let pocket = Dim::Extra(mod_api::__private::dimension_key("test:pocket"));
        let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
        let (overrides, _) =
            worldgen_overrides_in_world(&world, &resources_root, &[], &[], &base_conditions)
                .unwrap();
        let Some(Ok((ModLevel::Terrain(terrain), generator))) = mod_terrain(
            &overrides,
            resources,
            &generators,
            pocket,
            sunrise_client::chunk::LevelShape::OVERWORLD,
            0,
            true,
            None,
            None,
        ) else {
            panic!("no-stage checkerboard must open without noise-settings bundle");
        };
        assert_eq!(generator.as_str(), "test:floor");
        let chunk0 = terrain.finish(terrain.proto(&mut (), 0, 0).unwrap());
        assert_eq!(
            chunk0.block(0, -64, 0),
            registry.default_state("minecraft:stone").unwrap()
        );
        assert_eq!(biomes.name(chunk0.biome(0, 0, 0)), "minecraft:desert");
        let chunk1 = terrain.finish(terrain.proto(&mut (), 1, 0).unwrap());
        assert_eq!(biomes.name(chunk1.biome(0, 0, 0)), "minecraft:plains");
    }

    #[test]
    #[ignore = "daily"]
    fn both_server_roles_record_regions_before_world_generation() {
        mod_platform::__private::initialize().unwrap();
        use super::super::native_entity_refs::tests::with_game_role;
        use execution_role::ExecutionRole::{DedicatedServer, IntegratedServer};
        struct Regions;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Regions {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar
                    .declare_biome_region(
                        "ice",
                        mod_api::BiomeRegion::new(
                            "minecraft:overworld".into(),
                            3,
                            vec![mod_api::BiomePoint {
                                biome: "minecraft:ice_spikes".into(),
                                temperature: (-2., 2.),
                                humidity: (-2., 2.),
                                continentalness: (-2., 2.),
                                erosion: (-2., 2.),
                                depth: (-2., 2.),
                                weirdness: (-2., 2.),
                                offset: 0.,
                            }],
                        ),
                    )
                    .unwrap();
            }
        }
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        for role in [IntegratedServer, DedicatedServer] {
            let scratch = tempfile::tempdir().unwrap();
            let world = scratch.path().join("world");
            mod_api::testing::with_dispatch(
                vec![mod_api::ModEntry::new("region_fixture", Box::new(Regions))].into(),
                || {
                    with_game_role(root.clone(), role, |game| {
                        let request = WorldRequest {
                            owner: None,
                            chunk_reports: sunrise_client::chunk_io::PROCESS_REPORTS.clone(),
                            stem_order: None,
                            save_root: world.clone(),
                            folder: "world".into(),
                            name: Some("Region fixture".into()),
                            seed: Some(12345),
                            creative: true,
                            create: None,
                            rule_registry: Level::default().rule_registry,
                        };
                        game.load_world(request).unwrap();
                        let layout = sunrise_client::save::biome_regions::layout(
                            &world,
                            "minecraft:overworld",
                        )
                        .unwrap()
                        .unwrap();
                        assert_eq!(layout.regions.len(), 1, "{role:?}");
                        assert_eq!(layout.regions[0].id, "region_fixture:ice");
                        assert_eq!(layout.regions[0].weight, 3);
                        assert_eq!(layout.regions[0].points[0].biome, "minecraft:ice_spikes");
                    });
                },
            );
        }
    }

    #[test]
    fn worker_panics_reach_the_client() {
        let loader = WorldLoader {
            handle: std::thread::spawn(|| panic!("broken world initialization")),
        };
        assert_eq!(
            loader.finish().err().unwrap().to_string(),
            "world initialization worker panicked"
        );
    }

    #[test]
    fn prepared_world_can_cross_threads() {
        fn send<T: Send>() {}
        send::<PreparedWorld>();
    }
}

#[cfg(test)]
mod unused_feature_startup_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn actual_world_prepare_rejects_unused_invalid_feature() {
        let resources = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(resources.clone()),
            Vec::new(),
            None,
            &resources,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = resources;
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        for (case, height) in [("valid", 0), ("invalid", -1)] {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path().join(case);
            let pack = root.join("datapacks/h259");
            std::fs::create_dir_all(pack.join("data/h259/worldgen/feature")).unwrap();
            std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"H259 unused feature","min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
            let json = format!(
                r#"{{"type":"minecraft:fill_layer","height":{height},"state":"minecraft:stone"}}"#
            );
            std::fs::write(
                pack.join("data/h259/worldgen/feature/unused_probe.json"),
                &json,
            )
            .unwrap();
            let mut level = Level {
                seed: 259,
                data_packs: Some((vec!["vanilla".into(), "file/h259".into()], vec![])),
                ..Level::default()
            };
            level.write(&root, &Default::default()).unwrap();
            sunrise_client::save::write_world_gen_settings(
                &root,
                259,
                sunrise_client::save::WorldGen {
                    preset: sunrise_client::save::WorldPreset::Flat,
                    structures: false,
                    bonus_chest: false,
                },
            )
            .unwrap();
            let (overrides, _) = worldgen_overrides_in_world(
                &root,
                &options.resources,
                &level.enabled_features,
                &loaded.pack.additional_data,
                &sunrise_world::conditions::LoadConditions::vanilla(),
            )
            .unwrap();
            assert_eq!(
                overrides
                    .entries("features")
                    .find(|(id, _)| *id == "h259:unused_probe")
                    .unwrap()
                    .1,
                json
            );
            assert!(overrides.entries("placed_features").next().is_none());
            assert_eq!(
                sunrise_client::save::WorldGen::read(&root).unwrap().preset,
                sunrise_client::save::WorldPreset::Flat
            );
            let save_root = root.clone();
            let request = WorldRequest {
                save_root: root,
                folder: case.into(),
                name: None,
                seed: Some(259),
                creative: false,
                create: None,
                owner: None,
                chunk_reports: sunrise_client::chunk_io::PROCESS_REPORTS.clone(),
                stem_order: None,
                rule_registry: Level::default().rule_registry,
            };
            let mut prepared = request
                .prepare(
                    &loaded.resources,
                    &loaded.pack,
                    &options,
                    &sunrise_world::datafix::progress::UpgradeProgress::new(),
                )
                .unwrap();
            // The registry check runs in the terrain phase of the load, after the world's
            // registry winners are admitted, as `functions.rs` drives it.
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = crate::game::functions::pack_stack(
                &save_root,
                &options.resources,
                &empty,
                &empty,
                false,
                &prepared.level.enabled_features,
                &loaded.pack.additional_data,
            );
            let biomes = WorldBiomeRegistry::load(
                loaded.resources.biomes(),
                &save_root,
                loaded.pack.assets.registry,
                &stack,
                &stack.data_packs(Vec::new()),
                &sunrise_world::loot::PackOverlay::default(),
                &Default::default(),
            )
            .unwrap();
            let result = prepared.prepare_terrain(
                &loaded.resources,
                &loaded.pack,
                &options,
                &settings,
                &biomes,
            );
            if height == 0 {
                assert!(result.is_ok(), "valid unused registry entry must load");
                drop(result);
            } else {
                let error = match result {
                    Ok(_) => panic!(
                        "unused invalid h259:unused_probe was accepted by actual world loading"
                    ),
                    Err(error) => error.to_string(),
                };
                assert!(error.contains("h259:unused_probe"), "{error}");
            }
        }
    }
}

#[cfg(test)]
mod unused_feature_preservation_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn startup_leaf_preservation_other_id_and_unrelated_feature() {
        let resources = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(resources.clone()),
            Vec::new(),
            None,
            &resources,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = resources;
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("preservation");
        let pack = root.join("datapacks/h259");
        let entries = pack.join("data/controls/worldgen/feature");
        std::fs::create_dir_all(&entries).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"H259 preservation controls","min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
        let leaf = r#"{"type":"minecraft:fill_layer","height":1,"state":"minecraft:stone"}"#;
        let unrelated = r#"{"type":"minecraft:no_op"}"#;
        std::fs::write(entries.join("another_id.json"), leaf).unwrap();
        std::fs::write(entries.join("unrelated.json"), unrelated).unwrap();
        let mut level = Level {
            seed: 259,
            data_packs: Some((vec!["vanilla".into(), "file/h259".into()], vec![])),
            ..Level::default()
        };
        level.write(&root, &Default::default()).unwrap();
        sunrise_client::save::write_world_gen_settings(
            &root,
            259,
            sunrise_client::save::WorldGen {
                preset: sunrise_client::save::WorldPreset::Flat,
                structures: false,
                bonus_chest: false,
            },
        )
        .unwrap();
        let (mut overrides, _) = worldgen_overrides_in_world(
            &root,
            &options.resources,
            &level.enabled_features,
            &loaded.pack.additional_data,
            &sunrise_world::conditions::LoadConditions::vanilla(),
        )
        .unwrap();
        assert_eq!(
            overrides.entries("features").collect::<Vec<_>>(),
            vec![
                ("controls:another_id", leaf),
                ("controls:unrelated", unrelated),
            ]
        );
        // Labeled native preservation controls, not additional Java runtime coverage.
        overrides
            .validate_fill_layers(loaded.pack.assets.registry)
            .unwrap();
        // The existing winning-entry map still owns precedence, not the validation phase.
        overrides.set(
            "features",
            "controls:another_id",
            r#"{"type":"minecraft:fill_layer","height":-1,"state":"minecraft:stone"}"#.into(),
        );
        let error = overrides
            .validate_fill_layers(loaded.pack.assets.registry)
            .unwrap_err();
        assert!(error.to_string().contains("controls:another_id"));
        overrides.set("features", "controls:another_id", leaf.into());
        overrides
            .validate_fill_layers(loaded.pack.assets.registry)
            .unwrap();
        let request = WorldRequest {
            save_root: root,
            folder: "preservation".into(),
            name: None,
            seed: Some(259),
            creative: false,
            create: None,
            owner: None,
            chunk_reports: sunrise_client::chunk_io::PROCESS_REPORTS.clone(),
            stem_order: None,
            rule_registry: Level::default().rule_registry,
        };
        assert!(
            request
                .prepare(
                    &loaded.resources,
                    &loaded.pack,
                    &options,
                    &sunrise_world::datafix::progress::UpgradeProgress::new(),
                )
                .is_ok(),
            "preservation: other-id leaf and unrelated no_op must still prepare"
        );
    }
}
