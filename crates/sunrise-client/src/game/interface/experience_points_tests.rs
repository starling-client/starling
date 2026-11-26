//! `Player.giveExperiencePoints` with a negative amount, through the game's own walk:
//! the loss goes down a level at a time through `Player.giveExperienceLevels(-1)`.
use crate::game::{
    Screen,
    game_tests::tests::server_game,
    native_sound_refs::tests::{guest, packets},
    viewed_worlds::tests::client,
};
use sunrise_client::{entities::Experience, motion::PlayerMotion};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// The death screen's score arrives through player metadata, not the XP total packet.
#[test]
#[ignore = "daily"]
fn the_death_screen_receives_the_signed_player_score() {
    let mut resources = None;
    let mut game = client(&mut resources);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    let (mut guest, received) = guest(7, 42, [0.0; 3]);
    guest.score = -1;
    guest.sync_score();
    let mut sent = packets(&received);
    assert_eq!(sent.len(), 1);
    game.handle_packet(sent.pop().unwrap()).unwrap();
    game.handle_packet(C::SetExperience(play::SetExperience {
        level: 0,
        progress: 0.0,
        total: 100,
    }))
    .unwrap();
    game.handle_packet(C::SetHealth(play::SetHealth {
        health: 0.0,
        food: 20,
        saturation: 5.0,
    }))
    .unwrap();
    assert_eq!(
        (game.screen, game.player_score, game.experience.total),
        (Screen::Dead, -1, 100)
    );
}

/// A point lost at level 0 leaves nothing of the 100: `giveExperienceLevels(-1)` zeroes the
/// total when the level would drop below 0.
#[test]
#[ignore = "daily"]
fn a_point_lost_at_level_zero_zeroes_the_total() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.experience = Experience {
        level: 0,
        progress: 0.0,
        total: 100,
    };
    game.give_experience_points(-1);
    let xp = &game.experience;
    assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
}

/// The mirror: above level 0 the same loss only takes the point given and walks the progress
/// back a level (level 5 needs 17, so it lands on level 4).
#[test]
#[ignore = "daily"]
fn a_point_lost_above_level_zero_only_takes_that_point() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.experience = Experience {
        level: 5,
        progress: 0.0,
        total: 100,
    };
    game.give_experience_points(-1);
    let xp = &game.experience;
    assert_eq!(xp.total, 99);
    assert_eq!(xp.level, 4);
    assert!(
        (xp.progress - (1.0 - 1.0 / 15.0)).abs() < 1e-6,
        "{}",
        xp.progress
    );
}
