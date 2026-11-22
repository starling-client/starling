//! F3's lightmap chart (F3+4, `DebugScreenOverlay`'s lightmap texture) and the renderer's
//! readback of a pack-drawn light texture: the game asks for the readback exactly while the
//! chart is on screen (`Game::debug_shows_lightmap`, given to `Renderer::set_lightmap_readback`
//! each frame).
use crate::game::*;
use sunrise_client::debug_keys::KeyAction;

/// F3+4 pressed: F3 held, 4 pressed and released, F3 released.
fn f3_and_4(game: &mut Game<'_>) {
    game.debug_key(KeyCode::F3, KeyAction::Press);
    game.debug_key(KeyCode::Digit4, KeyAction::Press);
    game.debug_key(KeyCode::Digit4, KeyAction::Release);
    game.debug_key(KeyCode::F3, KeyAction::Release);
}

#[test]
#[ignore = "daily"]
fn the_lightmap_readback_is_asked_for_only_while_f3_charts_the_lightmap() {
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    game.screen = Screen::Playing;
    assert!(!game.debug_shows_lightmap(), "F3 closed");
    f3_and_4(&mut game);
    assert!(game.debug && game.debug_charts.lightmap);
    assert!(game.debug_shows_lightmap(), "the chart is on screen");
    // F1 hides the overlay, chart and all, with no screen open.
    game.ui.hide_gui = true;
    assert!(!game.debug_shows_lightmap(), "hidden by F1");
    game.ui.hide_gui = false;
    // F3 alone closes the overlay; the chart's switch stays, and comes back with it.
    game.debug_key(KeyCode::F3, KeyAction::Press);
    game.debug_key(KeyCode::F3, KeyAction::Release);
    assert!(!game.debug);
    assert!(!game.debug_shows_lightmap(), "F3 closed again");
    // An always-on entry keeps the overlay drawn with F3 closed: no charts come with it.
    game.debug_entries
        .set_status("fps", sunrise_client::debug_entries::Status::AlwaysOn);
    assert!(game.debug_overlay_visible());
    assert!(!game.debug_shows_lightmap(), "only always-on lines");
    game.debug_key(KeyCode::F3, KeyAction::Press);
    game.debug_key(KeyCode::F3, KeyAction::Release);
    assert!(
        game.debug_shows_lightmap(),
        "F3 open again, still charting it"
    );
    // F3+4 again turns the chart off with the overlay left open.
    f3_and_4(&mut game);
    assert!(!game.debug_charts.lightmap);
    assert!(!game.debug_shows_lightmap(), "another chart, or none");
}
