use super::*;
use crate::game::{
    Audio, Game, Options, Progress, Screen, Settings, execution_role::ExecutionRole,
    load_resources, native_entity_refs::tests::with_game,
};
use sunrise_protocol::connection::{Connection, Pipe};

/// `ChatComponent.addClientSystemMessage`: the line joins this client's
/// chat history and no packet leaves for the server.
#[test]
#[ignore = "daily"]
fn a_mod_chat_line_shows_locally_and_sends_nothing() {
    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, server) = Pipe::pair();
        game.connect_local(client, "ChatLineTest").unwrap();
        // Whatever connecting wrote is not this line's; drain it until the
        // client waits for the login answer, so the check below is about
        // `chat_line` alone.
        let (mut reader, _writer) = Connection::local(server).split();
        reader
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        while reader.recv().is_ok() {}
        mod_api::prelude::NetworkClientHost::chat_line(game, mod_api::Text::literal("local only"))
            .unwrap();
        let history: Vec<_> = game
            .remote
            .as_ref()
            .unwrap()
            .chat
            .iter()
            .map(|(line, _)| line.clone())
            .collect();
        assert_eq!(history, ["local only"]);
        assert!(
            reader.recv().unwrap_err().is_timed_out(),
            "a local chat line sends nothing to the server"
        );
    });
}

#[test]
fn typed_chat_is_sent_as_the_client_chat_event_leaves_it() {
    assert_eq!(chat_after_event("hello", |_, _| {}), Some("hello".into()));
    assert_eq!(
        chat_after_event("hello", |m, _| m.push_str(" world")),
        Some("hello world".into())
    );
    assert_eq!(chat_after_event("hello", |_, c| *c = true), None);
    assert_eq!(chat_after_event("hello", |m, _| m.clear()), None);
}

/// A mod's pre-filled chat opens only over the world, with its text
/// typed in and nothing sent.
#[test]
#[ignore = "daily"]
fn mod_open_chat_fills_the_box_only_over_the_world() {
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
    let mut game = Game::new(
        &loaded.resources,
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

    // No player: the title screen.
    game.player = None;
    assert!(game.open_chat("/tp ").is_err());
    assert!(game.ui.chat_screen.is_none());
    game.player = Some(
        sunrise_client::motion::PlayerMotion::new(game.motion, sunrise_core::geometry::Vec3::ZERO)
            .unwrap(),
    );
    game.screen = Screen::Paused;
    assert!(game.open_chat("/tp ").is_err());
    assert!(game.ui.chat_screen.is_none());
    game.screen = Screen::Playing;
    assert!(game.open_chat("/tp ").is_ok());
    assert!(game.screen == Screen::Chat);
    assert_eq!(game.ui.chat_screen.as_ref().unwrap().input.value, "/tp ");
    // The chat is a screen too: a second call doesn't replace what's typed.
    assert!(game.open_chat("hello").is_err());
    assert_eq!(game.ui.chat_screen.as_ref().unwrap().input.value, "/tp ");
}
