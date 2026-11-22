use super::*;
use crate::game::native_entity_refs::tests::{fixture_root, with_client_game, with_game_role};
use crate::game::{execution_role::ExecutionRole, lan::LanHost};
use mod_api::prelude::ItemsHost;
use mod_api::{BuildCreativeModeTabContentsEvent, Hooks, HostSide, ModEntry};
use std::sync::Mutex;
use sunrise_client::lan::{
    Lan, NetConfig,
    chunks::ChunkEncoder,
    data::SyncData,
    hub::{Hub, HubConfig},
};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_world::loot::PackOverlay;

#[derive(Default)]
struct Seen {
    events: Vec<(LogicalSide, CreativeTabPermissions, String)>,
    token: Option<CreativeTabEdit>,
}

struct Edit(Arc<Mutex<Seen>>);

fn stack(id: &str, count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(mod_api::__private::item_key(id).unwrap(), count).unwrap()
}

fn stone_key(max_stack: i32) -> mod_api::ItemStack {
    let mut stone = stack("minecraft:stone", 42);
    stone
        .set_components(Compound::from([(
            "minecraft:max_stack_size".into(),
            Tag::Int(max_stack),
        )]))
        .unwrap();
    stone
}

#[mod_api::__subscriptions]
impl Hooks for Edit {
    fn build_creative_mode_tab_contents(
        &self,
        host: HostSide,
        event: &mut BuildCreativeModeTabContentsEvent<'_>,
    ) {
        let (host, side): (&mut dyn mod_api::Host, _) = match host {
            HostSide::Client(host) => (host, LogicalSide::Client),
            HostSide::Server(host) => (host, LogicalSide::Server),
        };
        {
            let mut seen = self.0.lock().unwrap();
            seen.events
                .push((side, event.permissions, event.tab.to_owned()));
            seen.token = Some(event.edit);
        }
        if event.tab != "test:a" {
            return;
        }
        let previous = host.creative_tabs(event.permissions).unwrap();
        let b = &previous.tab("test:b").unwrap().display_items;
        match event.permissions {
            CreativeTabPermissions::Normal => assert!(b.is_empty()),
            CreativeTabPermissions::Operator => {
                assert_eq!(b, &[stack("minecraft:sand", 1)]);
                let published = host.built_creative_tabs().unwrap().unwrap();
                assert_eq!(published.permissions, CreativeTabPermissions::Normal);
                assert_eq!(
                    published.tab("test:b").unwrap().display_items,
                    [stack("minecraft:sand", 1)]
                );
            }
        }
        let other = match event.permissions {
            CreativeTabPermissions::Normal => CreativeTabPermissions::Operator,
            CreativeTabPermissions::Operator => CreativeTabPermissions::Normal,
        };
        assert!(matches!(
            host.creative_tabs(other),
            Err(CreativeTabReadError::Reentrant)
        ));
        assert_eq!(
            event.edit.insert(
                host,
                CreativeTabPlacement::Last,
                stack("minecraft:dirt", 2),
                CreativeTabVisibility::Both
            ),
            Err(CreativeTabEditError::Count),
        );
        assert_eq!(
            event.edit.insert(
                host,
                CreativeTabPlacement::Before(stone_key(64)),
                stack("minecraft:dirt", 1),
                CreativeTabVisibility::Both
            ),
            Err(CreativeTabEditError::MissingAnchor(CreativeTabList::Search)),
        );
        let mut absent = stack("minecraft:iron_sword", 50);
        absent.set_damage(1000);
        assert_eq!(
            event.edit.insert(
                host,
                CreativeTabPlacement::After(absent.clone()),
                stack("minecraft:glass", 1),
                CreativeTabVisibility::Parent,
            ),
            Err(CreativeTabEditError::MissingAnchor(CreativeTabList::Parent))
        );
        event
            .edit
            .remove(host, &absent, CreativeTabVisibility::Both)
            .unwrap();
        absent.set_damage(u32::MAX);
        assert_eq!(
            event
                .edit
                .remove(host, &absent, CreativeTabVisibility::Both),
            Err(CreativeTabEditError::Stack(
                mod_api::StackWriteError::InvalidComponents {
                    reason: mod_api::StackError::InvalidDamage,
                },
            )),
        );
        assert_eq!(
            event.edit.insert(
                host,
                CreativeTabPlacement::Last,
                stack("minecraft:stick", 1),
                CreativeTabVisibility::Both,
            ),
            Err(CreativeTabEditError::Duplicate(CreativeTabList::Search))
        );
        // The disabled barrier still anchors the insertion during the event.
        event
            .edit
            .insert(
                host,
                CreativeTabPlacement::Before(stack("minecraft:barrier", 2)),
                stack("minecraft:torch", 1),
                CreativeTabVisibility::Search,
            )
            .unwrap();
        event
            .edit
            .remove(
                host,
                &stack("minecraft:stick", 47),
                CreativeTabVisibility::Search,
            )
            .unwrap();
    }
}

struct Observe;

#[mod_api::__subscriptions]
impl Hooks for Observe {
    fn build_creative_mode_tab_contents(
        &self,
        host: HostSide,
        event: &mut BuildCreativeModeTabContentsEvent<'_>,
    ) {
        if event.tab != "test:a" {
            return;
        }
        let host: &dyn mod_api::Host = match host {
            HostSide::Client(host) => host,
            HostSide::Server(host) => host,
        };
        assert_eq!(
            event.edit.entries(host, CreativeTabList::Parent).unwrap(),
            [
                stack("minecraft:dirt", 1),
                stack("minecraft:stone", 1),
                stack("minecraft:barrier", 1),
                stack("minecraft:stick", 1)
            ]
        );
        assert_eq!(
            event.edit.entries(host, CreativeTabList::Search).unwrap(),
            [stack("minecraft:torch", 1), stack("minecraft:barrier", 1)]
        );
        let mut stone = stone_key(64);
        stone.set_count(1).unwrap();
        event
            .edit
            .insert(
                host,
                CreativeTabPlacement::Last,
                stone,
                CreativeTabVisibility::Search,
            )
            .unwrap();
    }
}

fn definition(max_level: i32) -> serde_json::Value {
    serde_json::json!({
        "description": {"text":"Fixture levels"}, "supported_items":"minecraft:iron_sword",
        "weight": 1, "anvil_cost": 1, "max_level": max_level,
        "min_cost": {"base": 1,"per_level_above_first": 1},
        "max_cost": {"base": 10,"per_level_above_first": 1}, "slots": ["mainhand"], "effects": {},
    })
}

fn install_dedicated_holders(game: &mut Game<'_>, max_level: i32) {
    let files = vec![(
        "enchantment/test:levels".into(),
        vec![definition(max_level).to_string().into_bytes()],
    )];
    let sync = Arc::new(
        SyncData::load(
            &game.options.resources,
            &PackOverlay::new(&files),
            game.res.item_defaults(),
            &sunrise_client::crafting::ResolvedSources::default(),
        )
        .unwrap(),
    );
    let net = NetConfig {
        listen: false,
        ..NetConfig::loopback(0, String::new(), 2)
    };
    let lan = Lan::start_with(sync.clone(), net).unwrap();
    let encoder = ChunkEncoder::new(
        [] as [(Arc<str>, bool); 0],
        &sync.builtin,
        &[],
        &[],
        Default::default(),
    );
    let config = HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: false,
        seed: 0,
        max_players: 2,
        integrated: false,
        levels: game.level_shapes,
        level_seeds: game.levels.runtime.seeds(),
    };
    game.lan = Some(LanHost::for_test(Hub::new(
        lan,
        sync,
        encoder,
        config,
        game.loop_context.clone(),
    )));
    game.native_lifecycle_open_started();
    game.native_lifecycle.world_open_ready();
}

fn replace_client_definition(game: &mut Game<'_>, max_level: i32) {
    let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
    registries.registries.insert(
        "minecraft:enchantment".into(),
        vec![(
            "test:levels".into(),
            Some(sunrise_client::scoreboard::json_to_tag(&definition(
                max_level,
            ))),
        )],
    );
    game.set_registries(Arc::new(registries));
}

fn book_level(snapshot: &CreativeModeTabs) -> i32 {
    let books = &snapshot.tab("test:holders").unwrap().display_items;
    let levels: Vec<_> = books
        .iter()
        .filter_map(|book| {
            let Tag::Int(level) = book
                .components()?
                .as_nbt()
                .get(&"minecraft:stored_enchantments".into())?
                .get("test:levels")?
            else {
                return None;
            };
            Some(*level)
        })
        .collect();
    let [level] = levels.as_slice() else {
        panic!("expected one highest-level fixture book, got {levels:?}")
    };
    *level
}

#[test]
#[ignore = "daily"]
fn creative_catalog_builds_lazily_and_edits_before_filtering_on_both_sides() {
    use crate::game::native_client_session::tests::logged_client;
    use sunrise_client::creative::Creative;
    let row = |item: &str, visibility: &str| {
        serde_json::json!({
            "kind":"stack", "snbt":format!(r#"{{id:"{item}",count:1}}"#), "visibility":visibility,
        })
    };
    let both = "PARENT_AND_SEARCH_TABS";
    let catalog = serde_json::json!({
        "schema":4, "item_features":{"minecraft:barrier":["test:disabled"]}, "potion_features":{},
        "tabs":[
            {"id":"test:a","title":"test:a","type":"CATEGORY","build":[
                row("minecraft:stone", "PARENT_TAB_ONLY"), row("minecraft:stick", "SEARCH_TAB_ONLY"), row("minecraft:barrier", both)
            ]},
            {"id":"minecraft:hotbar","title":"minecraft:hotbar","type":"HOTBAR","build":[]},
            {"id":"test:b","title":"test:b","type":"CATEGORY","build":[row("minecraft:sand", both)],"operator_build":[row("minecraft:glass", both)]},
            {"id":"test:holders","title":"test:holders","type":"CATEGORY","build":[{"kind":"enchantments","all_levels":false,"visibility":both}]},
            {"id":"minecraft:search","title":"minecraft:search","type":"SEARCH","build":[]},
            {"id":"minecraft:inventory","title":"minecraft:inventory","type":"INVENTORY","build":[]},
        ]
    });
    let seen = Arc::new(Mutex::new(Seen::default()));
    let setups = vec![
        ModEntry::new("first_edit", Box::new(Edit(seen.clone()))),
        ModEntry::new("later_edit", Box::new(Observe)),
    ]
    .into();
    let check = |game: &mut Game<'_>, side| {
        game.creative_screen = Creative::from_json(&catalog).unwrap();
        assert!(!game.creative_screen.has_contents());
        let flags = |game: &mut Game<'_>, ids: [&str; 2]| {
            let flags = ids.map(str::to_owned);
            if let Some(remote) = &mut game.remote {
                remote.features = flags.into();
            } else {
                game.level.enabled_features = flags.into();
            }
        };
        flags(game, ["test:alpha", "test:beta"]);
        assert_eq!(game.stock_sort_order()[0].item.as_ref(), "minecraft:stick");
        assert!(seen.lock().unwrap().events.is_empty());
        {
            // A plain read before any build answers nothing and builds nothing.
            let reader: &dyn mod_api::Host = game;
            assert_eq!(reader.built_creative_tabs(), Ok(None));
            assert!(seen.lock().unwrap().events.is_empty());
        }
        game.build_creative(CreativeTabPermissions::Normal).unwrap();
        let built_events = seen.lock().unwrap().events.len();
        let reader: &dyn mod_api::Host = game;
        let normal = reader.built_creative_tabs().unwrap().unwrap();
        assert_eq!(normal.permissions, CreativeTabPermissions::Normal);
        let cached = ItemsHost::creative_tabs(game, CreativeTabPermissions::Normal).unwrap();
        assert!(Arc::ptr_eq(&normal, &cached));
        assert_eq!(seen.lock().unwrap().events.len(), built_events);
        assert_eq!(game.stock_sort_order()[0].item.as_ref(), "minecraft:torch");
        let a = normal.tab("test:a").unwrap();
        assert_eq!(
            a.display_items,
            [
                stack("minecraft:dirt", 1),
                stack("minecraft:stone", 1),
                stack("minecraft:stick", 1)
            ]
        );
        assert_eq!(
            a.search_items,
            [stack("minecraft:torch", 1), stack("minecraft:stone", 1)]
        );
        flags(game, ["test:beta", "test:alpha"]);
        let host: &mut dyn mod_api::Host = game;
        assert!(!a.contains(host, &stack("minecraft:dirt", 1)));
        assert!(a.contains(host, &stack("minecraft:torch", 8)));
        assert_eq!(
            normal
                .tabs_showing(host, &stack("minecraft:dirt", 1))
                .next()
                .unwrap()
                .id,
            "test:a"
        );
        assert!(
            normal
                .tabs_showing(host, &stack("minecraft:torch", 1))
                .next()
                .is_none()
        );
        assert!(a.contains(host, &stone_key(64)));
        assert_eq!(
            normal.tabs_showing(host, &stone_key(64)).next().unwrap().id,
            "test:a"
        );
        assert!(!a.contains(host, &stone_key(32)));
        assert!(normal.tabs_showing(host, &stone_key(32)).next().is_none());
        let cached = host.creative_tabs(CreativeTabPermissions::Normal).unwrap();
        assert!(Arc::ptr_eq(&normal, &cached));
        let old = seen.lock().unwrap().token.unwrap();
        assert_eq!(
            old.entries(host, CreativeTabList::Search),
            Err(CreativeTabEditError::Stale)
        );
        assert_eq!(
            old.remove(
                host,
                &stack("minecraft:stone", 1),
                CreativeTabVisibility::Both
            ),
            Err(CreativeTabEditError::Stale)
        );
        let operator = host
            .creative_tabs(CreativeTabPermissions::Operator)
            .unwrap();
        assert!(!Arc::ptr_eq(&normal, &operator));
        assert_eq!(
            operator.tab("minecraft:search").unwrap().display_items[..3],
            [
                stack("minecraft:torch", 1),
                stack("minecraft:stone", 1),
                stack("minecraft:glass", 1)
            ]
        );
        assert_eq!(
            normal.tab("test:b").unwrap().display_items,
            [stack("minecraft:sand", 1)]
        );
        let actual = std::mem::take(&mut seen.lock().unwrap().events);
        let expected: Vec<_> = [
            CreativeTabPermissions::Normal,
            CreativeTabPermissions::Operator,
        ]
        .into_iter()
        .flat_map(|permissions| {
            [
                "test:a",
                "test:b",
                "test:holders",
                "minecraft:hotbar",
                "minecraft:search",
                "minecraft:inventory",
            ]
            .map(|tab| (side, permissions, tab.to_owned()))
        })
        .collect();
        assert_eq!(actual, expected);

        assert_eq!(book_level(&normal), 2);
        normal
    };
    let check_replacement = |game: &mut Game<'_>, old: &CreativeModeTabs| {
        // Registry installation must not dispatch; the next demand uses the changed definition.
        assert!(seen.lock().unwrap().events.is_empty());
        let fresh = ItemsHost::creative_tabs(game, CreativeTabPermissions::Normal).unwrap();
        assert_eq!(book_level(&fresh), 4);
        assert_eq!(book_level(old), 2);
        assert_eq!(seen.lock().unwrap().events.len(), 6);
        seen.lock().unwrap().events.clear();
    };
    mod_platform::testing::with_mods(setups, || {
        let mut dedicated = None;
        with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
            install_dedicated_holders(game, 2);
            dedicated = Some(check(game, LogicalSide::Server));
        });
        with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
            install_dedicated_holders(game, 4);
            game.creative_screen = Creative::from_json(&catalog).unwrap();
            check_replacement(game, dedicated.as_deref().unwrap());
        });
        with_client_game(|game| {
            let _server = logged_client(game);
            game.native_lifecycle
                .begin_open(mod_api::Authority::RemoteClient);
            replace_client_definition(game, 2);
            let old = check(game, LogicalSide::Client);
            replace_client_definition(game, 4);
            check_replacement(game, &old);
            game.native_lifecycle.begin_close();
            game.native_lifecycle_world_closed();
            assert!(game.native_lifecycle.epoch().is_none());
            assert!(game.creative_contents.published.is_some());
            let host: &dyn mod_api::Host = game;
            assert_eq!(
                host.built_creative_tabs(),
                Err(CreativeTabReadError::Unavailable)
            );
        });
    });
}

#[test]
fn a_tab_reads_back_its_title_as_a_translatable_with_the_title_as_fallback() {
    let catalog = Catalog::parse(&serde_json::json!({
        "schema":4, "item_features":{}, "potion_features":{},
        "tabs":[
            {"id":"minecraft:building_blocks","title":"Building Blocks","type":"CATEGORY","build":[]},
            {"id":"ruby:gems","title":"Ruby Gems","type":"CATEGORY","build":[]},
        ]
    }))
    .unwrap();
    let lists = [Arc::default(), Arc::default()];
    let tabs = snapshot(&catalog, CreativeTabPermissions::Normal, &lists);
    let name = |tab: &str| match &tabs.tab(tab).unwrap().display_name.content {
        mod_api::text::TextContent::Translatable {
            key,
            fallback,
            arguments,
        } => (key.clone(), fallback.clone(), arguments.is_empty()),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        name("minecraft:building_blocks"),
        (
            "itemGroup.buildingBlocks".to_owned(),
            Some("Building Blocks".to_owned()),
            true
        )
    );
    assert_eq!(
        name("ruby:gems"),
        (
            "itemGroup.ruby.gems".to_owned(),
            Some("Ruby Gems".to_owned()),
            true
        )
    );
}

/// A dedicated server's `creative_tabs` builds vanilla's tabs from the prepared catalog: the
/// building blocks tab starts with oak log, as the client's creative screen shows it.
#[test]
#[ignore = "daily"]
fn a_servers_creative_tabs_hold_vanillas_items() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
            use sunrise_client::save::{Level, write_dat};

            let save = tempfile::tempdir().unwrap();
            write_dat(
                &save.path().join("level.dat"),
                Tag::compound([(
                    "Data",
                    Tag::compound([("DataVersion", Tag::Int(sunrise_core::version::DATA_VERSION))]),
                )]),
            )
            .unwrap();
            game.level = Level::read(save.path()).unwrap();
            install_dedicated_holders(game, 2);
            for permissions in [
                CreativeTabPermissions::Normal,
                CreativeTabPermissions::Operator,
            ] {
                let tabs = ItemsHost::creative_tabs(game, permissions).unwrap();
                let building = tabs.tab("minecraft:building_blocks").unwrap();
                let oak_log = stack("minecraft:oak_log", 1);
                assert_eq!(building.display_items.first(), Some(&oak_log));
                assert!(building.search_items.contains(&oak_log));
                assert!(
                    !tabs
                        .tab("minecraft:natural_blocks")
                        .unwrap()
                        .display_items
                        .is_empty()
                );
                assert!(
                    !tabs
                        .tab("minecraft:search")
                        .unwrap()
                        .display_items
                        .is_empty()
                );
            }
        });
    });
}
