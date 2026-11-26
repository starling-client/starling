//! `sleep_status_announce`: a mod cancels or replaces the action-bar sleeping line.
use crate::game::Game;
use mod_api::{Hooks, ModEntry, ServerHost, SleepStatusAnnounceEvent};

/// Cancels the line.
struct Mute;
#[mod_api::__subscriptions]
impl Hooks for Mute {
    fn sleep_status_announce(
        &self,
        _: &mut dyn ServerHost,
        _: &mut SleepStatusAnnounceEvent<'_>,
    ) -> mod_api::Flow {
        mod_api::Flow::Cancel
    }
}

/// Replaces the line with its own.
struct Reword;
#[mod_api::__subscriptions]
impl Hooks for Reword {
    fn sleep_status_announce(
        &self,
        _: &mut dyn ServerHost,
        event: &mut SleepStatusAnnounceEvent<'_>,
    ) -> mod_api::Flow {
        *event.line = mod_api::Text::literal("Sweet dreams");
        mod_api::Flow::Continue
    }
}

/// Guest 12 in bed with a one percent rule: one of the two players sleeps and one is needed.
fn one_sleeper(game: &mut Game<'_>) {
    assert!(game.ui.rules.set(
        "players_sleeping_percentage",
        sunrise_client_core::gamerules::Value::Int(1)
    ));
    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.bed = Some([5, 64, 5]);
    });
}

#[test]
#[ignore = "daily"]
fn a_canceling_listener_sends_no_overlay_line() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        one_sleeper(game);
        let mods: mod_api::Mods = vec![ModEntry::new("mute", Box::new(Mute))].into();
        mod_platform::testing::with_mods(mods, || game.announce_sleep_status());
        assert!(game.titles.overlay.is_none());

        // The same change announced with no mod canceling shows vanilla's line.
        game.sleep_seen = (0, 0);
        game.announce_sleep_status();
        assert_eq!(
            game.titles.overlay.as_ref().unwrap().plain(),
            "Sleeping through this night"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_replaced_line_is_what_players_receive() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        one_sleeper(game);
        let mods: mod_api::Mods = vec![ModEntry::new("reword", Box::new(Reword))].into();
        mod_platform::testing::with_mods(mods, || game.announce_sleep_status());
        assert_eq!(
            game.titles.overlay.as_ref().unwrap().plain(),
            "Sweet dreams"
        );
    });
}
