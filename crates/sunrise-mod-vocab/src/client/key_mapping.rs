//! Vanilla mapping handles and the keys a native mod registers on Options > Controls.

use crate::{ClientModifiers, ModKeyMappingRef, VanillaKeyMappingRef};
use std::num::NonZeroU64;

/// Construct a handle from the game's client generation and resolved mapping slot.
pub fn vanilla_key_mapping_ref(client_generation: NonZeroU64, slot: usize) -> VanillaKeyMappingRef {
    VanillaKeyMappingRef {
        client_generation,
        slot,
    }
}

/// Inspect a handle at the game's client and slot boundary.
pub fn vanilla_key_mapping_parts(mapping: VanillaKeyMappingRef) -> (NonZeroU64, usize) {
    (mapping.client_generation, mapping.slot)
}

/// Construct a handle from a resolved slot in the client's mod mapping registry.
pub fn mod_key_mapping_ref(registry_generation: NonZeroU64, slot: usize) -> ModKeyMappingRef {
    ModKeyMappingRef {
        registry_generation,
        slot,
    }
}

/// Inspect a handle at the game's mod mapping registry boundary.
pub fn mod_key_mapping_parts(mapping: ModKeyMappingRef) -> (NonZeroU64, usize) {
    (mapping.registry_generation, mapping.slot)
}

/// The screens in which two mappings can conflict.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum KeyConflictContext {
    /// Conflicts on every screen.
    #[default]
    Universal,
    /// Conflicts while playing.
    InGame,
    /// Conflicts in a GUI.
    Gui,
}
impl KeyConflictContext {
    /// Whether these screen contexts overlap.
    pub fn conflicts(self, other: Self) -> bool {
        self == Self::Universal || other == Self::Universal || self == other
    }
}
#[cfg(test)]
mod context_tests {
    use super::KeyConflictContext::*;
    #[test]
    fn key_context_case_order() {
        assert_eq!([Universal as u8, InGame as u8, Gui as u8], [0, 1, 2]);
        assert!(Universal.conflicts(Gui));
        assert!(!InGame.conflicts(Gui));
    }
}

/// A key a mod registers through `Registrar::register_key_mapping`: it is listed under the mod's name on
/// Options > Controls, and the player's binding and reset are kept by mod id in `sunrise-mods.txt`.
/// Its default is `key` with `modifiers` held; shift, control and alt are bound, super is not.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct KeyRegistration {
    /// The key's id, unique within its mod (`zoom`).
    pub id: String,
    /// The translation key for the label Controls shows.
    pub label: String,
    /// The default key (`key.keyboard.z`).
    pub key: String,
    /// The default modifiers, in GLFW's input-event encoding (shift 1, control 2, alt 4).
    pub modifiers: ClientModifiers,
    /// The category translation key within its mod's heading; empty for none.
    pub category: String,
    /// When this mapping can conflict with another binding.
    pub context: KeyConflictContext,
}

impl KeyRegistration {
    /// Sets the conflict context.
    pub fn context(mut self, context: KeyConflictContext) -> Self {
        self.context = context;
        self
    }
    /// Key `id` of the registering mod, bound to `key` with no modifiers and labelled `id`.
    pub fn new(id: &str, key: &str) -> Self {
        Self {
            id: id.to_owned(),
            label: id.to_owned(),
            key: key.to_owned(),
            modifiers: ClientModifiers::default(),
            category: String::new(),
            context: KeyConflictContext::Universal,
        }
    }

    /// Sets the label translation key Controls shows.
    pub fn label(mut self, label: &str) -> Self {
        self.label = label.to_owned();
        self
    }

    /// Sets the modifiers the default binding holds.
    pub fn modifiers(mut self, modifiers: ClientModifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// Sets the category translation key the binding is listed under.
    pub fn category(mut self, category: &str) -> Self {
        self.category = category.to_owned();
        self
    }
}
