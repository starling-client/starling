//! Starling's part of graphics_ui.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

pub(super) fn display_setting((option, text): (DisplayOption, &'static str)) -> Setting {
    Setting {
        item: Item::Display(option),
        cost: Cost::None,
        text,
    }
}

/// What the Calibration group says above Calibrate...
pub(super) const CALIBRATE_NOTE: &str = "Picks the fastest drawing for this phone.";

/// The Performance page's Calibration group, on Android: what it does, then Calibrate...
pub(super) fn calibration_group() -> (&'static str, Vec<Setting>) {
    (
        "Calibration",
        vec![
            h(H::PerfCalibrateNote, Cost::None, CALIBRATE_NOTE),
            h(
                H::PerfCalibrate,
                Cost::None,
                "Tries the ways of drawing that look the same, in a test world, and keeps the fastest.",
            ),
        ],
    )
}
