//! Starling's part of settings.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// What a button of the player's own on the touch controls does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomAction {
    /// Holds one of the game's controls (`key.sprint`) while a finger is on it.
    Key(String),
    /// Holds a key of the keyboard (`key.keyboard.f1`) while a finger is on it.
    Input(String),
    /// Sends a command or a chat line when pressed.
    Send(String),
}

/// A button of the player's own on the touch controls (its place is the layout's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomButton {
    pub id: u32,
    pub action: CustomAction,
    /// One of [`CUSTOM_ICONS`]; none shows what it does in words.
    pub icon: Option<String>,
    /// Wears the player's own image instead (`own_buttons`).
    pub image: bool,
    /// Wears another image of the player's own while pressed (`own_buttons::pressed_slot`).
    pub pressed: bool,
    /// A press turns it on and the next one off; otherwise it is on while held.
    pub toggle: bool,
}

/// What a custom button can wear: `starling:touch/custom_<name>` and its `_pressed`
/// (packaging/touch/make.py draws them).
pub const CUSTOM_ICONS: [&str; 15] = [
    "star", "heart", "shield", "skull", "home", "crown", "flag", "plus", "check", "cross",
    "potion", "pickaxe", "apple", "fire", "music",
];

impl CustomButton {
    /// What it shows on it.
    pub fn label(&self) -> String {
        match &self.action {
            CustomAction::Key(name) => crate::lang::get(name),
            CustomAction::Input(name) => crate::keybinds::display_name(name).plain(),
            CustomAction::Send(line) => line.clone(),
        }
    }
}
