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
fn drawn(bodies: &[CelestialBody], switches: SkySwitches) -> Vec<(u32, CelestialBody)> {
    vanilla_bodies(bodies, bodies.len(), switches).collect()
}

/// The Overworld's two quads: the sun's, then the moon's.
const SUN_AND_MOON: &[CelestialBody] = &[Sun, Moon];

#[test]
fn both_bodies_draw_by_default() {
    assert_eq!(drawn(SUN_AND_MOON, ALL_ON), [(0, Sun), (1, Moon)]);
}

#[test]
fn sun_false_leaves_only_the_moon() {
    let switches = SkySwitches {
        sun: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, switches), [(1, Moon)]);
}

#[test]
fn moon_false_leaves_only_the_sun() {
    let switches = SkySwitches {
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, switches), [(0, Sun)]);
}

#[test]
fn sun_and_moon_false_leave_nothing() {
    let switches = SkySwitches {
        sun: false,
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, switches), []);
}

/// The sky and stars switches don't touch the bodies.
#[test]
fn sky_and_stars_false_keep_both_bodies() {
    let switches = SkySwitches {
        sky: false,
        stars: false,
        ..ALL_ON
    };
    assert_eq!(drawn(SUN_AND_MOON, switches), [(0, Sun), (1, Moon)]);
}

/// The moon alone (the sun's sprite missing) is the first quad, and still the moon: `sun=false`
/// keeps it, `moon=false` leaves it out.
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
    assert_eq!(drawn(&[Moon], ALL_ON), [(0, Moon)]);
    assert_eq!(drawn(&[Moon], sun_off), [(0, Moon)]);
    assert_eq!(drawn(&[Moon], moon_off), []);
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
    assert_eq!(drawn(&[Sun], ALL_ON), [(0, Sun)]);
    assert_eq!(drawn(&[Sun], moon_off), [(0, Sun)]);
    assert_eq!(drawn(&[Sun], sun_off), []);
}

/// The End's flash, its one quad, is neither the sun nor the moon.
#[test]
fn the_end_flash_draws_whatever_sun_and_moon_say() {
    let switches = SkySwitches {
        sun: false,
        moon: false,
        ..ALL_ON
    };
    assert_eq!(drawn(&[EndFlash], switches), [(0, EndFlash)]);
}

/// Only the quads uploaded draw: an entry past them has no quad.
#[test]
fn an_entry_past_the_quads_uploaded_isnt_drawn() {
    let drawn: Vec<_> = vanilla_bodies(SUN_AND_MOON, 1, ALL_ON).collect();
    assert_eq!(drawn, [(0, Sun)]);
}
