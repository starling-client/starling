//! `sleepers_needed`: a mod sets how many players must sleep to skip the night.
use mod_api::{Hooks, Host, ModEntry, SleepersNeededEvent};
use std::sync::{Arc, Mutex};

/// Answers `self.0` sleepers, and records what each call saw.
struct Need(i64, Arc<Mutex<Vec<(String, i32, i64, i64)>>>);
#[mod_api::__subscriptions]
impl Hooks for Need {
    fn sleepers_needed(&self, _: &dyn Host, event: &mut SleepersNeededEvent<'_>) {
        self.1.lock().unwrap().push((
            event.dimension.to_owned(),
            event.percentage,
            event.active,
            event.needed,
        ));
        event.needed = self.0;
    }
}

#[test]
#[ignore = "daily"]
fn a_mods_count_decides_the_skip_and_the_announcement() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        assert!(game.ui.rules.set(
            "players_sleeping_percentage",
            sunrise_client_core::gamerules::Value::Int(100)
        ));
        let clock = game.level_clock_name(game.dim);
        super::sleep_skip_night_tests::deep_asleep(game);
        // Guests 12 and 13 count and only 12 is in bed: vanilla needs both.
        assert!(!game.enough_slept());

        let heard = Arc::new(Mutex::new(Vec::new()));
        let one: mod_api::Mods =
            vec![ModEntry::new("one", Box::new(Need(1, heard.clone())))].into();
        mod_platform::testing::with_mods(one, || {
            assert!(game.enough_slept());
            game.announce_sleep_status();
            if game.options.dedicated.is_none() {
                assert_eq!(
                    game.titles.overlay.as_ref().unwrap().translation_key(),
                    Some("sleep.skipping_night")
                );
            }
            game.tick_sleep();
        });
        let level = game.dim.id().to_owned();
        assert_eq!(heard.lock().unwrap().first(), Some(&(level, 100, 2, 2)));
        assert_eq!(game.command_clock_time(&clock), 24_000);
        assert!(game.lan.as_ref().unwrap().hub.guests[0].bed.is_none());

        // An answer below 1 counts as 1: with nobody in bed, the night stays.
        let zero: mod_api::Mods = vec![ModEntry::new("zero", Box::new(Need(0, heard)))].into();
        mod_platform::testing::with_mods(zero, || assert!(!game.enough_slept()));
    });
}
