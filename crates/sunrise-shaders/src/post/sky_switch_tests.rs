//! `sky`, `stars`, `sun` and `moon` in `shaders.properties` (spec 3.2): on unless `false`.

use super::*;

/// The switches `load_post` reads from `text` as `shaders.properties`.
fn switches(text: &str) -> SkySwitches {
    let zip = crate::pack::tests::zip(&[("shaders/final.fsh", "void main(){}")]);
    let pack = Pack::from_zip_bytes("sky.zip", zip).unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse(text, &macros);
    load_post(&pack, "", &properties, &macros, &GameInputs::default()).sky_switches
}

const ALL_ON: SkySwitches = SkySwitches {
    sky: true,
    stars: true,
    sun: true,
    moon: true,
};

#[test]
fn every_part_draws_without_the_keys() {
    assert_eq!(switches(""), ALL_ON);
}

#[test]
fn true_keeps_every_part() {
    assert_eq!(
        switches("sky=true\nstars=true\nsun=true\nmoon=true\n"),
        ALL_ON
    );
}

#[test]
fn sky_false_leaves_out_only_the_sky() {
    let expected = SkySwitches {
        sky: false,
        ..ALL_ON
    };
    assert_eq!(switches("sky=false\n"), expected);
}

#[test]
fn stars_false_leaves_out_only_the_stars() {
    let expected = SkySwitches {
        stars: false,
        ..ALL_ON
    };
    assert_eq!(switches("stars = false\n"), expected);
}

#[test]
fn sun_false_leaves_out_only_the_sun() {
    let expected = SkySwitches {
        sun: false,
        ..ALL_ON
    };
    assert_eq!(switches("sun=false\n"), expected);
}

#[test]
fn moon_false_leaves_out_only_the_moon() {
    let expected = SkySwitches {
        moon: false,
        ..ALL_ON
    };
    assert_eq!(switches("moon=false\n"), expected);
}

/// The file's conditional directives pick the value (3.1): the branch not taken doesn't count.
#[test]
fn a_conditional_branch_picks_the_value() {
    let text = "#ifdef MC_VERSION\nstars=false\n#else\nsun=false\n#endif\n";
    let expected = SkySwitches {
        stars: false,
        ..ALL_ON
    };
    assert_eq!(switches(text), expected);
}
