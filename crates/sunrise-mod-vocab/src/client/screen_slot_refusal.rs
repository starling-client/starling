//! Refusals when a client mod moves, activates, or hides a slot of an open mod screen.

/// Why [`set_screen_slot`](crate::prelude::ScreensClientHost::set_screen_slot) changed
/// nothing. Every refusal leaves the screen's slots as they were.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ScreenSlotRefusal {
    /// No mod screen is open, or the open one is for another menu type.
    NoScreen,
    /// The slot is outside the menu's container backing.
    NoSuchSlot,
    /// A position that is not finite.
    BadPosition,
    /// Asked with no game backend: screens are the client's.
    Unavailable,
}

impl std::fmt::Display for ScreenSlotRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoScreen => "no mod screen is open for that menu type",
            Self::NoSuchSlot => "the screen declared no such slot",
            Self::BadPosition => "the position is not finite",
            Self::Unavailable => "screens are on the client",
        })
    }
}

impl std::error::Error for ScreenSlotRefusal {}

impl crate::NoBackend for ScreenSlotRefusal {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
