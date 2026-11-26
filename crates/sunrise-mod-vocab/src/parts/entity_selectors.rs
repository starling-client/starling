//! Entity selector tokens mods register (`Registrar::register_entity_selector`), kept for the
//! command registries built at every world open.
use super::*;
use std::sync::Arc;

/// A selector token a mod registered: its owner, the word after `@`, and its type.
#[derive(Clone)]
pub(crate) struct EntitySelectorInfo {
    pub(crate) owner: crate::callback_owner::CallbackOwner,
    pub(crate) name: Box<str>,
    pub(crate) selector: Arc<dyn crate::EntitySelectorType>,
}

/// The selector tokens every installed mod registered, in token-name order.
pub(crate) fn installed_entity_selectors() -> &'static [EntitySelectorInfo] {
    installed().map_or(&[], |installed| installed.keys.entity_selectors.as_slice())
}

impl dyn Registrar + '_ {
    /// Register selector token `name` (`allies` gives `@allies`), once, from `Hooks::register`.
    ///
    /// Follows NeoForge's `EntitySelectorManager.register`: `name` must be non-empty, use only
    /// Brigadier unquoted-string characters (`[0-9A-Za-z_.+-]`), not collide with vanilla's
    /// `p`, `a`, `r`, `s`, `e`, or `n`, and be unique among registered tokens while startup
    /// registration is open. Invalid calls return
    /// [`EntitySelectorRegistrationError`](crate::EntitySelectorRegistrationError) without
    /// poisoning unrelated registrations.
    ///
    /// `selector.base()` is not called during registration; protocol string and payload limits
    /// apply at the network boundary when syncing token metadata to clients.
    pub fn register_entity_selector<T: crate::EntitySelectorType>(
        &mut self,
        name: &str,
        selector: T,
    ) -> Result<(), crate::EntitySelectorRegistrationError> {
        self.add_entity_selector(name, Arc::new(selector))
    }
}

/// Registrar operations for entity selector tokens.
pub trait EntitySelectorsRegistrar {
    /// Register selector token `name` with `selector`; mods reach it through
    /// `register_entity_selector`.
    fn add_entity_selector(
        &mut self,
        name: &str,
        selector: Arc<dyn crate::EntitySelectorType>,
    ) -> Result<(), crate::EntitySelectorRegistrationError>;
}
