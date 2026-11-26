//! Starling's part of chat_tests.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
pub(super) fn enter_completes_a_failing_command_word_only_when_one_completion_fits() {
    use winit::keyboard::KeyCode::Enter;

    let font = font();
    let m = Measure(&font);
    let registries = Registries {
        permission: 4,
        items: [
            "minecraft:diamond",
            "minecraft:diamond_sword",
            "minecraft:dirt",
            "minecraft:totem_of_undying",
        ]
        .map(String::from)
        .to_vec(),
        ..Registries::default()
    };
    let c = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    let sent = |typed: &str| {
        let mut screen = ChatScreen::new(&m, &c, "", false, 0, 640, 360, full());
        screen.type_text(&m, &c, typed);
        screen.key(&m, &c, &[], Enter, false, false)
    };
    for (typed, completed) in [
        ("/gamemode c", "/gamemode creative"),
        ("/gamemode su", "/gamemode survival"),
        // Brigadier's completions take the word whatever its case.
        ("/Gamemode c", "/gamemode creative"),
        ("/gamemode Creative", "/gamemode creative"),
        ("/gamemode c @s", "/gamemode creative @s"),
        ("/give @p totem", "/give @p minecraft:totem_of_undying"),
    ] {
        assert_eq!(
            sent(typed),
            ChatAction::Submit(completed.to_owned()),
            "{typed}"
        );
        assert_eq!(complete_command(&c, typed).as_deref(), Some(completed));
    }
    for typed in [
        // Two completions: survival and spectator; diamond and diamond_sword.
        "/gamemode s",
        "/give @p dia",
        // A completion that matches inside the id, not from the word's start.
        "/give @p undying",
        // Valid as typed.
        "/gamemode creative",
        // One completion, but the command still wants its argument.
        "/gamemod",
        // Free text and chat lines.
        "/say hel",
        "/msg @p hel",
        "hello wor",
    ] {
        assert_eq!(sent(typed), ChatAction::Submit(typed.to_owned()), "{typed}");
        assert_eq!(complete_command(&c, typed), None, "{typed}");
    }
}

#[test]
pub(super) fn enter_leaves_a_word_the_server_completes() {
    // An `ask_server` node: its completions would need the server's reply.
    let registries = Registries {
        permission: 4,
        ..Registries::default()
    };
    let mut d = crate::command::vanilla::game_dispatcher().clone();
    let child = |d: &crate::command::Dispatcher, n: usize, name: &str| {
        d.nodes[n]
            .children
            .iter()
            .copied()
            .find(|&c| d.nodes[c].name() == name)
            .expect(name)
    };
    let gamemode = child(&d, child(&d, 0, "gamemode"), "gamemode");
    d.nodes[gamemode].suggests = Some(crate::command::Suggests::AskServer);
    let c = Commands {
        dispatcher: &d,
        registries: &registries,
    };
    assert_eq!(complete_command(&c, "/gamemode c"), None);
    // Without the server's provider the same word completes.
    let local = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    assert_eq!(
        complete_command(&local, "/gamemode c").as_deref(),
        Some("/gamemode creative")
    );
}

#[test]
pub(super) fn the_keyboard_editor_sets_the_whole_input_and_its_selection() {
    let font = font();
    let m = Measure(&font);
    let registries = Registries::default();
    let c = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    let mut screen = ChatScreen::new(&m, &c, "draft", false, 0, 640, 360, full());
    // Refused characters go, and the positions follow the text kept.
    screen.set_text(&m, &c, "a\u{a7}b\ncd", 5, 2);
    assert_eq!(screen.value(), "abcd");
    assert_eq!((screen.input.cursor, screen.input.highlight), (3, 1));
    // The maximum length holds.
    screen.set_text(&m, &c, &"x".repeat(300), 300, 300);
    assert_eq!(screen.value().chars().count(), 256);
    assert_eq!(screen.input.cursor, 256);
}

/// The text the box's suggestion list offers, in the order it shows them.
pub(super) fn offered(screen: &ChatScreen) -> Vec<String> {
    screen
        .suggestions
        .as_ref()
        .map(|list| list.list.iter().map(|s| s.text.clone()).collect())
        .unwrap_or_default()
}

/// Typing a command completes it, as the on-screen keyboard's whole-text edit does (`set_text`):
/// `/gamemode ` offers the modes and `/sa` the commands that start so, for a player who may use
/// them (permission 2, single player with cheats on).
#[test]
pub(super) fn the_chat_completes_commands_for_a_game_master() {
    let font = font();
    let m = Measure(&font);
    let registries = Registries {
        permission: 2,
        ..Registries::default()
    };
    let c = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    let mut screen = ChatScreen::new(&m, &c, "", false, 0, 640, 360, full());
    screen.set_text(&m, &c, "/gamemode ", 10, 10);
    let mut modes = offered(&screen);
    modes.sort();
    assert_eq!(modes, ["adventure", "creative", "spectator", "survival"]);
    screen.set_text(&m, &c, "/sa", 3, 3);
    assert!(offered(&screen).iter().any(|s| s == "say"));
    // Without the permission the game masters' commands are not offered.
    let player = Registries::default();
    let c = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &player,
    };
    screen.set_text(&m, &c, "/gamemode", 9, 9);
    assert!(offered(&screen).is_empty());
}

/// The build and the draw both `resize` the chat, each to the size it lays out in. A resize to
/// the size it has keeps the suggestions (as at each frame of a settled screen); a resize to
/// another one makes the box again, with none. The two must therefore agree on the size, or the
/// chat is made again at each frame and never shows a suggestion (a camera cutout's strips make
/// the draw's width narrower than the frame's: `Game::width_between_cutouts`).
#[test]
pub(super) fn a_resize_to_the_chats_own_size_keeps_its_suggestions() {
    let font = font();
    let m = Measure(&font);
    let registries = Registries {
        permission: 2,
        ..Registries::default()
    };
    let c = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    let mut screen = ChatScreen::new(&m, &c, "", false, 0, 600, 360, full());
    screen.set_text(&m, &c, "/gamemode ", 10, 10);
    assert!(screen.suggestions_visible());
    screen.resize(&m, &c, 600, 360);
    assert!(
        screen.suggestions_visible(),
        "the same size changes nothing"
    );
    screen.resize(&m, &c, 640, 360);
    assert!(
        !screen.suggestions_visible(),
        "another size makes the box again"
    );
    assert_eq!(screen.value(), "/gamemode ", "with the text it held");
    // The next edit completes again.
    screen.set_text(&m, &c, "/gamemode s", 11, 11);
    assert!(screen.suggestions_visible());
}
