//! Cached server data and the preload/reload boundaries that own its resolved registries.
use super::Game;
use std::sync::{Arc, Mutex};
use sunrise_client::{
    advancements::Tree,
    crafting::ResolvedSources,
    items::Items,
    lan::data::{FrozenDamage, PackRegistryDefinitions, SyncData, WorldTags},
};
use sunrise_world::item_defaults::ItemData;

/// The recipes one server load resolved (the `ingredient` hook asked once per
/// occurrence), shared by every table and the network copy that reads them.
struct ServerSources {
    root: std::path::PathBuf,
    generation: u64,
    overlay: u64,
    sources: Arc<ResolvedSources>,
}

/// One slot, held while it is built: the builder of a generation and pack set
/// is the only one that asks the hook, and the others wait for its answer.
static SOURCES: Mutex<Option<ServerSources>> = Mutex::new(None);

/// Every byte of the packs' files, what a pack set is keyed by.
pub(super) fn overlay_key(overlay: &sunrise_world::loot::PackOverlay) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    overlay.files.hash(&mut hasher);
    overlay.priority_layers.hash(&mut hasher);
    overlay.conditions.hash(&mut hasher);
    hasher.finish()
}

/// This server's recipes for `overlay`, resolved once per generation and pack
/// set: the first caller builds them and later ones adopt them (`/reload`
/// forgets them first, so it asks again).
pub(in crate::game) fn server_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    items: &Items,
    overlay: &sunrise_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    server_sources_with(root, item_data, items, overlay, false, || {
        server_data_load(root, item_data)
    })
}

/// `server_sources` for an owning load that goes on to build the tables with
/// `Recipes::from_resolved`, which reports the recipes that failed to resolve
/// together with its own. Sources with failures are never cached.
fn server_sources_collecting(
    root: &std::path::Path,
    item_data: &ItemData,
    items: &Items,
    overlay: &sunrise_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    server_sources_with(root, item_data, items, overlay, true, || {
        server_data_load(root, item_data)
    })
}

fn server_sources_with(
    root: &std::path::Path,
    item_data: &ItemData,
    items: &Items,
    overlay: &sunrise_world::loot::PackOverlay,
    collecting: bool,
    load: impl FnOnce() -> std::io::Result<sunrise_world::native_load_context::LoadContext>,
) -> std::io::Result<Arc<ResolvedSources>> {
    let generation = item_data.defaults().generation();
    let key = overlay_key(overlay);
    let mut slot = SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(held) = slot
        .as_ref()
        .filter(|held| held.generation == generation && held.overlay == key && held.root == root)
    {
        return Ok(held.sources.clone());
    }
    let context = load()?;
    let sources = collect(root, items, overlay, &context, collecting)?;
    if sources.is_complete() {
        *slot = Some(ServerSources {
            root: root.to_owned(),
            generation,
            overlay: key,
            sources: sources.clone(),
        });
    }
    Ok(sources)
}

fn collect(
    root: &std::path::Path,
    items: &Items,
    overlay: &sunrise_world::loot::PackOverlay,
    context: &sunrise_world::native_load_context::LoadContext,
    collecting: bool,
) -> std::io::Result<Arc<ResolvedSources>> {
    let loaded = ResolvedSources::load_collecting(root, items, overlay, context)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    if !collecting {
        loaded
            .check()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    }
    Ok(Arc::new(loaded))
}

/// Initial tables borrow the settings already captured with their item data.
/// Servers retain the existing single-flight recipe cache; later owning loads
/// still enter through `server_sources`/`load_sources` and capture fresh inputs.
pub(in crate::game) fn initial_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    items: &Items,
    overlay: &sunrise_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    use sunrise_world::native_load_context::DataLoadOrigin;
    let context = item_data.context();
    if context.origin() == DataLoadOrigin::ServerDataLoad {
        server_sources_with(
            root,
            item_data,
            items,
            overlay,
            true,
            || Ok(context.clone()),
        )
    } else {
        collect(root, items, overlay, context, true)
    }
}

/// Build server recipes privately so a refused reload leaves the current cache intact.
/// Like `load_sources`, for a caller that builds the tables with `Recipes::from_resolved`.
pub(in crate::game) fn prepare_server_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    items: &Items,
    overlay: &sunrise_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    collect(
        root,
        items,
        overlay,
        &server_data_load(root, item_data)?,
        true,
    )
}

pub(in crate::game) fn install_server_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &sunrise_world::loot::PackOverlay,
    sources: Arc<ResolvedSources>,
) {
    *SOURCES.lock().unwrap_or_else(|e| e.into_inner()) = Some(ServerSources {
        root: root.to_owned(),
        generation: item_data.defaults().generation(),
        overlay: overlay_key(overlay),
        sources,
    });
}

/// The recipes of a load of `origin`: a server's are shared, a client's own.
pub(in crate::game) fn load_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    items: &Items,
    origin: sunrise_world::native_load_context::DataLoadOrigin,
    overlay: &sunrise_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    use sunrise_world::native_load_context::DataLoadOrigin;
    match origin {
        DataLoadOrigin::ServerDataLoad => {
            server_sources_collecting(root, item_data, items, overlay)
        }
        DataLoadOrigin::ClientPreload | _ => collect(
            root,
            items,
            overlay,
            &data_load_context(root, item_data, origin)?,
            true,
        ),
    }
}

/// Let go of the resolved recipes: a `/reload`, a world closed or a preload no
/// world used. Not a world opening, which adopts what its preload resolved.
pub(in crate::game) fn forget_server_sources() {
    *SOURCES.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// What `sync_data` loaded from this resource generation.
#[derive(Clone)]
pub(super) struct Synced {
    pub(super) root: std::path::PathBuf,
    pub(super) with_packs: bool,
    pub(super) generation: u64,
    pub(super) result: std::result::Result<Arc<SyncData>, String>,
}

/// Only damage is bound eagerly for gameplay. Full wire data remains lazy
/// except when the entering world actually defines damage types.
#[derive(Default)]
pub(in crate::game) struct SyncCache {
    pub(super) data: Option<Synced>,
    pub(super) damage: Option<FrozenDamage>,
    pub(super) world_tags: Option<WorldTags>,
    pub(super) packed_registries: Option<Arc<PackRegistryDefinitions>>,
    pub(super) advancements: Option<Arc<Tree>>,
    pub(super) world_packs: bool,
}

static SYNC: Mutex<SyncCache> = Mutex::new(SyncCache {
    data: None,
    damage: None,
    world_tags: None,
    packed_registries: None,
    advancements: None,
    world_packs: false,
});

/// Load wire data once, retaining this session's frozen damage definitions.
pub(in crate::game) fn sync_data(
    root: &std::path::Path,
    item_data: &ItemData,
    items: &Items,
    overlay: &sunrise_world::loot::PackOverlay,
) -> std::result::Result<Arc<SyncData>, String> {
    let mut sync = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    cached_sync_data(&mut sync, root, overlay, item_data, items)
}

pub(super) fn cached_sync_data(
    sync: &mut SyncCache,
    root: &std::path::Path,
    overlay: &sunrise_world::loot::PackOverlay,
    item_data: &ItemData,
    items: &Items,
) -> std::result::Result<Arc<SyncData>, String> {
    let packs = !overlay.files.is_empty();
    let generation = item_data.defaults().generation();
    if sync
        .data
        .as_ref()
        .is_some_and(|cached| cached.generation != generation || cached.root != root)
    {
        sync.data = None;
    }
    let load = |packed, advancements, world_tags| {
        let sources =
            server_sources(root, item_data, items, overlay).map_err(|error| error.to_string())?;
        match &sync.damage {
            Some(damage) => SyncData::load_with_frozen_damage(
                root,
                overlay,
                damage,
                packed,
                item_data.defaults(),
                &sources,
                advancements,
                world_tags,
            ),
            None => SyncData::load(root, overlay, item_data.defaults(), &sources),
        }
        .map(Arc::new)
    };
    // A late vanilla preload must not replace a packed world's lazy snapshot.
    if sync.data.is_none() && sync.world_packs && !packs {
        return load(None, None, None);
    }
    let advancements = sync.advancements.as_ref();
    sync.data
        .get_or_insert_with(|| Synced {
            root: root.to_owned(),
            with_packs: packs,
            generation,
            result: load(
                sync.packed_registries.as_ref(),
                advancements,
                sync.world_tags.as_ref(),
            ),
        })
        .result
        .clone()
}

/// Keep a usable no-pack preload, but decode any eager world wire data in a
/// private cache so a refused decode cannot publish the world's registries.
pub(in crate::game) fn prepare_world_sync_data(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &sunrise_world::loot::PackOverlay,
    damage: &sunrise_world::damage_source::Catalog,
    items: Arc<Items>,
    packed_registries: Option<Arc<PackRegistryDefinitions>>,
    advancements: Arc<Tree>,
) -> std::result::Result<SyncCache, String> {
    let mut staged = SyncCache {
        data: SYNC.lock().unwrap_or_else(|e| e.into_inner()).data.clone(),
        packed_registries,
        world_tags: Some(WorldTags::new(items.clone(), damage)),
        advancements: Some(advancements),
        ..Default::default()
    };
    initialize_world_sync_data(&mut staged, root, overlay, damage, item_data, &items)?;
    Ok(staged)
}

pub(in crate::game) fn install_world_sync_data(staged: SyncCache) {
    *SYNC.lock().unwrap_or_else(|e| e.into_inner()) = staged;
}

pub(super) fn initialize_world_sync_data(
    sync: &mut SyncCache,
    root: &std::path::Path,
    overlay: &sunrise_world::loot::PackOverlay,
    damage: &sunrise_world::damage_source::Catalog,
    item_data: &ItemData,
    items: &Items,
) -> std::result::Result<(), String> {
    sync.world_packs = !overlay.files.is_empty();
    if sync.world_packs
        || sync.data.as_ref().is_some_and(|cached| {
            cached.generation != item_data.defaults().generation() || cached.root != root
        })
    {
        sync.data = None;
    }
    sync.damage = Some(FrozenDamage::new(damage, overlay));
    if overlay.kind("damage_type").next().is_some() {
        cached_sync_data(sync, root, overlay, item_data, items)?;
    }
    Ok(())
}

/// Let go of what `sync_data` loaded: a preload no world used
/// (game/world/world_loading.rs), or the previous session when another world opens.
pub(in crate::game) fn forget_sync_data() {
    *SYNC.lock().unwrap_or_else(|e| e.into_inner()) = SyncCache::default();
}

/// Without a hub yet, retain this world's frozen registries through `/reload`
/// too. World entry discards this snapshot before loading another world's.
pub(in crate::game) fn cached_reload_sync() -> Option<Arc<SyncData>> {
    SYNC.lock()
        .unwrap_or_else(|e| e.into_inner())
        .data
        .as_ref()
        .and_then(|cached| cached.result.as_ref().ok())
        .cloned()
}

/// Keep lazy-cache admission aligned with the current packs even when no
/// complete wire snapshot has been loaded yet. Damage definitions stay frozen.
#[cfg(test)]
pub(super) fn prepare_sync_reload(
    sync: &mut SyncCache,
    overlay: &sunrise_world::loot::PackOverlay,
) -> Option<Arc<SyncData>> {
    sync.world_packs = !overlay.files.is_empty();
    sync.data
        .as_ref()
        .and_then(|cached| cached.result.as_ref().ok())
        .cloned()
}

/// `/reload` (MinecraftServer.reloadResources) under a running hub: the
/// tags, recipes and advancements read again with the world's packs, the
/// synchronized registries `kept` (the hub's) as they were, and that becomes
/// what `sync_data` gives.
pub(in crate::game) fn prepare_wire_reload(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &sunrise_world::loot::PackOverlay,
    kept: &SyncData,
    sources: &ResolvedSources,
    advancements: &Arc<Tree>,
    world_tags: &WorldTags,
) -> std::result::Result<Arc<SyncData>, String> {
    SyncData::reload(
        root,
        overlay,
        kept,
        item_data.defaults(),
        sources,
        advancements,
        Some(world_tags),
    )
    .map(|mut sync| {
        sync.features.clone_from(&kept.features);
        Arc::new(sync)
    })
}

pub(in crate::game) fn install_wire_reload(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &sunrise_world::loot::PackOverlay,
    sync: Option<Arc<SyncData>>,
    advancements: Arc<Tree>,
    world_tags: WorldTags,
) {
    // Kept pack dialogs count as packs: a world opened without packs after
    // a `/reload` that dropped them must read its own registries again.
    let mut cache = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    cache.world_packs = !overlay.files.is_empty();
    cache.advancements = Some(advancements);
    cache.world_tags = Some(world_tags);
    cache.data = sync.map(|sync| Synced {
        root: root.to_owned(),
        with_packs: !overlay.files.is_empty() || sync.has_pack_entries(),
        generation: item_data.defaults().generation(),
        result: Ok(sync),
    });
}

impl Game<'_> {
    /// The commands' `minecraft:dialog` ids as the clients get them in the
    /// configuration phase (`sync`, the world's packs' dialogs among them):
    /// `/dialog show` parses an id against these and sends its index.
    pub(in crate::game) fn sync_dialog_ids(&mut self, sync: &SyncData) {
        let Some(names) = sync.dynamic.registries.get("minecraft:dialog") else {
            return;
        };
        let ids = names.iter().map(|(name, _)| name.clone()).collect();
        self.ui
            .registries
            .by_registry
            .entry("minecraft:dialog".to_owned())
            .or_default()
            .ids = ids;
    }
}

/// A server's own data load over `item_data`'s registry inputs: the owner of
/// the data a server reads and sends, whichever window shares the `ItemData`.
pub(in crate::game) fn server_data_load(
    root: &std::path::Path,
    item_data: &ItemData,
) -> std::io::Result<sunrise_world::native_load_context::LoadContext> {
    data_load_context(
        root,
        item_data,
        sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
}

fn data_load_context(
    root: &std::path::Path,
    item_data: &ItemData,
    origin: sunrise_world::native_load_context::DataLoadOrigin,
) -> std::io::Result<sunrise_world::native_load_context::LoadContext> {
    use sunrise_world::native_load_context::DataLoadOrigin;
    let inventory = sunrise_mod_package::enabled::EnabledMods::read(root)?;
    let ids: Vec<_> = if origin == DataLoadOrigin::ServerDataLoad {
        inventory.server_ids().collect()
    } else {
        inventory.client_ids().collect()
    };
    Ok(item_data.context().for_load(origin, ids))
}

/// The registries and tags a server sends without data packs over them, read
/// on a helper thread while a server's world prepares: most worlds have no
/// packs, so it is what `sync_data` would load once the world is open.
pub(in crate::game) struct SyncPreload {
    root: std::path::PathBuf,
    generation: u64,
    worker: std::thread::JoinHandle<std::result::Result<SyncData, String>>,
}

/// Start reading the server's data beside the world's preparation; `None`
/// when it is read already, or being read (`preload_world_data`).
pub(in crate::game) fn preload_sync_data(
    item_data: &Arc<ItemData>,
    items: Arc<Items>,
) -> Option<SyncPreload> {
    let generation = item_data.defaults().generation();
    if !SYNC.try_lock().is_ok_and(|sync| {
        sync.damage.is_none()
            && sync
                .data
                .as_ref()
                .is_none_or(|cached| cached.generation != generation)
    }) {
        return None;
    }
    let root = item_data.root().to_owned();
    let item_data = Arc::clone(item_data);
    // Loading recipes asks mods' ingredient and recipe property set hooks.
    std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .name("Server data preload".into())
        .spawn(move || {
            let none = sunrise_world::loot::PackOverlay::NONE;
            SyncData::load(
                item_data.root(),
                &none,
                item_data.defaults(),
                server_sources(item_data.root(), &item_data, &items, &none)
                    .map_err(|error| error.to_string())?
                    .as_ref(),
            )
        })
        .map(|worker| SyncPreload {
            root,
            generation,
            worker,
        })
        .ok()
}

impl SyncPreload {
    /// The world is open: without packs of its own (`overlay`), what was read
    /// becomes `sync_data`'s; with them, `sync_data` loads them as before.
    pub(in crate::game) fn adopt(self, overlay: &sunrise_world::loot::PackOverlay) {
        if !overlay.files.is_empty() {
            return;
        }
        if let Ok(read) = self.worker.join() {
            let mut sync = SYNC.lock().unwrap_or_else(|e| e.into_inner());
            if !sync.world_packs && sync.data.as_ref().is_none_or(|cached| cached.with_packs) {
                sync.data = Some(Synced {
                    root: self.root,
                    with_packs: false,
                    generation: self.generation,
                    result: read.map(Arc::new),
                });
            }
        }
    }
}

#[cfg(test)]
mod initial_source_tests {
    use super::*;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};

    #[test]
    fn client_preload_propagates_source_errors_instead_of_empty_tables() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), "{}").unwrap();
        std::fs::create_dir_all(root.path().join("data/minecraft")).unwrap();
        // A file where the recipe directory belongs is an owning-load error.
        std::fs::write(root.path().join("data/minecraft/recipe"), "not a directory").unwrap();
        let inputs = Arc::new(LoadInputs::new(Default::default(), Default::default()));
        let context = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, [] as [&str; 0]);
        let data = ItemData::load(root.path(), context).unwrap();
        let items = Items::for_tests(Default::default());
        assert!(
            initial_sources(
                root.path(),
                &data,
                &items,
                &sunrise_world::loot::PackOverlay::NONE
            )
            .is_err()
        );
    }

    #[test]
    fn initial_sources_keep_refused_recipes_for_the_table_build() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), "{}").unwrap();
        let folder = root.path().join("data/minecraft/recipe");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.json"), r#"{"type":"absent:serializer"}"#).unwrap();
        std::fs::write(
            folder.join("b.json"),
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[],"result":{"id":"minecraft:stick"}}"#,
        )
        .unwrap();
        let inputs = Arc::new(LoadInputs::new(Default::default(), Default::default()));
        let context = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, [] as [&str; 0]);
        let data = ItemData::load(root.path(), context).unwrap();
        let items = Items::for_tests(Default::default());
        let sources = initial_sources(
            root.path(),
            &data,
            &items,
            &sunrise_world::loot::PackOverlay::NONE,
        )
        .unwrap();
        assert!(!sources.is_complete());
        let none = sunrise_world::loot::PackOverlay::NONE;
        let error =
            sunrise_client::crafting::Recipes::from_resolved(root.path(), &items, &none, &sources)
                .err()
                .unwrap()
                .to_string();
        assert!(error.starts_with("2 recipes failed to load:"), "{error}");
        assert!(
            error.find("recipe/a.json").unwrap() < error.find("recipe/b.json").unwrap(),
            "{error}"
        );
    }
}
