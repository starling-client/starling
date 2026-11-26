//! The server's gameplay tags. Typed mod tag notices remain in remote_tags.
use super::super::Game;
use std::{collections::HashMap, sync::Arc};
use sunrise_client::items::EntityTypeTags;
impl Game<'_> {
    /// Apply the server's entity type tags on every client, including the
    /// integrated server's window. Preserve the integrated client's block
    /// and item tables.
    pub(in crate::game) fn install_server_items(&mut self) {
        let Some(remote) = self.remote.as_ref() else {
            return;
        };
        let names = |registry: &str| -> Option<HashMap<String, Vec<String>>> {
            let ids = remote.builtin.get(registry)?;
            let tags = remote
                .registries
                .tags
                .get(registry)?
                .iter()
                .map(|(name, members)| {
                    let members = members.iter().filter_map(|&id| ids.name(id));
                    (name.clone(), members.map(str::to_owned).collect())
                });
            Some(tags.collect())
        };
        let (blocks, items) = if self.integrated.is_none() {
            (names("minecraft:block"), names("minecraft:item"))
        } else {
            (None, None)
        };
        let base = self
            .world_tables
            .as_ref()
            .map_or_else(|| self.res.items(), |tables| &tables.items);
        let mut items = base.with_tags(blocks, items);
        let tags = EntityTypeTags::from_ordered(
            names(EntityTypeTags::REGISTRY).unwrap_or_default(),
            base.entity_type_tags.ids().clone(),
        );
        items.set_entity_type_tags(tags);
        if let Some(enchantments) = self.remote.as_mut().and_then(|r| r.enchantments.as_mut()) {
            Arc::make_mut(enchantments).entity_tags = items.entity_type_tags.members().clone();
        }
        self.server_items = Some(Arc::new(items));
        self.install_remote_registry_snapshots();
    }
    /// The synchronized registries gameplay and drawing read, from the server's registry packet:
    /// the block transformers, the sulfur cube archetypes and the decorated pot patterns. An
    /// entry the server sent without data (a known pack's) is the resource root's own file.
    /// The integrated server's window shares the tables its server thread installed.
    fn install_remote_registry_snapshots(&self) {
        use sunrise_client::mobs::sulfur_cube;
        if self.integrated.is_some() {
            return;
        }
        let Some(remote) = self.remote.as_ref() else {
            return;
        };
        let entries = |registry: &str| -> Vec<(String, serde_json::Value)> {
            let kind = registry.trim_start_matches("minecraft:");
            let mut found: Vec<_> = remote
                .registries
                .registries
                .get(registry)
                .into_iter()
                .flatten()
                .filter_map(|(name, data)| {
                    let value = match data {
                        Some(tag) => sunrise_client::scoreboard::tag_to_json(tag),
                        None => {
                            let (namespace, path) = name.split_once(':')?;
                            let file = self
                                .options
                                .resources
                                .join(format!("data/{namespace}/{kind}/{path}.json"));
                            let read = std::fs::read(&file)
                                .map_err(|error| error.to_string())
                                .and_then(|bytes| {
                                    serde_json::from_slice(&bytes)
                                        .map_err(|error| error.to_string())
                                });
                            match read {
                                Ok(value) => value,
                                Err(error) => {
                                    log::error!(
                                        "Sunrise: the server's {registry} entry {name} is not readable from {}: {error}",
                                        file.display()
                                    );
                                    return None;
                                }
                            }
                        }
                    };
                    Some((name.clone(), value))
                })
                .collect();
            found.sort_by(|(a, _), (b, _)| {
                let key = |name: &str| {
                    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
                    (path.to_owned(), namespace.to_owned())
                };
                key(a).cmp(&key(b))
            });
            found
        };
        let transformers = entries("minecraft:block_transformer");
        match sunrise_client::block_transform::Transformers::build(
            transformers.iter().map(|(id, value)| (id.as_str(), value)),
            self.pack.assets.registry,
        ) {
            Ok(built) => self.res.set_transformers(Arc::new(built)),
            Err(error) => {
                log::error!("Sunrise: the server's block transformers: {error}");
                self.res.set_transformers(Arc::default());
            }
        }
        let archetypes = entries("minecraft:sulfur_cube_archetype");
        let tags = self
            .server_items
            .as_ref()
            .map_or_else(HashMap::new, |items| items.item_tags.clone());
        sulfur_cube::install(sulfur_cube::build(
            &archetypes
                .iter()
                .map(|(id, value)| (id.clone(), value))
                .collect::<Vec<_>>(),
            &tags,
        ));
        self.install_pot_patterns(
            entries("minecraft:decorated_pot_pattern")
                .into_iter()
                .filter_map(|(id, value)| Some((id, value["asset_id"].as_str()?.to_owned())))
                .collect(),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::super::super::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use std::{collections::HashSet, sync::Arc};
    use sunrise_client::{items::ItemStack, mobs::equip::equippable_in_slot};
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        connection::Pipe,
        packets::configuration::{RegistryTags, TagEntry, UpdateTags},
    };

    fn client<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
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
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let res = &*resources.insert(loaded.resources);
        let mut game = Game::new(
            res,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "TagTest").unwrap();
        game
    }

    #[test]
    #[ignore = "daily"]
    fn default_and_patched_equippables_follow_the_servers_entity_tags() {
        crate::game::in_own_process(
            "game::remote::tables::tests::default_and_patched_equippables_follow_the_servers_entity_tags",
            || {
                let mut resources = None;
                let mut game = client(&mut resources);
                let builtin = game.remote.as_ref().unwrap().builtin.clone();
                let registry = "minecraft:entity_type";
                let tag = "minecraft:can_equip_saddle";
                let update = |entity: &str| UpdateTags {
                    registries: vec![RegistryTags {
                        registry: registry.into(),
                        tags: vec![TagEntry {
                            name: tag.into(),
                            ids: vec![builtin.get(registry).unwrap().id(entity).unwrap()],
                        }],
                    }],
                };
                let horse = "minecraft:horse";
                let camel = "minecraft:camel";
                let saddle = ItemStack::new("minecraft:saddle", 1);
                let mut patched = ItemStack::new("minecraft:stick", 1);
                patched.data_mut().set_other(
                    "minecraft:equippable",
                    Some(Tag::compound([
                        ("slot", Tag::String("saddle".into())),
                        ("allowed_entities", Tag::String("#can_equip_saddle".into())),
                    ])),
                );
                let assert_equips = |game: &Game<'_>, allowed, denied| {
                    for stack in [&saddle, &patched] {
                        assert!(equippable_in_slot(game.items(), stack, "saddle", allowed));
                        assert!(!equippable_in_slot(game.items(), stack, "saddle", denied));
                    }
                };

                // Configuration stores the tags before the client installs the server tables.
                Arc::make_mut(&mut game.remote.as_mut().unwrap().registries)
                    .add_tags(&update(horse));
                game.install_server_items();
                assert_equips(&game, horse, camel);

                game.remote_update_tags(&update(camel));
                assert_equips(&game, camel, horse);
            },
        );
    }

    /// A server whose data pack makes stone climbable and sticks planks: the
    /// client reads its tags, not the game's own (#7843).
    #[test]
    #[ignore = "daily"]
    fn a_servers_block_and_item_tags_replace_the_games() {
        let mut resources = None;
        let mut game = client(&mut resources);
        assert!(!game.items().block_tags["minecraft:climbable"].contains("minecraft:stone"));
        let builtin = game.remote.as_ref().unwrap().builtin.clone();
        let id = |registry: &str, name: &str| builtin.get(registry).unwrap().id(name).unwrap();
        let tag = |registry: &str, name: &str, members: &[&str]| RegistryTags {
            registry: registry.into(),
            tags: vec![TagEntry {
                name: name.into(),
                ids: members.iter().map(|m| id(registry, m)).collect(),
            }],
        };
        game.remote_update_tags(&UpdateTags {
            registries: vec![
                tag(
                    "minecraft:block",
                    "minecraft:climbable",
                    &["minecraft:ladder", "minecraft:stone"],
                ),
                tag(
                    "minecraft:item",
                    "minecraft:planks",
                    &["minecraft:stick", "minecraft:oak_planks"],
                ),
            ],
        });

        let items = game.items();
        let set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<HashSet<_>>();
        assert_eq!(
            items.block_tags["minecraft:climbable"],
            set(&["minecraft:ladder", "minecraft:stone"])
        );
        // The server's tags replace the game's: a tag it didn't send is gone.
        assert!(!items.block_tags.contains_key("minecraft:fences"));
        assert_eq!(
            items.item_tag_order["minecraft:planks"],
            ["minecraft:stick", "minecraft:oak_planks"]
        );
        assert!(items.item_tags["minecraft:planks"].contains("minecraft:stick"));
    }
}
