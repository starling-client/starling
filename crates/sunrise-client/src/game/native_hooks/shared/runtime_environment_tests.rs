#[test]
#[ignore = "daily"]
fn client_host_reads_current_pause_state() {
    use super::super::native_entity_refs::tests::with_game;
    use crate::game::execution_role::ExecutionRole;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        game.pause = false;
        assert!(!mod_api::prelude::ClientViewClientHost::is_paused(game));
        game.pause = true;
        assert!(mod_api::prelude::ClientViewClientHost::is_paused(game));
    });
}

#[test]
#[ignore = "daily"]
fn client_host_reads_playing_and_key_mappings() {
    use super::super::Screen;
    use super::super::native_entity_refs::tests::with_game;
    use crate::game::execution_role::ExecutionRole;
    use mod_api::prelude::ClientInputClientHost;
    use mod_api::{MappingError, Text};
    use winit::keyboard::KeyCode;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        game.screen = Screen::Playing;
        assert!(mod_api::prelude::ClientInputClientHost::playing(game));
        game.screen = Screen::Chat;
        assert!(!mod_api::prelude::ClientInputClientHost::playing(game));

        // Vanilla's default bindings: forward is W, attack the left mouse button.
        let forward = ClientInputClientHost::vanilla_key_mapping(game, "key.forward").unwrap();
        assert_eq!(forward.is_down(game), Ok(false));
        assert_eq!(forward.translated_key_message(game), Ok(Text::literal("W")));
        game.insert_device_key(KeyCode::KeyW);
        assert_eq!(forward.is_down(game), Ok(true));
        let attack = ClientInputClientHost::vanilla_key_mapping(game, "key.attack").unwrap();
        assert_eq!(
            attack.translated_key_message(game),
            Ok(Text::translate("key.mouse.left", Vec::new()))
        );
        assert_eq!(
            ClientInputClientHost::vanilla_key_mapping(game, "key.mymod.boost"),
            Err(MappingError::Unknown {
                key: "key.mymod.boost".into(),
            }),
        );

        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert_eq!(
            ClientInputClientHost::vanilla_key_mapping(game, "key.forward"),
            Err(MappingError::Unavailable),
        );
        assert_eq!(forward.binding(game), Err(MappingError::Unavailable));
    });
}

#[test]
fn observed_overlay_prefers_first_open_screen() {
    assert_eq!(
        super::observed_overlay_id(&[(false, "sign"), (false, "book")]),
        None
    );
    assert_eq!(
        super::observed_overlay_id(&[(false, "sign"), (true, "book")]),
        Some("book")
    );
    assert_eq!(
        super::observed_overlay_id(&[(true, "sign"), (true, "book")]),
        Some("sign")
    );
}

/// A mod's own screen opens only over the world, never over another
/// screen, an overlay (the title screen, a book, loading, no window) or
/// another mod's; opening it again is a no-op.
#[test]
fn a_mod_screen_opens_only_over_the_world() {
    use super::{Screen, mod_screen_opens};
    assert_eq!(mod_screen_opens(Screen::Playing, false, None, "a"), None);
    assert_eq!(
        mod_screen_opens(Screen::Playing, true, None, "a"),
        Some(false)
    );
    for other in [
        Screen::Paused,
        Screen::Inventory,
        Screen::Chat,
        Screen::Options,
    ] {
        assert_eq!(mod_screen_opens(other, false, None, "a"), Some(false));
    }
    assert_eq!(
        mod_screen_opens(Screen::Mod, false, Some("a"), "a"),
        Some(true)
    );
    assert_eq!(
        mod_screen_opens(Screen::Mod, false, Some("b"), "a"),
        Some(false)
    );
}

/// A mod container opens a slot grid or a furnace's menu, by its slot
/// count; no menu with rules of its own (a brewing stand's, a crafter's).
#[test]
fn mod_menus_are_slot_grids_and_furnaces() {
    use super::mod_menu;
    assert_eq!(mod_menu("minecraft:hopper"), Some(("minecraft:hopper", 5)));
    assert_eq!(mod_menu("minecraft:generic_9x3").map(|m| m.1), Some(27));
    assert_eq!(mod_menu("minecraft:furnace").map(|m| m.1), Some(3));
    assert_eq!(mod_menu("minecraft:brewing_stand"), None);
    assert_eq!(mod_menu("hopper"), None);
}

#[test]
fn block_containers_cannot_bind_an_owned_menu_type() {
    let menu = |id: &str, limits| {
        (
            id.into(),
            "test".into(),
            1,
            0,
            vec![],
            vec![],
            None,
            None,
            true,
            limits,
        )
    };
    sunrise_client::containers::with_mod_menu_types(
        [
            menu("test:block", None),
            menu("test:transient", Some(vec![1])),
        ],
        || {
            assert_eq!(super::mod_menu("test:block"), Some(("test:block", 1)));
            assert_eq!(super::mod_menu("test:transient"), None);
        },
    );
}

/// The config folder is `config/` beside `options.txt`, made when
/// missing.
#[test]
fn config_dir_is_config_in_the_game_folder() {
    use super::config_dir;
    let root = std::env::temp_dir().join(format!("sunrise-config-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    assert!(!root.join("config").exists());
    let dir = config_dir(&root).unwrap();
    assert_eq!(dir, root.join("config"));
    assert!(dir.is_dir());
    assert_eq!(config_dir(&root).unwrap(), dir);
    let _ = std::fs::remove_dir_all(&root);
}

/// Vanilla's F3 draws a column in each top corner (`extractLines`, left
/// and right aligned), and its charts along the bottom.
#[test]
fn the_debug_screen_takes_both_top_corners_and_its_charts_corners() {
    use super::{Corner, debug_takes};
    for corner in Corner::ALL {
        assert!(!debug_takes(corner, false, [true, true]), "{corner:?}");
    }
    assert!(debug_takes(Corner::TopLeft, true, [false, false]));
    assert!(debug_takes(Corner::TopRight, true, [false, false]));
    assert!(!debug_takes(Corner::BottomLeft, true, [false, false]));
    assert!(!debug_takes(Corner::BottomRight, true, [false, false]));
    assert!(debug_takes(Corner::BottomLeft, true, [true, false]));
    assert!(!debug_takes(Corner::BottomRight, true, [true, false]));
    assert!(debug_takes(Corner::BottomRight, true, [false, true]));
}
