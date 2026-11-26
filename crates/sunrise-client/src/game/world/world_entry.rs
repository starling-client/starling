//! Entering a world without blocking the window: the deferred entry, world creation, the world
//! resources' background load, and the world's own content tables.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
<<<<<<< HEAD
#[path = "world_entry/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;
||||||| 46c00106def
=======
use sunrise_client::items::{EntityTypeTags, LoadEntityTags};
>>>>>>> ed6772e2118

static BIOME_KEYS_SESSION: AtomicU64 = AtomicU64::new(1);

fn next_biome_keys_session() -> u64 {
    BIOME_KEYS_SESSION.fetch_add(1, Ordering::Relaxed)
}

impl Game<'_> {
    /// The server's data-pack enchantments, or the local world's registry.
    pub(super) fn item_enchantments(&self) -> &Arc<sunrise_client::enchant::Enchantments> {
        match self.remote.as_ref() {
            Some(remote) => remote
                .enchantments
                .as_ref()
                .unwrap_or(&self.items().enchantments),
            None => &self.stations().enchantments,
        }
    }

    pub(super) fn defer_world_entry(&mut self, entry: PendingWorldEntry) -> Result<bool> {
        self.poll_world_rest()?;
        if self.rest_loader.is_none() {
            return Ok(false);
        }
        self.perf.world_opening();
        if self.title.is_none() {
            let title = self
                .new_title(std::env::current_dir()?.join("saves"))
                .with_user(|| self.options.player_name());
            self.show_title(title);
        }
        self.pending_entry = Some(entry);
        log::info!("Sunrise: world entry waiting for world resources");
        Ok(true)
    }

    pub(super) fn resume_world_entry(&mut self) -> Result<()> {
        self.poll_integrated_start()?;
        self.poll_world_rest()?;
        self.poll_world_creation()?;
        if self.rest_loader.is_some() {
            return Ok(());
        }
        let on_world_screens = self.title.as_ref().is_some_and(|title| {
            !matches!(
                title.mode,
                sunrise_client::title::Mode::Main | sunrise_client::title::Mode::Multiplayer(_)
            )
        });
        world_loading::preload_world_data(
            on_world_screens,
            &self.options.resources,
            &self.pack.data_changes,
            self.pack.assets.registry,
        );
        if std::mem::take(&mut self.pending_pack_reload) {
            self.reload_packs();
        }
        match self.pending_entry.take() {
            Some(PendingWorldEntry::Local {
                folder,
                name,
                seed,
                creative,
                create,
            }) => {
                self.open_world(&folder, name.as_deref(), seed, creative, create.as_deref())?;
            }
            Some(PendingWorldEntry::Server {
                address,
                name,
                list_name,
                cookies,
                lan,
                realm,
            }) => {
                self.connect_server_with_options(
                    &address,
                    &name,
                    cookies,
                    lan,
                    realm,
                    list_name.as_deref(),
                )?;
            }
            Some(PendingWorldEntry::StudioReplay(path)) => {
                let path_str = path.to_string_lossy().to_string();
                if let Err(e) = self.open_studio_replay(&path_str) {
                    log::warn!("Sunrise Studio: could not open {path_str}: {e}");
                }
            }
            None => {}
        }
        Ok(())
    }

    /// `CreateWorldScreen.openCreateWorldScreen`: read the world data the create
    /// screen describes (`title.preparing`). The first call leaves the preparing
    /// screen up for a frame; the next reads.
    pub(super) fn poll_world_creation(&mut self) -> Result<()> {
        let Some(title) = self.title.as_ref() else {
            return Ok(());
        };
        if !title.preparing() {
            return Ok(());
        }
        if !std::mem::replace(&mut self.creation_read_started, true) {
            return Ok(());
        }
        self.creation_read_started = false;
        let (dir, packs) = title.creation_packs().unwrap_or_default();
        let outcome = self
            .read_creation_packs(dir.as_deref(), &packs)
            .map_err(|e| e.to_string());
        if let Some(title) = self.title.as_mut() {
            match outcome {
                Ok(()) => title.finish_preparing(),
                // `applyNewPackConfig`'s failure: back out of the read with
                // the message, the Create screen's options intact.
                Err(e) => {
                    log::warn!("Sunrise: preparing world creation failed: {e}");
                    // A new selection's failed read asks where to go
                    // (`dataPack.validation.failed`). The first read clears the
                    // flag before `failed` sets the error screen with Create as
                    // its back target, which `finish_preparing` would replace
                    // with a fresh world.
                    if !title.pack_validation_failed() {
                        title.finish_preparing();
                        title.failed(format!(
                            "{}: {e}",
                            sunrise_client::lang::get_or(
                                "createWorld.preparing",
                                "Preparing for world creation..."
                            )
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// `applyNewPackConfig` reloads with the new pack config through `WorldLoader.load`: the
    /// chosen packs' registries and worldgen are staged as a world open stages them (`open_world_datapacks`),
    /// and refused as it refuses them.
    fn read_creation_packs(
        &mut self,
        pack_dir: Option<&std::path::Path>,
        selected: &[String],
    ) -> Result<()> {
        let resources = self.options.resources.clone();
        let stack = super::functions::creation_stack(
            pack_dir,
            &resources,
            &self.pack.data_source,
            &self.pack.data_changes,
            selected,
            &self.pack.additional_data,
        );
        let staged = self.stage_world_tables(&stack, false, &resources)?;
        if let Some(tables) = staged.tables {
            tables
                .biomes
                .overrides
                .validate_fill_layers(self.pack.assets.registry)?;
        }
        Ok(())
    }

    pub(super) fn poll_world_rest(&mut self) -> Result<()> {
        if let Some(loader) = &self.rest_loader
            && loader.ready()?
        {
            self.rest_loader
                .take()
                .ok_or("missing world loader")?
                .finish()?;
        }
        Ok(())
    }

    // ---- the world's data packs' content tables ---------------------------

    /// The content tables the world plays with: its data packs' over the
    /// game's own when it has packs, `Resources.rest`'s own otherwise.
    pub(super) fn items(&self) -> &Items {
        world_items!(self)
    }
    /// The level's suffocation rules, for its mobs and its players.
    pub(super) fn level_suffocation(
        &self,
    ) -> &sunrise_world::suffocation::SuffocationRegistry<'static> {
        level_suffocation_of(&self.world_tables, &self.res)
    }
    pub(super) fn recipes(&self) -> &Recipes {
        match &self.world_tables {
            Some(t) => &t.recipes,
            None => self.synced_recipes().unwrap_or_else(|| self.res.recipes()),
        }
    }
    pub(in crate::game) fn active_providers(&self) -> &Providers {
        self.world_tables
            .as_ref()
            .map_or_else(|| self.res.providers(), |t| &t.providers)
    }
    /// The station recipes menus and screens use: a client of a server numbers and draws its
    /// stonecutter buttons by the list the server sent, else the world's own.
    pub(super) fn station_recipes(&self) -> &sunrise_client::stations::StationRecipes {
        self.recipe_ui
            .server_stations
            .as_ref()
            .unwrap_or(&self.stations().recipes)
    }
    pub(super) fn stations(&self) -> &sunrise_client::stations::StationData {
        match &self.world_tables {
            Some(t) => &t.stations,
            None => self.res.stations(),
        }
    }

    /// The biome colours used by the mesher and client tint queries. On a
    /// world without its own table, a colour refresh updates the pack's table.
    pub(super) fn mesh_biomes(&self) -> &Arc<Biomes> {
        self.world_biomes
            .as_ref()
            .unwrap_or(&self.pack.context.biomes)
    }

    /// What the chunk and far-terrain workers mesh with: the pack's context,
    /// over the world's biomes when it has its own.
    pub(super) fn worker_context(&self) -> Arc<WorkerContext> {
        self.pack.context_with(self.mesh_biomes().clone())
    }
    pub(super) fn pack_files(&self) -> sunrise_world::loot::PackOverlay<'_> {
        pack_files(&self.world_tables)
    }
    /// Load the world's data packs' content tables (`None`: the world has no
    /// packs, and `Resources.rest` stays in use). This re-reads the tables,
    /// so it runs once per world open and on `/reload`.
    pub(super) fn stage_world_tables(
        &mut self,
        stack: &functions::PackStack,
        reload: bool,
        root: &std::path::Path,
    ) -> Result<super::condition_reload_state::StagedWorldTables> {
        // The ask of this load, both here and on `/reload`: a native mod
        // recomputes the pack files it contributes every time.
        let mut contributed = Vec::new();
        self.native_data_pack_files(&mut contributed);
        let modifier_files = global_loot_modifiers::contributed(&contributed);
        let data = Arc::new(
            stack.data_packs(
                contributed
                    .into_iter()
                    .filter(|(key, _)| !key.starts_with("loot_modifiers/"))
                    .collect(),
            ),
        );
        let functions::OverlayFiles {
            files,
            read_errors,
            priority_layers,
        } = functions::overlay_files(&data);
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(root)?;
        let base_conditions = load_conditions::conditions_for_mods(&self.mods, &inventory)?;
        // Tag files are not conditional. Collect the candidate's static holders
        // and pending tags before registry codecs can inspect them.
        let tag_files = sunrise_world::loot::PackOverlay::new(&files);
        let mut items = Items::load_registry(
            root,
            self.pack.assets.registry,
            self.res.facts(),
            &tag_files,
            self.res.item_defaults(),
            if reload {
                LoadEntityTags::Reload(&self.items().entity_type_tags)
            } else {
                LoadEntityTags::Frozen(self.res.items().entity_type_tags.ids())
            },
        )?;
        if reload {
            items.retain_unbound_tags(self.items());
        }
        class_tags::install(&mut items, self.res.facts());
        let mut view = load_conditions::RegistryView {
            ids: Default::default(),
            tags: items
                .bound_tags()
                .map(|(registry, tags)| (registry.to_owned(), tags.map(str::to_owned).collect()))
                .collect(),
        };
        use sunrise_client::lan::data::{BIOME_REGISTRY, REGISTRY_KINDS, SyncData};
        let builtin = sunrise_protocol::registry::Builtin::find(root)
            .map(|path| sunrise_protocol::registry::Builtin::load(&path))
            .transpose()?
            .map(sunrise_client::lan::data::with_custom_stats)
            .unwrap_or_default();
        for (key, ids) in &builtin.registries {
            view.ids.insert(
                key.clone(),
                ids.names
                    .iter()
                    .filter(|id| !id.is_empty())
                    .cloned()
                    .collect(),
            );
        }
        view.ids.insert(
            "minecraft:item".into(),
            items.items.keys().map(ToString::to_string).collect(),
        );
        view.ids.insert(
            "minecraft:block".into(),
            self.res
                .facts()
                .states
                .iter()
                .map(|state| state.name.to_string())
                .collect(),
        );
        view.ids.insert(
            EntityTypeTags::REGISTRY.into(),
            items.entity_type_tags.ids().iter().cloned().collect(),
        );
        let mut pack_tags = HashMap::new();
        let mut banner_tags = std::collections::BTreeMap::new();
        let gameplay_tags = WorldTables::PACK_TAG_REGISTRIES
            .into_iter()
            .chain(sunrise_client::neoforge_mob_maps::MAPS.map(|definition| definition.directory))
            .filter(|directory| !matches!(*directory, "item" | "entity_type" | "damage_type"))
            .collect::<std::collections::BTreeSet<_>>();
        let mut timeline_tags = std::collections::BTreeMap::new();
        let mut bind_tags = |registry: &str,
                             view: &mut load_conditions::RegistryView|
         -> Result<()> {
            let ids = view.ids.get(registry);
            let tags = sunrise_client::lan::data::registry_tag_bindings(
                root,
                &tag_files,
                registry,
                |id| ids.is_some_and(|ids| ids.contains(id)),
            )?;
            if registry == "minecraft:timeline" {
                timeline_tags = tags.clone();
            }
            view.tags.insert(
                registry.to_owned(),
                tags.iter()
                    .filter(|(_, members)| !members.is_empty())
                    .map(|(tag, _)| tag.clone())
                    .collect(),
            );
            if registry == "minecraft:banner_pattern" {
                banner_tags.clone_from(&tags);
            }
            if let Some(directory) = registry
                .strip_prefix("minecraft:")
                .and_then(|name| gameplay_tags.get(name))
            {
                pack_tags.insert(
                    *directory,
                    tags.into_iter()
                        .map(|(tag, members)| (tag, members.into_iter().collect::<HashSet<_>>()))
                        .collect(),
                );
            }
            Ok(())
        };
        // Item, block, fluid and entity type tags already came from Items' loader.
        let static_registries = builtin
            .registries
            .keys()
            .map(String::as_str)
            .chain(gameplay_tags.iter().filter_map(|directory| {
                view.ids
                    .get_key_value(&format!("minecraft:{directory}"))
                    .map(|(key, _)| key.as_str())
            }))
            .filter(|&key| {
                !matches!(key, BIOME_REGISTRY | "minecraft:damage_type")
                    && !REGISTRY_KINDS.iter().any(|table| table.key == key)
                    && !view.tags.contains_key(key)
            })
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        for registry in &static_registries {
            bind_tags(registry, &mut view)?;
        }
        // WORLD_REGISTRIES codecs see static holders (not their own pending entries)
        // and NeoForge's ResourceManagerRegistryLoadTask uses VANILLA_SET.
        let frozen = self.world_tables.as_ref().filter(|_| reload);
        let fixed = REGISTRY_KINDS.iter().map(|table| table.key);
        for registry in fixed
            .clone()
            .chain(["minecraft:damage_type", BIOME_REGISTRY])
        {
            view.ids.remove(registry);
            view.tags.remove(registry);
        }
        let registry_base = base_conditions.with_features(["minecraft:vanilla"]);
        let registry_conditions = view.conditions(&registry_base);
        let registry_overlay = sunrise_world::loot::PackOverlay {
            conditions: &registry_conditions,
            files: &files,
            priority_layers: &priority_layers,
        };
        let base = if reload {
            &self.health_resources
        } else {
            &self.res.motion().health
        };
        let health = base.with_packs(root, &registry_overlay, reload)?;
        let (packed_registry_definitions, packed_admission) = if let Some(tables) = frozen {
            (tables.packed_registries.clone(), Ok(()))
        } else {
            let (packed, admission) = SyncData::stage_registry_definitions(
                root,
                self.res.item_defaults(),
                &registry_overlay,
                &read_errors,
                &items.component_registry,
                |id| {
                    items
                        .component_registry
                        .ids("minecraft:sound_event")
                        .binary_search_by(|sound| sound.as_str().cmp(id))
                        .is_ok()
                },
            );
            (Arc::new(packed), admission)
        };
        let variants = frozen
            .map(|tables| tables.variants.clone())
            .unwrap_or_else(|| Arc::new(packed_registry_definitions.variant_tables()));
        let biomes = if let Some(tables) = frozen {
            tables.biomes.clone()
        } else {
            Arc::new(world_loading::WorldBiomeRegistry::load(
                self.res.biomes(),
                root,
                self.pack.assets.registry,
                stack,
                &data,
                &registry_overlay,
                &read_errors,
            )?)
        };
        let frozen_tests = frozen.map(|tables| {
            (
                tables.test_environments.clone(),
                tables.test_instances.clone(),
            )
        });
        view.ids.insert(
            BIOME_REGISTRY.into(),
            biomes
                .table
                .registered_keys()
                .iter()
                .map(ToString::to_string)
                .collect(),
        );
        for (registry, id, admitted) in packed_registry_definitions.entries() {
            let ids = view.ids.entry(registry.to_owned()).or_default();
            if admitted {
                ids.insert(id);
            } else {
                ids.remove(&id);
            }
        }
        view.ids.insert(
            "minecraft:damage_type".into(),
            health
                .damage
                .entries()
                .map(|(_, definition)| definition.name.clone())
                .collect(),
        );
        for registry in fixed.chain([BIOME_REGISTRY]) {
            bind_tags(registry, &mut view)?;
        }
        // Painting.create's candidates: the world's painting_variant registry (the
        // root's variants under the packs' winners) and `#minecraft:placeable` over it.
        let painting_variants = {
            let all = packed_registry_definitions
                .painting_variants(paintings::PaintingVariants::load(root).all);
            let mut tags = sunrise_client::lan::data::registry_tag_bindings(
                root,
                &tag_files,
                "minecraft:painting_variant",
                |id| all.iter().any(|variant| &*variant.id == id),
            )?;
            let placeable = tags
                .remove("minecraft:placeable")
                .unwrap_or_default()
                .into_iter()
                .map(Arc::from)
                .collect();
            paintings::PaintingVariants { all, placeable }
        };
        // WORLD_REGISTRIES owns the structures; its raw definitions are not holders.
        // First open uses the same cached data as live features. Reload binds new tags
        // to the frozen owner, just as TagLoader does before server resource reload.
        let generator = if reload {
            self.registry_generator_data()
        } else {
            // This bundle can be unavailable independently of the resource tables.
            // Terrain preparation owns the existing diagnostics for it.
            sunrise_client::vanillagen::shared_data_from(
                &root.join("worldgen"),
                &biomes.overrides,
                self.pack.assets.registry,
                stack.content.source(),
            )
            .ok()
        };
        if let Some(generator) = generator {
            const STRUCTURES: &str = "minecraft:worldgen/structure";
            let mut holders = std::collections::BTreeSet::new();
            if generator
                .visit_registry_ids(STRUCTURES, &mut |id| {
                    holders.insert(id.to_owned());
                    std::ops::ControlFlow::Continue(())
                })
                .is_some()
            {
                view.ids.insert(STRUCTURES.to_owned(), holders);
                bind_tags(STRUCTURES, &mut view)?;
            }
            // Placed features are holders too: the decoded ones (the codecs live generation
            // places with, not the source files) and their tags, for a downstream
            // `neoforge:registered` or `neoforge:tag_empty`. Reload keeps the frozen owner's.
            const PLACED: &str = "minecraft:worldgen/placed_feature";
            let live = if reload {
                self.res.behaviours().live_features()
            } else {
                sunrise_world::worldgen::live::LiveFeatures::new(&generator)
                    .ok()
                    .flatten()
                    .map(Arc::new)
            };
            if let Some(ids) = live
                .as_ref()
                .and_then(|features| features.registry_ids(PLACED))
            {
                view.ids.insert(PLACED.to_owned(), ids.clone());
                bind_tags(PLACED, &mut view)?;
            }
        }
        let damage_tags: HashMap<_, HashSet<_>> = health
            .damage
            .tags()
            .map(|tag| {
                (
                    tag.to_owned(),
                    health.damage.tag_members(tag).map(str::to_owned).collect(),
                )
            })
            .collect();
        view.tags.insert(
            "minecraft:damage_type".into(),
            damage_tags
                .iter()
                .filter(|(_, members)| !members.is_empty())
                .map(|(tag, _)| tag.clone())
                .collect(),
        );
        pack_tags.insert("damage_type", damage_tags);
        if !view.tags.contains_key("minecraft:villager_trade")
            && let Ok(tags) = sunrise_client::items::load_tags(root, "villager_trade", &tag_files)
        {
            view.tags.insert(
                "minecraft:villager_trade".to_owned(),
                tags.into_iter()
                    .filter(|(_, members)| !members.is_empty())
                    .map(|(tag, _)| tag)
                    .collect(),
            );
        }

        let datapack_candidate = self.stage_datapack_registries(
            stack,
            root,
            &registry_overlay,
            &mut view,
            &load_conditions::VanillaValueInputs {
                items: &items,
                tag_files: &tag_files,
                biomes: &biomes.table,
            },
        );
        // The downstream reload listeners see every completed producer and the
        // world's flags, not the codec's vanilla flags or a previous generation.
        let conditions = view
            .conditions(&registry_base.with_features(stack.features.iter().map(String::as_str)));
        let overlay = sunrise_world::loot::PackOverlay {
            conditions: &conditions,
            files: &files,
            priority_layers: &priority_layers,
        };
        // RegistryDataLoader.RELOADABLE_REGISTRIES: providers are staged and
        // published with the other reloadable data, including on /reload.
        let providers = Providers::load_with(root, &overlay);
        let mut data_map_types = Vec::new();
        self.native_data_map_types(&mut data_map_types);
        // Visit independently readable registries before consuming any result.
        // One rejected winner must not hide the other entries' diagnostics.
        let loot = sunrise_world::loot::LootData::load_candidate_with_damage(
            root,
            &overlay,
            self.res.core.item_data.defaults(),
            &health.damage,
            &items.ordered_tags(),
        );
        let advancements = Arc::new(match &loot {
            Ok(loot) => sunrise_client::advancements::Tree::load_with_predicates(
                &root.join("data"),
                &overlay,
                loot.predicates(),
            ),
            Err(_) => sunrise_client::advancements::Tree::load_with(&root.join("data"), &overlay),
        });
        let install_items_result = packed_registry_definitions.install_items(
            &mut items,
            root,
            &overlay,
            self.res.item_defaults(),
        );
        // The block registry binds each transform's blocks and states, so a name it does not
        // know refuses the load with the other registry errors.
        let transformers = sunrise_client::block_transform::Transformers::build(
            packed_registry_definitions
                .validated("minecraft:block_transformer")
                .iter()
                .map(|(id, value)| (id.as_str(), *value)),
            self.pack.assets.registry,
        );
        // Item damage predicates use the same frozen registry as player hurt.
        Arc::make_mut(&mut items.enchantments).bind_damage_catalog(&health.damage);
        items.bind_component_damage_catalog(&health.damage);
        if let Ok(loot) = &loot {
            items.component_defaults = loot.component_defaults().clone();
        }
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let _recipe_files = sunrise_client::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = if reload {
            lan::sync::prepare_server_sources(root, &self.res.core.item_data, &items, &overlay)
        } else {
            lan::sync::load_sources(
                root,
                &self.res.core.item_data,
                &items,
                self.options.execution_role.data_load_origin(),
                &overlay,
            )
        };
        // Loot preparation resolves references after the native edits. Include
        // failures here even when a condition rejected the referenced entry.
        let loot = loot.and_then(|loot| {
            let context = lan::sync::server_data_load(root, &self.res.core.item_data)?;
            let data =
                loot.prepare((!mod_api::__private::mods().is_empty()).then_some(&context))?;
            Ok((data, context))
        });
        // A rejected early registry must not hide independent late load errors.
        // No table is installed, even if another owning loader succeeded.
        let errors = [
            packed_admission.err(),
            providers.as_ref().err().cloned(),
            install_items_result.err().map(|error| error.to_string()),
            transformers.as_ref().err().cloned(),
            conditions.check_errors().err(),
            (!advancements.refused.is_empty()).then(|| advancements.refused.join("\n")),
            loot.as_ref().err().map(ToString::to_string),
            datapack_candidate.as_ref().err().cloned(),
            sources.as_ref().err().map(ToString::to_string),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !errors.is_empty() {
            return Err(errors.join("\n").into());
        }
        let providers = providers?;
        // Bind the world-owned registries' ordered tags against the admitted entries
        // (including datapack and mod biomes, enchantments, structures, and world clocks).
        bind_world_registry_tags(&mut items, root, &tag_files, &overlay, &biomes.table)?;
        if reload {
            items.retain_unbound_tags(self.items());
        }
        let transformers = Arc::new(transformers?);
        let sources = sources?;
        let (mut loot, context) = loot?;
        let recipes = Recipes::from_resolved(root, &items, &overlay, &sources)?;
        let mut stations = sunrise_client::stations::StationData::from_sources(
            root,
            &items,
            &overlay,
            sunrise_client::banner::Banners::stage(
                root,
                self.res.item_defaults(),
                &packed_registry_definitions,
                banner_tags,
            ),
            (&sources, &recipes),
        )?;
        self.prepare_professions(root, &items, &overlay, &mut stations)?;
        // Use the same candidate's IDs and bound tags for data map targets.
        let (datapack_registries, datapack_vanilla_values) = match datapack_candidate? {
            Some((candidate, values)) => (
                Some(mod_api::DatapackRegistrySet::from_candidate(candidate)),
                values,
            ),
            None => (None, None),
        };
        let datapack_set = if self.is_logical_server() {
            datapack_registries.as_ref()
        } else {
            self.remote
                .as_ref()
                .and_then(|remote| remote.datapacks.received())
                .map(Arc::as_ref)
        };
        let registries =
            native_data_maps::WorldRegistries::new([], datapack_set, &items, root, &overlay)
                .with_candidate(&view, &pack_tags)
                .with_biome_keys(biomes.table.registered_keys());
        let causes_suffocation = items
            .block_tags
            .get("minecraft:causes_suffocation")
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let suffocation = sunrise_client::mobs::suffocation::bind(
            self.res.core.collision.clone(),
            &causes_suffocation,
        )?;
        // Items already decoded the root's command export. Block-state-only
        // command metadata is not the registry universe, and professions and
        // biomes belong to the live world candidate rather than that export.
        for registry in ["minecraft:sound_event", "minecraft:game_event"] {
            if items.component_registry.ids(registry).is_empty() {
                return Err(format!(
                    "builtin data maps require {registry} in command-registries.json"
                )
                .into());
            }
        }
        let builtin = neoforge_mob_maps::load(
            &overlay,
            |directory| match directory {
                "item" => Some(&items.item_tags),
                "entity_type" => Some(items.entity_type_tags.members()),
                _ => pack_tags.get(directory),
            },
            |registry, id| match registry {
                "minecraft:item" => items.items.contains_key(id),
                "minecraft:villager_profession" => stations.professions.resolve(id).is_some(),
                "minecraft:worldgen/biome" => biomes.table.contains(id),
                EntityTypeTags::REGISTRY => items
                    .entity_type_tags
                    .ids()
                    .binary_search_by(|known| known.as_str().cmp(id))
                    .is_ok(),
                _ => items
                    .component_registry
                    .ids(registry)
                    .binary_search_by(|known| known.as_str().cmp(id))
                    .is_ok(),
            },
        )?;
        // The timeline tags the raid code reads bind from this world's packs too.
        let gameplay = Arc::new(sunrise_client::patrol::GameplayAttributes::load(
            root, &overlay,
        )?);
        let data_maps = native_data_maps::DataMaps::load(&data_map_types, &overlay, &registries)
            .with_builtins(builtin)
            .with_biome_keys(mod_api::__private::BiomeKeys::new(
                next_biome_keys_session(),
                biomes.table.registered_keys().iter().cloned(),
            ));
        drop(registries);
        // Vanilla reads tags of these registries too: `LootTable.LIST_CODEC` (advancement
        // rewards, the loot table trigger) and `Recipe.LIST_CODEC` (the recipe triggers).
        pack_tags.insert(
            "loot_table",
            loot.registry_tags("loot_table")
                .into_iter()
                .flatten()
                .map(|(tag, members)| (tag.clone(), members.iter().cloned().collect()))
                .collect(),
        );
        pack_tags.insert(
            "recipe",
            sunrise_world::tag_layers::load(root, "recipe", &overlay, |id| {
                stations.recipe_index.contains(id)
            })?
            .into_iter()
            .map(|(tag, members)| (tag, members.into_iter().collect()))
            .collect(),
        );
        // `HolderSetCodec.lookupTag`: an advancement naming a tag that is not bound is refused.
        let unbound = advancements.unbound_tags(|registry, tag| {
            pack_tags
                .get(registry)
                .is_some_and(|tags| tags.contains_key(tag))
        });
        if !unbound.is_empty() {
            return Err(unbound.join("\n").into());
        }
        // Check again after `StationData::from_sources` (`Trades::load`) and other late
        // condition readers, before live state (`resolve_enchantment_registry`) changes.
        conditions.check_errors()?;
        // The API's key order is live state as well: do not change it for a
        // candidate refused by the condition readers.
        api_values::resolve_enchantment_registry(&items)?;
        let serializers = self
            .mods
            .loot_modifier_serializers
            .get_or_init(global_loot_modifiers::Serializers::capture);
        let (modifiers, loot_modifier_manager) = serializers.prepare(
            global_loot_modifiers::files(root, stack, modifier_files),
            &conditions,
            &context,
            &loot,
        )?;
        loot.set_global_modifiers(modifiers);
        items.set_loot_data(&loot);
        let loot = Arc::new(loot);
        let (test_environments, test_instances) = if let Some(tables) = frozen_tests {
            tables
        } else {
            let mut functions = mod_api::GameTestFunctions::default();
            self.native_register_game_tests(&mut functions);
            test_instances::build_game_test_tables(&packed_registry_definitions, &functions)
        };
        Ok(super::condition_reload_state::StagedWorldTables {
            tables: Some(WorldTables {
                conditions,
                suffocation,
                files,
                priority_layers,
                items: Arc::new(items),
                timelines: sunrise_client::environment::TimelineTable {
                    timelines: packed_registry_definitions.timelines(),
                    tags: timeline_tags,
                },
                packed_registries: packed_registry_definitions,
                painting_variants,
                transformers,
                variants,
                providers,
                biomes,
                loot,
                loot_modifier_manager,
                recipes,
                stations,
                reload_sources: reload.then_some(sources),
                data_maps,
                pack_tags,
                gameplay,
                test_environments,
                test_instances,
            }),
            advancements,
            health,
            datapack_registries,
            datapack_vanilla_values,
            data,
        })
    }
    /// A world open's tables, and the first generation of the registries the mods declared.
    pub(super) fn open_world_tables(
        &mut self,
        staged: super::condition_reload_state::StagedWorldTables,
    ) -> Result<()> {
        self.world_biomes_base = staged
            .tables
            .as_ref()
            .map(|tables| tables.biomes.table.clone());
        self.world_biomes = self.world_biomes_base.clone();
        self.world_tables = staged.tables;
        self.health_resources = staged.health;
        self.mods.datapack_registries = staged.datapack_registries.map(Arc::new);
        self.mods.datapack_vanilla_values = staged.datapack_vanilla_values.map(Arc::new);
        self.fns.data = staged.data;
        self.adv.tree = staged.advancements;
        self.adv.reset_tree();
        self.load_server_loot();
        self.data_map_registries = self
            .world_tables
            .as_ref()
            .map_or_else(Vec::new, |t| t.data_maps.registries());
        self.install_block_maps();
        self.install_registry_snapshots();
        self.install_fluid_interactions()?;
        self.install_profession_save_snapshots();
        self.install_level_environments();
        self.sync_timeline_registries();
        self.share_mob_maps();
        self.share_biome_keys();
        self.sync_test_registry_ids();
        sunrise_client::datapack_net::serve(self.mods.datapack_registries.clone());
        Ok(())
    }
    /// Publish what gameplay and drawing read of the open world's data-driven registries, from
    /// the tables just opened: the block transformers, the trial spawner configs, the sulfur cube
    /// archetypes and the decorated pot patterns, replacing the last world's.
    pub(in crate::game) fn install_registry_snapshots(&self) {
        use sunrise_client::mobs::{sulfur_cube, trial_spawner};
        let Some(tables) = self.world_tables.as_ref() else {
            return;
        };
        let registries = &tables.packed_registries;
        self.res.set_transformers(tables.transformers.clone());
        trial_spawner::install(
            registries
                .validated("minecraft:trial_spawner")
                .into_iter()
                .filter_map(|(id, value)| {
                    let config =
                        <trial_spawner::TrialConfig as trial_spawner::TrialConfigExt>::decode(
                            &sunrise_client::scoreboard::json_to_tag(value),
                        )?;
                    Some((id, config))
                })
                .collect(),
        );
        sulfur_cube::install(sulfur_cube::build(
            &registries.validated("minecraft:sulfur_cube_archetype"),
            &tables.items.item_tags,
        ));
        self.install_pot_patterns(
            registries
                .validated("minecraft:decorated_pot_pattern")
                .into_iter()
                .filter_map(|(id, value)| Some((id, value["asset_id"].as_str()?.to_owned())))
                .collect(),
        );
    }
    /// The resource root's own `block_transformer` files as the world's registry, for tests that
    /// boot a game without opening a world.
    #[cfg(test)]
    pub(in crate::game) fn install_root_block_transformers(&self) {
        let dir = self
            .options
            .resources
            .join("data/minecraft/block_transformer");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let files: Vec<_> = entries
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path.extension().and_then(|e| e.to_str()) == Some("json")).then(|| {
                    let id = format!("minecraft:{}", path.file_stem().unwrap().to_string_lossy());
                    let value =
                        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap())
                            .unwrap();
                    (id, value)
                })
            })
            .collect();
        let transformers = sunrise_client::block_transform::Transformers::build(
            files.iter().map(|(id, value)| (id.as_str(), value)),
            self.pack.assets.registry,
        )
        .unwrap();
        self.res.set_transformers(Arc::new(transformers));
    }
    /// Publish the pot patterns of the registry (`id` to its `asset_id`) to the block entity renderer.
    pub(in crate::game) fn install_pot_patterns(&self, assets: HashMap<String, String>) {
        self.res.block_entity_render().set_pot_patterns(
            sunrise_client::block_entity_render::pot_pattern_textures(
                self.res.item_defaults(),
                |pattern| assets.get(pattern).cloned(),
            ),
        );
    }

    /// Publishes every level's dimension type overrides and the world's `minecraft:timeline`
    /// table to `Environment::set_level_attributes` so `EnvironmentAttributeSystem` reads
    /// pack timelines for built-in and custom levels alike.
    pub(super) fn install_level_environments(&self) {
        let Some(tables) = self.world_tables.as_ref() else {
            return;
        };
        let overrides = &tables.biomes.overrides;
        let levels = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ]
        .into_iter()
        .chain(self.level_shapes.extra.iter().map(|level| level.id));
        let attributes = levels
            .map(|level| {
                let json = overrides
                    .level_type_json(level)
                    .map_or(serde_json::Value::Null, |(_, json)| json);
                (level.to_owned(), json)
            })
            .collect();
        sunrise_client::environment::Environment::set_level_attributes(
            attributes,
            tables.timelines.clone(),
        );
    }
    /// Sends each joined Sunrise client the snapshot of the set just published, built from the
    /// offer owned by its connection. A refused replacement ends that connection with its reason.
    pub(super) fn send_datapack_snapshots(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        for guest in host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online() && g.brand == "sunrise")
        {
            sunrise_client::datapack_net::reload_connection(&guest.sender);
        }
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    /// Publish tables before function admission, leaving advancement rewards queued
    /// until the reload's new function store is available.
    pub(super) fn install_world_tables(&mut self, prepared: PreparedWorldReload) {
        let PreparedWorldReload {
            structure_spawns,
            tables,
            advancements,
            health,
            parts,
            wire,
            wire_tags,
            datapack_registries,
            datapack_vanilla_values,
            data,
        } = prepared;
        self.fns.data = data;
        self.close_stonecutters_for_reload();
        lan::sync::forget_server_sources();
        if let Some(tables) = &tables
            && let Some(sources) = &tables.reload_sources
        {
            let overlay = sunrise_world::loot::PackOverlay {
                conditions: &tables.conditions,
                files: &tables.files,
                priority_layers: &tables.priority_layers,
            };
            lan::sync::install_server_sources(
                &self.options.resources,
                &self.res.core.item_data,
                &overlay,
                sources.clone(),
            );
        }
        self.mods.datapack_registries = datapack_registries.map(Arc::new);
        self.mods.datapack_vanilla_values = datapack_vanilla_values.map(Arc::new);
        self.health_resources = health;
        self.world_tables = tables;
        // Publish the paired item and world evaluators before any callbacks.
        self.load_server_loot();
        self.data_map_registries = self
            .world_tables
            .as_ref()
            .map_or_else(Vec::new, |t| t.data_maps.registries());
        self.install_block_maps();
        self.install_registry_snapshots();
        if let Err(error) = self.install_fluid_interactions() {
            log::error!("Sunrise: {error}");
        }
        self.install_profession_save_snapshots();
        self.install_level_environments();
        self.sync_timeline_registries();
        self.share_mob_maps();
        self.share_biome_keys();
        self.world.pois.clear();
        for world in self.levels.parked_worlds() {
            world.pois.clear();
        }
        sunrise_client::datapack_net::serve(self.mods.datapack_registries.clone());
        self.refresh_recipe_property_context();
        self.structure_spawns = structure_spawns;
        self.refresh_infiniburn();
        self.install_parts(parts);
        self.damage_tag_generation = self.damage_tag_generation.wrapping_add(1);
        self.forget_book();
        // Before the players are sent the reloaded data below.
        self.sync_recipe_content();
        // PlayerList.reloadResources: the tags and recipes go to the players
        // and their advancements reload. The synchronized registries (the
        // dialogs, and the commands' ids of them) stay the hub's.
        lan::sync::install_wire_reload(
            &self.options.resources,
            &self.res.core.item_data,
            &self.pack_files(),
            wire.clone(),
            advancements.clone(),
            wire_tags,
        );
        if self.lan.is_some()
            && let Some(sync) = wire
        {
            self.with_advancement_hub(|hub, owners| hub.reload_resources(sync, owners));
        }
        // Send datapack snapshots after `UpdateTags` so clients re-decoding vanilla values
        // (`HolderSet` / `BlockPredicate` tags) already hold the reload's tags. When native mods
        // defer `finish_reload_resources` until `run_native_advancement_grants`, that path sends
        // the snapshots right after `UpdateTags`.
        if !self
            .lan
            .as_ref()
            .is_some_and(|lan| lan.hub.native_reload_pending)
        {
            self.send_datapack_snapshots();
        }
        self.adv.tree = advancements;
        self.adv.reset_tree();
        self.announce_data_bound(mod_api::TagsUpdateCause::ServerDataLoad { reload: true });
        self.resolve_enchant_particles();
        self.announce_data_maps_updated(mod_api::DataMapsUpdateCause::ServerReload);
        self.build_screen();
    }

    pub(super) fn join_world_rest(&mut self) -> Result<()> {
        if self.rest_loader.is_some() {
            let waited = Instant::now();
            log::info!("Sunrise: waiting for world resources");
            while self.rest_loader.is_some() {
                self.poll_world_rest()?;
                if self.rest_loader.is_some() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            log::info!(
                "Sunrise: world resources joined after {} ms",
                waited.elapsed().as_millis()
            );
        }
        if self.res.rest.get().is_none() {
            return Err("world resources are unavailable".into());
        }
        Ok(())
    }
}

impl Game<'_> {
    pub(super) fn active_enchantment_definitions(&self) -> &sunrise_client::enchant::Enchantments {
        self.item_enchantments()
    }
    /// Build and publish one server snapshot for this world load or reload.
    pub(super) fn load_server_loot(&mut self) {
        if let Some(tables) = &self.world_tables {
            self.server_items = Some(tables.items.clone());
            self.copy_loot_enchantment_answers(&tables.loot);
            self.server_loot.set(Some(tables.loot.clone()));
            global_loot_modifiers::bind_retained(
                &self.reload_resources,
                &tables.loot_modifier_manager,
            );
            return;
        }
        self.server_items = None;
        let overlay = pack_files(&self.world_tables);
        let native = if mod_api::__private::mods().is_empty() {
            None
        } else {
            match lan::sync::server_data_load(&self.options.resources, &self.res.core.item_data) {
                Ok(context) => Some(context),
                Err(error) => {
                    log::warn!("Sunrise: server loot inventory unavailable: {error}");
                    self.server_loot.set(None);
                    return;
                }
            }
        };
        let loot = match sunrise_world::loot::LootData::load_with_damage(
            &self.options.resources,
            &overlay,
            self.res.item_defaults(),
            &self.health_resources.damage,
            &self.items().ordered_tags(),
            native.as_ref(),
        ) {
            Ok(loot) => Some(Arc::new(loot)),
            Err(error) => {
                log::warn!("Sunrise: server loot tables unavailable: {error}");
                None
            }
        };
        if let Some(loot) = &loot {
            let mut items = self.items().clone();
            items.set_loot_data(loot);
            self.server_items = Some(Arc::new(items));
            self.copy_loot_enchantment_answers(loot);
        }
        self.server_loot.set(loot);
    }

    pub(super) fn prepare_parts(&self, items: &Items) -> Result<PreparedParts> {
        // Some unit fixtures construct the Part tables without loading game content.
        #[cfg(not(test))]
        if !mod_api::__private::content_owners_published() {
            return Err("Mod parts refused: content ownership has not been published".into());
        }
        let registry = self.pack.assets.registry;
        let block_tags = &items.block_tags;
        let parts = mod_api::__private::prepare_parts(
            block_tags,
            &items.item_tags,
            mod_api::__private::KnownIds {
                block: &|id| registry.default_state(id).is_some(),
                item: &|id| items.items.contains_key(id),
            },
        )
        .map_err(|refused| match refused {
            mod_api::__private::ResolvePartsError::ItemKeys(errors) => format!(
                "Mod item keys refused:\n{}",
                errors
                    .into_iter()
                    .map(|e| format!("mod {} requested unknown item {}", e.mod_id, e.id))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            mod_api::__private::ResolvePartsError::Parts(errors) => {
                format!("Mod parts refused:\n{}", errors.join("\n"))
            }
            _ => "Mod parts refused".to_owned(),
        })?;
        let block_states = api_values::prepare_block_states(registry)?;
        let cauldrons = self.prepare_cauldron_interactions(items)?;
        Ok(PreparedParts {
            cauldrons,
            block_states,
            parts,
        })
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    pub(super) fn prepare_world_reload(
        &mut self,
        stack: &functions::PackStack,
        root: &std::path::Path,
    ) -> Result<PreparedWorldReload> {
        let super::condition_reload_state::StagedWorldTables {
            tables,
            advancements,
            health,
            datapack_registries,
            datapack_vanilla_values,
            data,
            ..
        } = self.stage_world_tables(stack, true, root)?;
        let items = tables
            .as_ref()
            .map_or_else(|| self.res.shared_items(), |tables| tables.items.clone());
        let parts = self.prepare_parts(&items)?;
        let wire_tags = sunrise_client::lan::data::WorldTags::new(items.clone(), &health.damage);
        let overlay = pack_files(&tables);
        let kept = self
            .lan
            .as_ref()
            .map(|host| host.hub.lan.shared.sync())
            .or_else(lan::sync::cached_reload_sync);
        let wire = if let Some(kept) = kept {
            let sources = match tables
                .as_ref()
                .and_then(|tables| tables.reload_sources.clone())
            {
                Some(sources) => sources,
                None => {
                    let sources = lan::sync::prepare_server_sources(
                        root,
                        &self.res.core.item_data,
                        &items,
                        &overlay,
                    )?;
                    sources.check()?;
                    sources
                }
            };
            Some(
                lan::sync::prepare_wire_reload(
                    root,
                    &self.res.core.item_data,
                    &overlay,
                    &kept,
                    &sources,
                    &advancements,
                    &wire_tags,
                )
                .map_err(|error| format!("wire reload: {error}"))?,
            )
        } else {
            None
        };
        overlay.conditions.check_errors()?;
        Ok(PreparedWorldReload {
            advancements,
            structure_spawns: sunrise_client::mobs::spawn_selection::StructureSpawns::load(root),
            tables,
            health,
            parts,
            wire,
            wire_tags,
            datapack_registries,
            datapack_vanilla_values,
            data,
        })
    }

    /// Resolve the mods' parts against the tags this game plays with (`/reload` again): mods
    /// that attach parts to a block or item it doesn't have are an error.
    pub(super) fn resolve_parts(&self) -> Result<()> {
        let prepared = self.prepare_parts(self.items())?;
        if self
            .remote
            .as_ref()
            .is_some_and(|r| !r.registries.tags.is_empty())
        {
            // During login this binds the local state keys. The effective item snapshot
            // belongs to install_remote_tags, after it publishes the server's tag view.
            self.bind_parts(prepared);
        } else {
            self.install_parts(prepared);
        }
        self.resolve_enchant_particles();
        Ok(())
    }
    /// Bind parts, then resolve support and primary item answers against the published tags.
    pub(super) fn install_parts(&self, prepared: PreparedParts) {
        self.bind_parts(prepared);
        self.resolve_primary_items();
    }
    /// Publish state keys and bind parts before their state-only answers are cached.
    /// A server also publishes its parts for world generation.
    pub(super) fn bind_parts(&self, prepared: PreparedParts) {
        if self.is_logical_server()
            && let Some(host) = &self.lan
        {
            cauldron_prediction::publish_registrations(
                &prepared.cauldrons,
                host.hub
                    .guests
                    .simulation_iter()
                    .filter(|g| g.placed || g.held),
            );
        }
        cauldron_interactions::install(prepared.cauldrons);
        if self.is_logical_server() {
            native_worldgen_parts::publish(
                prepared.parts.as_ref(),
                &prepared.block_states,
                self.pack.assets.registry,
            );
        }
        api_values::install_block_states(&self.tick, prepared.block_states);
        api_values::install_resolved_parts(&self.tick, prepared.parts);
        self.mark_collision_shape_states();
        self.resolve_block_paths();
        self.resolve_piston_reactions();
    }
    pub(super) fn resolve_block_paths(&self) {
        use mod_api::{HookId, PathComputationType};
        use sunrise_client::mobs::data::PartAnswers;
        let data = self.res.mob_data();
        let pathfindable =
            Self::resolve_per_state(HookId::IsPathfindable, data.states.len(), |state| {
                PathComputationType::ALL
                    .into_iter()
                    .filter(|&kind| {
                        let mut pathfindable = data.block_pathfindable(state, kind);
                        self.native_is_pathfindable(state, kind, &mut pathfindable);
                        pathfindable
                    })
                    .fold(0, |bits, kind| bits | PartAnswers::pathfindable_bit(kind))
            });
        data.resolve_parts(
            self.res.facts(),
            &PartAnswers {
                pathfindable: pathfindable.as_deref(),
            },
        );
    }
}

/// Bind the world-owned registries' ordered tags (enchantments, biomes, structures,
/// and world clocks) onto `items` against the admitted entries.
pub(super) fn bind_world_registry_tags(
    items: &mut Items,
    root: &std::path::Path,
    tag_files: &sunrise_world::loot::PackOverlay<'_>,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    biomes: &Biomes,
) -> Result<()> {
    items.bind_enchantment_tags(root, tag_files)?;
    items.bind_registry_tags(
        root,
        tag_files,
        sunrise_client::items::TagRegistry::Biome,
        "worldgen/biome",
        |id| biomes.contains(id),
    )?;
    let mut structures: std::collections::HashSet<String> = items
        .component_registry
        .ids("minecraft:worldgen/structure")
        .iter()
        .cloned()
        .collect();
    for (id, _) in overlay.kind("worldgen/structure") {
        structures.insert(id.to_owned());
    }
    items.bind_registry_tags(
        root,
        tag_files,
        sunrise_client::items::TagRegistry::Structure,
        "worldgen/structure",
        |id| structures.contains(id),
    )?;
    let mut clocks: std::collections::HashSet<String> = items
        .component_registry
        .ids("minecraft:world_clock")
        .iter()
        .cloned()
        .collect();
    for id in mod_api::__private::world_clock_declarations() {
        clocks.insert(id.to_owned());
    }
    for (id, _) in overlay.kind("world_clock") {
        clocks.insert(id.to_owned());
    }
    items.bind_registry_tags(
        root,
        tag_files,
        sunrise_client::items::TagRegistry::WorldClock,
        "world_clock",
        |id| clocks.contains(id),
    )?;
    Ok(())
}

/// [`Game::level_suffocation`] from the fields it reads, for a caller that holds other fields: the
/// world's own tables (its datapacks' `causes_suffocation`), else the built-in ones.
pub(super) fn level_suffocation_of<'a>(
    world_tables: &'a Option<WorldTables>,
    res: &'a Resources,
) -> &'a sunrise_world::suffocation::SuffocationRegistry<'static> {
    world_tables
        .as_ref()
        .map_or_else(|| res.suffocation(), |t| &t.suffocation)
}

#[cfg(test)]
mod creation_read_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

    #[test]
    #[ignore = "daily"]
    fn the_creation_read_refuses_a_pack_with_an_empty_jukebox_song() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root,
            )),
            None,
        )
        .unwrap();
        game.build_registries();

        let folder =
            std::env::temp_dir().join(format!("sunrise-creation-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        // Where Create World's Data Packs screen lists it: directly in the `mcworld-` folder.
        let pack = folder.join("bad_song");
        std::fs::create_dir_all(pack.join("data/test/jukebox_song")).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(pack.join("data/test/jukebox_song/empty.json"), "{}").unwrap();

        assert!(game.read_creation_packs(Some(&folder), &[]).is_ok());
        assert!(
            game.read_creation_packs(Some(&folder), &["file/bad_song".to_owned()])
                .is_err()
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
