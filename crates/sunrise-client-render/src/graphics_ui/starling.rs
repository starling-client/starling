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

#[cfg(test)]
mod present_mode_tests {
    use super::super::*;

    /// Mailbox and Immediate only run with VSync off, so choosing one turns VSync off, Fifo
    /// turns it on, and VSync turned on puts a Mailbox or Immediate choice back to Auto.
    #[test]
    fn present_mode_and_vsync_follow_each_other() {
        use crate::graphics::PresentMode as P;
        let none = EffectiveOutput::NONE;
        let mut s = Settings::default();
        s.vsync = true;
        s.sunrise.present_mode = P::Fifo;
        Item::Sunrise(H::PresentMode).cycle(&mut s, false, [854, 480], &none);
        assert_eq!((s.sunrise.present_mode, s.vsync), (P::Mailbox, false));
        Item::Sunrise(H::PresentMode).cycle(&mut s, true, [854, 480], &none);
        assert_eq!((s.sunrise.present_mode, s.vsync), (P::Fifo, true));
        s.sunrise.present_mode = P::Immediate;
        s.vsync = false;
        Item::Vanilla(Opt::Vsync).cycle(&mut s, false, [854, 480], &none);
        assert_eq!((s.sunrise.present_mode, s.vsync), (P::Auto, true));
    }
}
