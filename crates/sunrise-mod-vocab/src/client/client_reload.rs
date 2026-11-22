//! Client asset reload listeners and their registration context, the client
//! counterpart of `server_reload`, as NeoForge's
//! `AddClientReloadListenersEvent` beside its server event.
#[cfg(test)]
#[path = "client_reload_tests.rs"]
mod tests;

use crate::{
    FileToIdConverter, Identifier, ReloadResourceError,
    world::reload::{Boundary, ReloadFiles},
    world::reload_order::ReloadOrderError,
};
use std::sync::{Arc, LazyLock};

/// Sunrise's vanilla client reload listeners, from the 26.3 `Minecraft`
/// resource-manager registration order: language, textures, shaders, sounds,
/// the atlas, fonts, the grass and foliage colour listeners, the model manager,
/// equipment assets, paletted textures, then the entity and block-entity render
/// dispatchers, particles, the game renderer, the level extractor, clouds, the
/// GPU warnlist, regional compliance and the GUI.
// no wire: namespace for local reload dependency IDs, not a payload.
#[non_exhaustive]
pub struct VanillaClientListeners;
impl VanillaClientListeners {
    /// Vanilla's model manager: building the next models in prepare and publishing
    /// them in apply, with the model-baking notification. Atlas upload precedes it. Use this anchor
    /// rather than its string value, which need not
    /// match NeoForge's identifier.
    pub fn model_manager() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:model_manager").unwrap());
        ID.clone()
    }
}

/// Selected client asset files for one converter, with resource-pack overrides
/// resolved. Keys have their directory and extension removed. This cannot read
/// data files or filesystem paths. IDs and bytes are borrowed from this view.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: a local prepare view of the winning client asset files.
pub struct ClientReloadContext {
    files: ReloadFiles,
}
impl ClientReloadContext {
    pub(crate) fn new(files: ReloadFiles) -> Self {
        Self { files }
    }
    /// Converted IDs, across all namespaces, in deterministic resource-ID order.
    pub fn list(&self) -> impl Iterator<Item = &Identifier> {
        self.files.list()
    }
    /// The winning bytes for a converted ID, or None outside this directory.
    pub fn read(&self, id: &Identifier) -> Option<&[u8]> {
        self.files.read(id)
    }
}

/// A client-side `SimplePreparableReloadListener`, as `ServerReloadListener`
/// reads data: prepare reads the new pack view without changing live state;
/// the game consumes its returned action to publish the result. All prepares
/// finish before any apply, including vanilla's, runs on the game-owning thread.
/// Prepare cannot see another listener's new published state. Dependencies order
/// both phases deterministically; they do not remove the barrier. On a prepare
/// or engine preflight error, pending actions are dropped without running, keeping
/// the previous assets.
pub trait ClientReloadListener: Send + Sync + 'static {
    /// Parse and validate new assets without changing the listener's live state.
    /// Return `Ok(Box::new(move || { /* publish prepared assets */ }))` with owned
    /// data. The action is consumed once and cannot report an error.
    ///
    /// As vanilla's `SimpleJsonResourceReloadListener.scanDirectory`, a
    /// listener that parses its files logs and skips one that won't parse,
    /// carrying the rest through; returning `Err` aborts the whole reload and
    /// keeps every listener's previous assets. Attach the original error to the
    /// file's ID with [`ReloadResourceError::new`] to preserve its cause.
    /// A panic in `prepare` or apply skips this listener for the session. The
    /// mod and listener are reported, and unrelated listeners still apply.
    /// Native callbacks must publish their own state atomically: the engine
    /// cannot undo writes a closure makes to private mod memory.
    fn prepare(
        &self,
        context: &ClientReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError>;
}
impl ClientReloadListener for Boundary {
    fn prepare(
        &self,
        _: &ClientReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        Ok(Box::new(|| {}))
    }
}

/// The client reload's listener registry, as `add_client_reload_listeners` hands it out: its
/// registration methods and a read-only view of names and edges, rebuilt on each client reload.
/// Listener names need not belong to the registering mod. A rejected registration call
/// returns its error and does not change the registry. A callback that panics or incurs
/// a phase refusal loses all of its pending registrations and dependencies.
pub trait ClientReloadListeners {
    /// Register a unique listener and converter, e.g. `FileToIdConverter::json("machines")`.
    /// Keep the `Arc` to read the listener's published state outside reloads.
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ClientReloadListener>,
    ) -> Result<(), ReloadOrderError>;
    /// Require `first` before `second`; both names must already be registered.
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError>;
    /// The registered names in registration order, vanilla anchor included.
    fn get_registry(&self) -> Vec<Identifier>;
    /// The dependency edges as `(first, second)` names.
    fn get_graph(&self) -> Vec<(Identifier, Identifier)>;
    /// The name of the last vanilla client listener preceding mod listeners by default.
    fn get_last_vanilla_listener(&self) -> Identifier;
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ClientReloadListeners`].
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ClientListenerRegistry {
    core: crate::world::reload::ReloadCore<dyn ClientReloadListener>,
}
impl ClientReloadListeners for ClientListenerRegistry {
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ClientReloadListener>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id, converter, listener)
    }
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_dependency(first, second)
    }
    fn get_registry(&self) -> Vec<Identifier> {
        self.core.registry().map(|(id, _)| id.clone()).collect()
    }
    fn get_graph(&self) -> Vec<(Identifier, Identifier)> {
        self.core
            .graph()
            .map(|(first, second)| (first.clone(), second.clone()))
            .collect()
    }
    fn get_last_vanilla_listener(&self) -> Identifier {
        VanillaClientListeners::model_manager()
    }
}
impl ClientListenerRegistry {
    /// Run one registration callback and discard only its appended rows and edges on failure.
    // hidden: the engine stages one listener at a time; mods receive only the registrar trait.
    #[doc(hidden)]
    pub fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn ClientReloadListeners) -> bool,
    ) {
        let checkpoint = self.core.checkpoint();
        if !listen(self) {
            self.core.rollback(
                checkpoint,
                (
                    crate::HookId::AddClientReloadListeners,
                    "add_client_reload_listeners",
                ),
            );
        }
    }

    pub(crate) fn new() -> Self {
        Self {
            core: crate::world::reload::ReloadCore::new(
                VanillaClientListeners::model_manager(),
                || -> Arc<dyn ClientReloadListener> { Arc::new(Boundary) },
            ),
        }
    }
    pub(crate) fn finish(
        self,
    ) -> Result<
        Vec<crate::world::reload::RegisteredListener<dyn ClientReloadListener>>,
        ReloadOrderError,
    > {
        self.core.finish(&VanillaClientListeners::model_manager())
    }
}
