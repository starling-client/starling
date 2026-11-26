//! The startup registrar's config declarations.
use crate::{ConfigListener, ConfigRegistrationError, ConfigSpec};
use std::sync::Arc;

/// Register native configuration files under the calling mod's identity
/// (NeoForge's `ModContainer.registerConfig`).
// no wire: the registrar borrows startup state and retains native callbacks in this process.
pub trait ConfigSpecsRegistrar {
    /// Register a config in startup declaration order (NeoForge's
    /// `ModContainer.registerConfig`). The optional listener hears only this
    /// config's lifecycle (`ConfigEvent`, NeoForge's `ModConfigEvent`). Use
    /// `None` if the file does not need a listener.
    ///
    /// A duplicate registered filename is reported and returned as an error without
    /// replacing the first declaration, even if its owner or kind is different.
    fn register_config(
        &mut self,
        spec: ConfigSpec,
        listener: Option<Arc<ConfigListener>>,
    ) -> Result<(), ConfigRegistrationError>;
}
