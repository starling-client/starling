//! Title screen buttons (docs/layers.md, "Platform registries"): a crate above the engine adds a
//! full-width button to the title screen without an engine file naming it. Each shown button
//! takes a row of its own below Minecraft Realms, as NeoForge's Mods does; one shows only while
//! it says so, so with none shown the title screen is vanilla's. A button names the engine
//! screen its row opens in [`Opens`]; the engine routes a press by the button's registration,
//! so the row the player selected is the row taken.
use crate::registry::{Beyond, Registry};

/// The main-menu id of the first registered button; the engine gives each further registration
/// the next id. A press carries the id of the row it hit, so it resolves the same registration
/// however the buttons' state has moved in between.
pub const FIRST_ID: u8 = 200;

/// The most buttons the registry accepts: [`FIRST_ID`] and the ids after it are the `u8`s a
/// main-menu button id holds.
pub const MAX_BUTTONS: usize = u8::MAX as usize - FIRST_ID as usize + 1;

/// What the title screen knows when it asks a button whether it shows.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Title {
    /// The packages in the mods folder, readable or not: the rows of the Mods screen's list.
    pub mods: usize,
}

impl Title {
    /// The title screen's facts: `mods` packages in the mods folder.
    pub fn new(mods: usize) -> Self {
        Self { mods }
    }
}

/// The engine screen a button opens. A new destination is an addition.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opens {
    /// The Mods screen: the installed mods, as NeoForge lists them.
    Mods,
}

/// One button.
pub trait Button: Sync {
    /// Its label: a translation key.
    fn label(&self) -> &'static str;
    /// Whether the title screen shows it now. Asked each time the screen lays its buttons out.
    fn shown(&self, title: &Title) -> bool;
    /// The screen pressing it opens.
    fn opens(&self) -> Opens;
}

/// Why [`register`] refused a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refused {
    /// The game sealed the list: buttons register before it runs, as [`crate::TooLate`].
    TooLate,
    /// [`MAX_BUTTONS`] registrations hold every id from [`FIRST_ID`] on.
    Full,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLate => "the game already started: title buttons register before it runs",
            Self::Full => "the title screen has no main-menu id left for the button",
        })
    }
}

impl std::error::Error for Refused {}

static BUTTONS: Registry<dyn Button> = Registry::new();

/// Adds `button` to the title screen. Before the game starts, as [`crate::register`]; the rows
/// keep the order buttons registered in, and only [`MAX_BUTTONS`] of them fit the main menu's
/// ids.
pub fn register(button: &'static dyn Button) -> Result<(), Refused> {
    register_in(&BUTTONS, button)
}

fn register_in(buttons: &Registry<dyn Button>, button: &'static dyn Button) -> Result<(), Refused> {
    match buttons.register_max(button, MAX_BUTTONS) {
        Ok(()) => Ok(()),
        Err(Beyond::TooLate) => Err(Refused::TooLate),
        Err(Beyond::Full) => Err(Refused::Full),
    }
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::*;

    /// Every registered button, sealing the list on the first call.
    pub fn all() -> &'static [&'static dyn Button] {
        BUTTONS.all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Row;

    impl Button for Row {
        fn label(&self) -> &'static str {
            "test.title.row"
        }
        fn shown(&self, _: &Title) -> bool {
            true
        }
        fn opens(&self) -> Opens {
            Opens::Mods
        }
    }

    #[test]
    fn the_registry_refuses_a_button_the_main_menu_has_no_id_for() {
        // A registry of the test's own: the game's list stays unsealed for other tests.
        static BUTTONS: Registry<dyn Button> = Registry::new();
        static ROWS: [Row; MAX_BUTTONS] = [Row; MAX_BUTTONS];
        for row in &ROWS {
            assert_eq!(register_in(&BUTTONS, row), Ok(()));
        }
        // The list is not sealed yet: a full registry is not a late one.
        assert_eq!(register_in(&BUTTONS, &Row), Err(Refused::Full));
        assert_eq!(
            register_in(&BUTTONS, &Row).unwrap_err().to_string(),
            "the title screen has no main-menu id left for the button"
        );
        // Reading the list seals it; a further registration is late, not full.
        assert_eq!(BUTTONS.all().len(), MAX_BUTTONS);
        assert_eq!(register_in(&BUTTONS, &Row), Err(Refused::TooLate));
    }
}
