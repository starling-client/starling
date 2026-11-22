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
