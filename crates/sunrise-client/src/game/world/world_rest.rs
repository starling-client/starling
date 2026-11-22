use super::*;
use sunrise_client::mobs::spawner::SpawnerTypeFacts;

pub(super) fn load_world_rest(
    resource_root: &sunrise_assets::resource_source::ResourceRoot,
    conditions: &sunrise_world::conditions::LoadConditions,
    resources: &Resources,
    spawner_types: HashMap<String, SpawnerTypeFacts>,
    assets: &Assets,
    behaviour_json: &serde_json::Value,
    gameplay_projection: &[sunrise_client::blocks::GameplayProjection],
    entity_models: Option<&sunrise_client::mobs::EntityModelDocument>,
    item_defaults: &sunrise_world::item_defaults::ItemDefaults,
    visibility: sunrise_world::block_render::FaceVisibility,
    progress: &Progress,
    started: Instant,
    // False on a dedicated server, which draws nothing: no block entity
    // renderer or animate tables (client-side display ticks).
    draws: bool,
) -> Result<WorldRest> {
    let root = resource_root.prepared();
    let facts = resources.facts();
    let biomes = resources.biomes();
    let threads = resources.threads();
    let registry = assets.registry;
    let content = sunrise_client::datapack::DataPack::from_source(
        "sunrise-content".into(),
        resource_root.data_changes().clone(),
    );
    let store = sunrise_client::datapack::DataStore::load(&[&content]);
    let files = store.files.into_iter().collect::<Vec<_>>();
    let overlay = &sunrise_world::loot::PackOverlay {
        conditions,
        files: &files,
        priority_layers: &store.priority_layers,
    };
    progress.set(6);
    // Independent tables load together, within the worker budget the settings
    // allow. Only the recipes and the stations read another table: the items.
    let load_items = || -> Result<_> {
        let mut items = startup::time("rest.items", || {
            Items::load(root, registry, facts, overlay, resources.item_defaults())
        })?;
        items.bind_component_damage_catalog(&resources.motion().health.damage);
        api_values::resolve_item_registry(&items)?;
        log::info!("load items: {:.2}s", started.elapsed().as_secs_f32());
        progress.set(7);
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let _recipe_files = sunrise_client::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = startup::time("rest.recipe_sources", || {
            lan::sync::initial_sources(root, &resources.core.item_data, &items, overlay)
        })?;
        let recipes = startup::time("rest.recipes", || {
            Recipes::from_resolved(root, &items, overlay, &sources)
        })?;
        let stations = startup::time("rest.stations", || {
            sunrise_client::stations::StationData::from_sources(
                root,
                &items,
                overlay,
                resources.item_defaults(),
                (&sources, &recipes),
            )
        });
        Ok((items, recipes, stations))
    };
    let load_behaviours = || {
        startup::time("rest.behaviours", || {
            Behaviours::load_with_gameplay(registry, root, behaviour_json, gameplay_projection)
        })
    };
    let load_block_entity_render = || -> Result<_> {
        let _step = startup::step("rest.block_entity_render");
        if !draws {
            return Ok(Default::default());
        }
        let mut renderer = BlockEntityRenderer::load_with_visibility(
            resource_root,
            registry,
            visibility,
            resources.item_defaults(),
        )?;
        renderer.add_mod_signs(facts, registry);
        Ok(renderer)
    };
    let load_animate = || resource_tables::load_animate_tables(root, registry, draws, biomes);
    let load_mobs = || -> Result<_> {
        let _step = startup::step("rest.mobs");
        let catalog = &resources.motion().attributes;
        let mut types = if draws {
            match entity_models {
                Some(document) => MobTypes::load_with_document_and_defaults(
                    root,
                    document,
                    item_defaults.entries(),
                    catalog,
                )?,
                None => MobTypes::load_with_defaults(root, item_defaults.entries(), catalog)?,
            }
        } else {
            MobTypes::load_for_server(root, catalog)?
        };
        // #845: the declared sets take their types' rows — the merged rows
        // the live attribute map's catalog took in core (the template's with
        // the declared set on top) — as their attribute map and spawn
        // fields, so a mob of the type spawns with what the map answers.
        for declaration in mod_api::__private::entity_attribute_declarations() {
            let row = catalog.entity_row(&declaration.id).map(std::sync::Arc::new);
            match (types.types.get_mut(&declaration.id), row) {
                (Some(kind), Some(row)) => kind.take_attribute_row(row, catalog),
                (Some(_), None) => log::warn!(
                    "Sunrise: mod entity attributes of {} never reached the catalog; ignored",
                    declaration.id
                ),
                (None, _) => log::warn!(
                    "Sunrise: mod entity attributes of {} name no loaded entity type; ignored",
                    declaration.id
                ),
            }
        }
        // #846: the types mods added attributes to take their grown rows the
        // same way, so a new mob of the type spawns with them.
        resource_load::take_modified_attribute_rows(
            &mut types.types,
            catalog,
            mod_api::__private::entity_attribute_modifications(),
        );
        // The types mods name to `entity_tick_each` hear their ticks one entity at a time
        // (game/modding/native_ticks.rs): marked once here, so a tick reads a flag.
        let each = mod_api::__private::entity_tick_each();
        let mob_types: HashMap<_, _> = types
            .types
            .into_iter()
            .map(|(k, mut v)| {
                v.flags.tick_alone = each.is_some_and(|each| each.names(&v.id));
                (k, Arc::new(v))
            })
            .collect();
        let mut mob_data = sunrise_client::mobs::MobData::load(resource_root, facts);
        // NeoForge keys placements by EntityType, not by the subset backed by MobTypes.
        // Use exactly the registry the spawners will consult, with facts already validated.
        mob_data
            .install_spawn_placements(mod_api::__private::spawn_placement_declarations(), |id| {
                spawner_types.contains_key(id)
            })?;
        crate::game::entity_part_state::refuse_entity_part_state_kinds(|kind| {
            mob_types.contains_key(kind)
        })?;
        mob_data.cem = std::sync::RwLock::new(Arc::new(if draws {
            sunrise_client::mobs::cem::CemModels::load(resource_root.source(), &mob_types)
        } else {
            sunrise_client::mobs::cem::CemModels::default()
        }));
        sunrise_client::mobs::trial_spawner::load_registry(root);
        Ok((mob_types, mob_data))
    };
    let load_transformers = || -> Result<_> {
        let _step = startup::step("rest.transformers_models");
        let transformers = sunrise_client::block_transform::Transformers::load(root, registry)?;
        let player_type = match entity_models {
            Some(document) => perspective::player_type_with_document(root, document),
            None => perspective::player_type(root),
        };
        let models = (
            player_type,
            player_arms(root, "minecraft:player#main", entity_models),
            player_arms(root, "minecraft:player_slim#main", entity_models),
            match entity_models {
                Some(document) => document.model("minecraft:spin_attack#main"),
                None => sunrise_client::mobs::load_model(root, "minecraft:spin_attack#main"),
            },
        );
        Ok((transformers, models))
    };
    // The errors cross threads as text.
    fn text<T, E: std::fmt::Display>(
        result: std::result::Result<T, E>,
    ) -> std::result::Result<T, String> {
        result.map_err(|e| e.to_string())
    }
    let (
        items,
        behaviours,
        block_entity_render,
        animate,
        mobs,
        transformers,
        providers,
        music,
        ambient,
        nether_environment,
        end_environment,
    ) = if threads > 1 {
        std::thread::scope(|scope| {
            let items = named_scope(scope, "items", || text(load_items()));
            let behaviours = named_scope(scope, "behaviours", || text(load_behaviours()));
            let block_entity_render = named_scope(scope, "block_entity_render", || {
                text(load_block_entity_render())
            });
            let animate = named_scope(scope, "animate", || text(load_animate()));
            let mobs = named_scope(scope, "mobs", || text(load_mobs()));
            let transformers = named_scope(scope, "transformers", || text(load_transformers()));
            let providers = named_scope(scope, "providers", || {
                startup::time("rest.providers", || Providers::load(root))
            });
            let music = named_scope(scope, "music", || {
                startup::time("rest.music", || {
                    sunrise_client::music::MusicTables::load(root)
                })
            });
            let ambient = named_scope(scope, "ambient", || {
                startup::time("rest.ambient", || {
                    sunrise_client::ambience::AmbientTables::load(root)
                })
            });
            let nether_environment = named_scope(scope, "nether_environment", || {
                startup::time("rest.nether_environment", || {
                    text(Environment::load_dimension(
                        &sunrise_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_nether",
                    ))
                })
            });
            let end_environment = named_scope(scope, "end_environment", || {
                startup::time("rest.end_environment", || {
                    text(Environment::load_dimension(
                        &sunrise_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_end",
                    ))
                })
            });
            (
                items.join().unwrap(),
                behaviours.join().unwrap(),
                block_entity_render.join().unwrap(),
                animate.join().unwrap(),
                mobs.join().unwrap(),
                transformers.join().unwrap(),
                providers.join().unwrap(),
                music.join().unwrap(),
                ambient.join().unwrap(),
                nether_environment.join().unwrap(),
                end_environment.join().unwrap(),
            )
        })
    } else {
        (
            text(load_items()),
            text(load_behaviours()),
            text(load_block_entity_render()),
            text(load_animate()),
            text(load_mobs()),
            text(load_transformers()),
            Providers::load(root),
            startup::time("rest.music", || {
                sunrise_client::music::MusicTables::load(root)
            }),
            startup::time("rest.ambient", || {
                sunrise_client::ambience::AmbientTables::load(root)
            }),
            startup::time("rest.nether_environment", || {
                text(Environment::load_dimension(
                    &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_nether",
                ))
            }),
            startup::time("rest.end_environment", || {
                text(Environment::load_dimension(
                    &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_end",
                ))
            }),
        )
    };
    // World staging checks conditions after the selected packs have supplied their winners.
    let (items, recipes, stations) = items?;
    let (animate, fog) = animate?;
    let (mob_types, mut mob_data) = mobs?;
    let (transformers, (player_type, player_arm, player_arm_slim, spin_attack)) = transformers?;
    if draws {
        mob_data.frame_models = startup::time("rest.frame_models", || {
            sunrise_client::mobs::render::BakedFrameModels::bake(
                mob_types
                    .values()
                    .chain(player_type.iter())
                    .map(Arc::as_ref),
            )
        });
    }
    let nether_environment = nether_environment?;
    let end_environment = end_environment?;
    log::info!(
        "Sunrise: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    progress.set(8);
    Ok(WorldRest {
        stations,
        items: Arc::new(items),
        recipes,
        transformers,
        providers,
        block_entity_render: block_entity_render?,
        animate,
        fog,
        music,
        ambient,
        nether_environment,
        end_environment,
        behaviours: Arc::new(behaviours?),
        mob_types,
        mob_data,
        spawner_types,
        player_type,
        player_arm,
        player_arm_slim,
        spin_attack,
    })
}
