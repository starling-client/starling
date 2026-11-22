//! The title screen's Mods button (`sunrise_hooks::title`): a row of its own below Minecraft
//! Realms, as NeoForge's, while the mods folder holds a mod. Without one the title screen is
//! vanilla's.
use sunrise_hooks::title::{Button, Opens, Title};

pub(crate) static MODS: Mods = Mods;

/// The button's label, in this crate's `assets/lang`: the crate that supplies the row supplies
/// its text (docs/layers.md, "Lang files"), registered with the button before the game starts.
pub(crate) static LANG: &sunrise_hooks::lang::Files =
    include!(concat!(env!("OUT_DIR"), "/locales.rs"));

pub(crate) struct Mods;

impl Button for Mods {
    fn label(&self) -> &'static str {
        "sunrise.mods.screen.title"
    }
    fn shown(&self, title: &Title) -> bool {
        title.mods > 0
    }
    fn opens(&self) -> Opens {
        Opens::Mods
    }
}
