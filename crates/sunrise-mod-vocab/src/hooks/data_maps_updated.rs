//! The `data_maps_updated` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The data maps of a registry were reloaded on the server or synced to the
    /// client (NeoForge's `DataMapsUpdatedEvent`): the place to build caches
    /// from the new values or post-process them. `registry` is the registry's id
    /// (`minecraft:item`, `minecraft:block`) and `cause` says how they arrived.
    /// A listener cannot cancel or change anything.
    ///
    /// `ServerReload`: the server merged the values of the types mods registered
    /// (`data_map_types`) from the world's data packs and mods' files.
    /// Fires once for each registry that has a type, right after
    /// `tags_updated`'s `ServerDataLoad` (tags are bound), on a world's first open
    /// and after every `/reload`. `context.data_map(...)`, `context.data_map_value(...)`
    /// and `context.data_map_entries(...)` read the new values.
    ///
    /// `ClientSync`: data maps are server-side and not synced to clients, so
    /// this cause does not fire; it comes with the sync.
    ///
    /// Receives a [`ReloadContext`] for published data queries; world handles and
    /// transactions are not present on this context.
    #[hook(
        receiver = "associated",
        context = "reload",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "DataMapsUpdatedEvent",
        version = 2
    )]
    fn data_maps_updated(registry: &str, cause: mod_api::DataMapsUpdateCause);
}
