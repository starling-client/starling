//! Server data reload listeners and their registration context.
#[cfg(test)]
#[path = "server_reload_tests.rs"]
mod tests;

use crate::{
    FileToIdConverter, Identifier, ReloadResourceError,
    world::reload::{Boundary, ReloadCore, ReloadFiles},
    world::reload_order::ReloadOrderError,
};
use std::{
    any::Any,
    collections::BTreeMap,
    marker::PhantomData,
    sync::{Arc, LazyLock},
};

/// Sunrise's vanilla server reload listeners.
// no wire: namespace for local reload dependency IDs, not a payload.
#[non_exhaustive]
pub struct VanillaServerListeners;
impl VanillaServerListeners {
    pub(crate) fn functions_ref() -> &'static Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:function_library").unwrap());
        &ID
    }
    /// Vanilla's function library: reading the next function files in prepare and
    /// admitting the library in apply. Use this anchor rather than its string value,
    /// which need not match NeoForge's identifier.
    pub fn functions() -> Identifier {
        Self::functions_ref().clone()
    }
}

/// Selected server data files for one converter, with pack overrides resolved.
/// Keys have their directory and extension removed. This cannot read assets or
/// filesystem paths. IDs and bytes are borrowed from this view.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: a local prepare view of the winning server data files.
pub struct ServerReloadContext {
    files: ReloadFiles,
}
impl ServerReloadContext {
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

/// A server-side `SimplePreparableReloadListener`: prepare reads the new pack
/// view without changing live state; the game consumes its returned action to
/// publish the result. All prepares finish before any apply, including vanilla's,
/// runs on the server's owning thread. Prepare cannot see another listener's new
/// published state. Dependencies order both phases; they do not remove the barrier.
/// On a prepare error, pending actions are dropped without running, keeping the
/// previous data and function library.
pub trait ServerReloadListener: Send + Sync + 'static {
    /// Parse and validate new data without changing the listener's live state.
    /// Return `Ok(Box::new(move || { /* publish prepared data */ }))` with owned
    /// data. The action is consumed once and cannot report an error. If the
    /// server cannot prepare its world tables, no apply action runs.
    ///
    /// As vanilla's `SimpleJsonResourceReloadListener.scanDirectory`, a
    /// listener that parses its files logs and skips one that won't parse,
    /// carrying the rest through; returning `Err` aborts the whole reload and
    /// keeps every listener's previous data. Attach the original error to the
    /// file's ID with [`ReloadResourceError::new`] to preserve its cause.
    /// A panic in `prepare` or apply skips this listener for the session. The
    /// mod and listener are reported, unrelated listeners still apply, and a
    /// failed listener does not replace its last admitted retained object.
    /// Native callbacks must publish their own state atomically: the engine
    /// cannot undo writes a closure makes to private mod memory.
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError>;
}
impl ServerReloadListener for Boundary {
    fn prepare(
        &self,
        _: &ServerReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        Ok(Box::new(|| {}))
    }
}

/// The server reload's listener registry, as `add_server_reload_listeners` hands it out: its
/// registration methods and a read-only view of names and edges, rebuilt on each server reload.
/// Listener names need not belong to the registering mod. A rejected registration call
/// returns its error and does not change the registry. A callback that panics or incurs a
/// phase refusal loses all of its pending registrations, retained listeners and dependencies.
pub trait ServerReloadListeners {
    /// Register a unique listener and converter, e.g. `FileToIdConverter::json("machines")`.
    /// Keep the `Arc` to read the listener's published state outside reloads.
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
    ) -> Result<(), ReloadOrderError>;
    /// [`add_listener`](Self::add_listener), keeping `retained` (the listener itself, as the
    /// mod holds it) in the reload resources; mods reach it through `add_retained_listener`.
    fn add_retained(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
        retained: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), ReloadOrderError>;
    /// Require `first` before `second`; both names must already be registered.
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError>;
    /// Borrow the registered names in registration order, vanilla anchor included.
    fn get_registry(&self) -> ServerReloadRegistryView<'_>;
    /// Borrow the dependency graph as `(first, second)` names.
    fn get_graph(&self) -> ServerReloadGraphView<'_>;
    /// Borrow the last vanilla server reload listener
    /// ([`VanillaServerListeners::functions`]), which unrelated mod listeners
    /// sort after by default (matching NeoForge's
    /// `SortedReloadListenerEvent.getLastVanillaListener`).
    fn get_last_vanilla_listener(&self) -> &Identifier;
}

// no wire: borrows the live server reload registry during registration.
/// The names registered so far in one server reload, borrowed from the registry.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServerReloadRegistryView<'a> {
    core: &'a ReloadCore<dyn ServerReloadListener>,
}

impl<'a> ServerReloadRegistryView<'a> {
    /// Iterate over registered names in registration order, vanilla anchor included.
    pub fn iter(&self) -> impl Iterator<Item = &'a Identifier> + use<'a> {
        self.core.registry().map(|(id, _)| id)
    }
}

// no wire: borrows the live server reload dependency graph during registration.
/// The current server reload dependencies, borrowed from the registry.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServerReloadGraphView<'a> {
    core: &'a ReloadCore<dyn ServerReloadListener>,
}

impl<'a> ServerReloadGraphView<'a> {
    /// Iterate over dependency edges in insertion order. Each `(first, second)`
    /// edge requires `first` to run before `second`; this is the explicit
    /// dependency graph, not a sorted list of listeners.
    pub fn iter(&self) -> impl Iterator<Item = (&'a Identifier, &'a Identifier)> + use<'a> {
        self.core.graph()
    }
}

impl dyn ServerReloadListeners + '_ {
    /// Register a listener whose object is also kept in the reload resources, as
    /// NeoForge's `addRetainedListener(key, listener)`: the caller builds the
    /// typed `ListenerKey` (`ListenerKey::new(id)`), and reads the object back
    /// through `ServerReloadResources` after the reload, from any server hook.
    pub fn add_retained_listener<T: ServerReloadListener>(
        &mut self,
        key: &ListenerKey<T>,
        converter: FileToIdConverter,
        listener: Arc<T>,
    ) -> Result<(), ReloadOrderError> {
        self.add_retained(key.id().clone(), converter, listener.clone(), listener)
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ServerReloadListeners`].
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServerListenerRegistry {
    core: ReloadCore<dyn ServerReloadListener>,
    retained: Vec<RetainedEntry>,
}
impl ServerReloadListeners for ServerListenerRegistry {
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id, converter, listener)
    }
    fn add_retained(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
        retained: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id.clone(), converter, listener)?;
        self.retained.push(RetainedEntry { id, any: retained });
        Ok(())
    }
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_dependency(first, second)
    }
    fn get_registry(&self) -> ServerReloadRegistryView<'_> {
        ServerReloadRegistryView { core: &self.core }
    }
    fn get_graph(&self) -> ServerReloadGraphView<'_> {
        ServerReloadGraphView { core: &self.core }
    }
    fn get_last_vanilla_listener(&self) -> &Identifier {
        VanillaServerListeners::functions_ref()
    }
}
impl ServerListenerRegistry {
    /// Run one registration callback and discard only its appended ordered and retained rows on failure.
    // hidden: the engine stages one listener at a time; mods receive only the registrar trait.
    #[doc(hidden)]
    pub fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn ServerReloadListeners) -> bool,
    ) {
        let checkpoint = self.core.checkpoint();
        let retained = self.retained.len();
        if !listen(self) {
            let site = (
                crate::HookId::AddServerReloadListeners,
                "add_server_reload_listeners",
            );
            let owner = crate::content::crash::current_native_mod();
            let discarded = self.retained.split_off(retained);
            self.core.rollback(checkpoint, site);
            for listener in discarded {
                crate::runtime::discard_registration(site, owner, listener);
            }
        }
    }

    pub(crate) fn new() -> Self {
        Self {
            core: ReloadCore::new(
                VanillaServerListeners::functions(),
                || -> Arc<dyn ServerReloadListener> { Arc::new(Boundary) },
            ),
            retained: Vec::new(),
        }
    }
    pub(crate) fn finish(self) -> Result<OrderedServerReload, ReloadOrderError> {
        Ok(OrderedServerReload {
            listeners: self.core.finish(VanillaServerListeners::functions_ref())?,
            retained: self.retained,
        })
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The ordered listeners and retained entries of one completed registration.
#[non_exhaustive]
pub struct OrderedServerReload {
    /// The listeners in dependency order.
    pub listeners: Vec<crate::world::reload::RegisteredListener<dyn ServerReloadListener>>,
    /// The entries the reload keeps.
    pub retained: Vec<RetainedEntry>,
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// One listener retained in the reload resources, as NeoForge's
/// `ReloadableServerResources` keeps a manager.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RetainedEntry {
    pub(crate) id: Identifier,
    pub(crate) any: Arc<dyn Any + Send + Sync>,
}

// no wire: typed lookup key for process-local retained trait objects; phantom type identity is not transferable.
/// A typed key to a retained reload listener, built by the registering mod
/// and read back through `ReloadResources`. `ListenerKey::new` accepts any
/// id; a key whose id does not match a registration, or whose type differs
/// from the registered object, reads back as `None`.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ListenerKey<T: ?Sized> {
    id: Identifier,
    marker: PhantomData<fn() -> T>,
}
impl<T: ?Sized> ListenerKey<T> {
    /// Names a listener key.
    pub fn new(id: Identifier) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
    /// The registered listener's name.
    pub fn id(&self) -> &Identifier {
        &self.id
    }
}
impl<T: ?Sized> Clone for ListenerKey<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            marker: PhantomData,
        }
    }
}
impl<T: ?Sized> std::fmt::Debug for ListenerKey<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerKey").field("id", &self.id).finish()
    }
}
impl<T: ?Sized> PartialEq for ListenerKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T: ?Sized> Eq for ListenerKey<T> {}

/// The retained server reload listeners of the last successful reload, as
/// NeoForge's `ReloadableServerResources`, reached from
/// `ServerHost::reload_resources`. A new reload replaces the whole store.
pub trait ServerReloadResources {
    /// The object retained under `id`, as its registering mod handed it over; mods read it
    /// through `listener`.
    fn retained(&self, id: &Identifier) -> Option<Arc<dyn Any + Send + Sync>>;
}
impl dyn ServerReloadResources + '_ {
    /// The retained listener registered under `key` in the last successful
    /// reload; `None` when nothing is registered under it, its type differs, or
    /// no reload has succeeded yet. The downcast itself checks the type.
    pub fn listener<T: ServerReloadListener>(&self, key: &ListenerKey<T>) -> Option<Arc<T>> {
        self.retained(key.id())?.downcast::<T>().ok()
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ServerReloadResources`].
#[derive(Default)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RetainedListeners {
    entries: BTreeMap<Identifier, Arc<dyn Any + Send + Sync>>,
}
impl ServerReloadResources for RetainedListeners {
    fn retained(&self, id: &Identifier) -> Option<Arc<dyn Any + Send + Sync>> {
        self.entries.get(id).cloned()
    }
}
impl RetainedListeners {
    pub(crate) fn install(
        &mut self,
        retained: Vec<RetainedEntry>,
        skipped: &std::collections::BTreeSet<Identifier>,
    ) {
        // Refused callbacks never replace a previously admitted retained object.
        self.entries.retain(|id, _| skipped.contains(id));
        self.entries.extend(
            retained.into_iter().filter_map(|entry| {
                (!skipped.contains(&entry.id)).then_some((entry.id, entry.any))
            }),
        );
    }
}
