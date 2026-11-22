//! The "Mod audio streams" switch (docs/extras.md): on by default, off the audio mods make
//! themselves plays on unheard. Its setting and its row in the Extras page's Sound group; the
//! engine only owns the gain it sets (`sunrise_platform::sound`).
use std::sync::atomic::{AtomicBool, Ordering};

use sunrise_platform::pages::{Act, Label, Page, Place, Row};
use sunrise_platform::settings::Store;
use sunrise_platform::sound::set_mod_stream_gain;

static ON: AtomicBool = AtomicBool::new(true);

/// The switch's line in `sunrise-extras.txt`, written only while it is off.
const KEY: &str = "modAudioStreams";
const LABEL: &str = "sunrise.extras.mod_audio_streams";
const ABOUT: &str = "sunrise.extras.mod_audio_streams.tooltip";

fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
    set_mod_stream_gain(if on { 1.0 } else { 0.0 });
}

/// The switch's line of `sunrise-extras.txt`, for `sunrise_platform::settings::register`.
pub static SETTING: Setting = Setting;

pub struct Setting;

impl Store for Setting {
    fn file(&self) -> &'static str {
        "sunrise-extras.txt"
    }
    fn owns(&self, key: &str) -> bool {
        key == KEY
    }
    fn load(&self, _key: &str, value: &str) {
        match value {
            "true" => set(true),
            "false" => set(false),
            _ => {}
        }
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        if !ON.load(Ordering::Relaxed) {
            line(KEY, "false");
        }
    }
    fn is_default(&self) -> bool {
        ON.load(Ordering::Relaxed)
    }
}

/// The switch's row in the Extras page's Sound group, for `sunrise_platform::pages::register`.
pub static SECTION: Section = Section;

pub struct Section;

impl Page for Section {
    fn id(&self) -> &'static str {
        "sunrise-official.mod-audio"
    }
    fn title(&self) -> &'static str {
        "sunrise.extras.header.sound"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::toggle(LABEL, ON.load(Ordering::Relaxed)));
    }
    fn act(&self, row: usize, act: Act) {
        if row == 0 && matches!(act, Act::Press | Act::PressBack) {
            set(!ON.load(Ordering::Relaxed));
        }
    }
    fn tooltip(&self, _row: usize) -> Option<Label> {
        Some(Label::Key(ABOUT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only test that touches the statics: on until turned off, the line is written only
    /// while off, and the engine's gain follows the switch.
    #[test]
    fn the_switch_sets_the_engines_gain_and_writes_its_line_only_while_off() {
        let lines = || {
            let mut lines = Vec::new();
            SETTING.save(&mut |key, value| lines.push(format!("{key}:{value}")));
            lines
        };
        assert!(SETTING.is_default() && lines().is_empty());
        SECTION.act(0, Act::Press);
        assert_eq!(lines(), ["modAudioStreams:false"]);
        assert!(!SETTING.is_default());
        SETTING.load(KEY, "true");
        assert!(SETTING.is_default() && lines().is_empty());
    }
}
