//! The sun and moon the game draws itself under a shader pack's `sun` and `moon` (spec 3.2).

use super::vanilla_bodies;
use crate::gfx::CelestialBody::{self, EndFlash, Moon, Sun};
use sunrise_shaders::SkySwitches;

const ALL_ON: SkySwitches = SkySwitches {
    sky: true,
    stars: true,
    sun: true,
    moon: true,
};

/// The quads drawn, by their place, and what each is, with a quad uploaded for each of `bodies`.
fn drawn(
    bodies: &[CelestialBody],
    own_sun: bool,
    switches: SkySwitches,
) -> Vec<(u32, CelestialBody)> {
    vanilla_bodies(bodies, bodies.len(), own_sun, switches).collect()
}

/// The Overworld's two quads: the sun's, then the moon's.
const SUN_AND_MOON: &[CelestialBody] = &[Sun, Moon];

#[test]
fn both_bodies_draw_by_default() {
    assert_eq!(drawn(SUN_AND_MOON, false, ALL_ON), [(0, Sun), (1, Moon)]);
}

#[test]
fn sun_false_leaves_only_the_moon() {
    let switches = SkySwitches {
        sun: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, false, switches), [(1, Moon)]);
}

#[test]
fn moon_false_leaves_only_the_sun() {
    let switches = SkySwitches {
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, false, switches), [(0, Sun)]);
}

#[test]
fn sun_and_moon_false_leave_nothing() {
    let switches = SkySwitches {
        sun: false,
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, false, switches), []);
}

/// The sky and stars switches don't touch the bodies.
#[test]
fn sky_and_stars_false_keep_both_bodies() {
    let switches = SkySwitches {
        sky: false,
        stars: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, false, switches), [(0, Sun), (1, Moon)]);
}

/// A pack sky drawing its own sun skips vanilla's, as before the switches.
#[test]
fn an_own_sun_still_skips_the_sun() {
    assert_eq!(drawn(SUN_AND_MOON, true, ALL_ON), [(1, Moon)]);
}

/// The moon alone (the sun's sprite missing) is the first quad, and still the moon: `sun=false`
/// keeps it, `moon=false` leaves it out, and an own sun doesn't skip it.
#[test]
fn the_moon_alone_follows_only_moon() {
    let sun_off = SkySwitches {
        sun: false,
        ..ALL_ON
    };
    let moon_off = SkySwitches {
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(&[Moon], false, ALL_ON), [(0, Moon)]);
    assert_eq!(drawn(&[Moon], false, sun_off), [(0, Moon)]);
    assert_eq!(drawn(&[Moon], false, moon_off), []);
    assert_eq!(drawn(&[Moon], true, ALL_ON), [(0, Moon)]);
}

/// The sun alone (the moon's sprite missing): `moon=false` keeps it, `sun=false` leaves it out.
#[test]
fn the_sun_alone_follows_only_sun() {
    let sun_off = SkySwitches {
        sun: false,
        ..ALL_ON
    };
    let moon_off = SkySwitches {
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(&[Sun], false, ALL_ON), [(0, Sun)]);
    assert_eq!(drawn(&[Sun], false, moon_off), [(0, Sun)]);
    assert_eq!(drawn(&[Sun], false, sun_off), []);
}

/// The End's flash, its one quad, is neither the sun nor the moon.
#[test]
fn the_end_flash_draws_whatever_sun_and_moon_say() {
    let switches = SkySwitches {
        sun: false,
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(&[EndFlash], true, switches), [(0, EndFlash)]);
}

/// Only the quads uploaded draw: an entry past them has no quad.
#[test]
fn an_entry_past_the_quads_uploaded_isnt_drawn() {
    let drawn: Vec<_> = vanilla_bodies(SUN_AND_MOON, 1, false, ALL_ON).collect();
    assert_eq!(drawn, [(0, Sun)]);
}
