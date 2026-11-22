//! Load-owned declarations and resolved mappings for registration-slot keys.
#[cfg(test)]
#[path = "menu_registry_tests.rs"]
mod menu_registry_tests;
use crate::{
    BlockEntityTypeKey, BlockKey, DimensionKey, EffectKey, EnchantmentKey, EntityTypeKey, FluidKey,
    ItemKey, MenuKey,
};
use std::{
    borrow::Borrow,
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
    sync::Arc,
};
use sunrise_mod_runtime::registry_keys::Names;

/// A canonical UTF-8 registry name. Keeps the text readable after the host borrow ends or the
/// registry publishes a new table, without copying the text or holding a registry lock.
/// Cloning shares the backing text. Display, equality, ordering and hashing use the full
/// namespaced string, not its slot or its registry's iteration order.
// no wire: pins a process-local registry table or borrows resident text.
// exhaustive: the private field keeps the storage and validated index opaque.
#[derive(Clone)]
pub struct RegistryName(RegistryNameText);

#[derive(Clone)]
enum RegistryNameText {
    Resident(&'static str),
    Snapshot { names: Names, index: usize },
}

impl RegistryName {
    pub(crate) const fn from_static(text: &'static str) -> Self {
        Self(RegistryNameText::Resident(text))
    }

    /// Borrow the full name, including its namespace, from this snapshot.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            RegistryNameText::Resident(text) => text,
            RegistryNameText::Snapshot { names, index } => &names.entries[*index].id,
        }
    }
}

impl Deref for RegistryName {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for RegistryName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for RegistryName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for RegistryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for RegistryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl PartialEq for RegistryName {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for RegistryName {}

impl PartialEq<str> for RegistryName {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for RegistryName {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<RegistryName> for str {
    fn eq(&self, other: &RegistryName) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<RegistryName> for &str {
    fn eq(&self, other: &RegistryName) -> bool {
        *self == other.as_str()
    }
}

impl PartialOrd for RegistryName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RegistryName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for RegistryName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

/// Pin the name at `slot` in this table, without retaining its borrow.
// hidden: registration snapshot construction, not a mod API.
#[doc(hidden)]
pub fn registry_name_snapshot(names: &Names, slot: u32) -> Option<RegistryName> {
    let index = slot as usize;
    names.entries.get(index)?;
    Some(RegistryName(RegistryNameText::Snapshot {
        names: names.clone(),
        index,
    }))
}

impl BlockKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Block, slot)
                .unwrap_or_else(|| (block_names()).id(slot).is_some());
        known.then_some(Self(slot)).ok_or("unknown BlockKey slot")
    }
}

impl FluidKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Fluid, slot)
                .unwrap_or_else(|| {
                    (crate::parts::resolved_fluids()
                        .get()
                        .unwrap_or(&crate::parts::requested_keys().fluids))
                    .id(slot)
                    .is_some()
                });
        known.then_some(Self(slot)).ok_or("unknown FluidKey slot")
    }
}

impl ItemKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Item, slot)
                .unwrap_or_else(|| {
                    (resolved_items()
                        .get()
                        .unwrap_or(&crate::parts::requested_keys().items))
                    .id(slot)
                    .is_some()
                });
        known.then_some(Self(slot)).ok_or("unknown ItemKey slot")
    }
}

pub(crate) fn resolved_items()
-> &'static std::sync::OnceLock<sunrise_mod_runtime::registry_keys::Names> {
    static EMPTY: std::sync::OnceLock<sunrise_mod_runtime::registry_keys::Names> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_items)
}

/// Complete the load-owned item table with all registry entries, preserving requested slots.
/// Missing forward references are refused before publishing any keys.
pub fn resolve_items<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), Vec<crate::parts::ItemKeyError>> {
    let names = crate::parts::requested_keys()
        .items
        .complete(ids, |mod_id, id| crate::parts::ItemKeyError {
            mod_id,
            id: id.to_owned(),
        })?;
    let _ = resolved_items().set(share_names(names));
    Ok(())
}

fn resolved_block_keys()
-> &'static std::sync::OnceLock<Arc<sunrise_mod_runtime::block_definitions::Blocks>> {
    static EMPTY: std::sync::OnceLock<Arc<sunrise_mod_runtime::block_definitions::Blocks>> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_blocks)
}

fn block_names() -> &'static sunrise_mod_runtime::registry_keys::Names {
    resolved_block_keys()
        .get()
        .map(|blocks| &blocks.names)
        .unwrap_or(&crate::parts::requested_keys().blocks)
}

/// Retain one copy of equal definitions across test installations.
///
/// A test installs the same declarations for every game it builds, and each install keeps its
/// own resolved table for the life of the process. Sharing is by full equality of the resolved
/// payload, never by hash alone. A production process installs once, so it keeps its own.
#[cfg(any(test, feature = "testing"))]
fn share_blocks(
    blocks: sunrise_mod_runtime::block_definitions::Blocks,
) -> Arc<sunrise_mod_runtime::block_definitions::Blocks> {
    #[cfg(any(test, feature = "testing"))]
    static SHARED: std::sync::Mutex<Vec<Arc<sunrise_mod_runtime::block_definitions::Blocks>>> =
        std::sync::Mutex::new(Vec::new());
    let mut shared = SHARED.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(equal) = shared.iter().find(|shared| ***shared == blocks) {
        return equal.clone();
    }
    let blocks = Arc::new(blocks);
    shared.push(blocks.clone());
    blocks
}
#[cfg(not(any(test, feature = "testing")))]
fn share_blocks(
    blocks: sunrise_mod_runtime::block_definitions::Blocks,
) -> Arc<sunrise_mod_runtime::block_definitions::Blocks> {
    Arc::new(blocks)
}

/// Retain one copy of equal key tables across test installations, as [`share_blocks`] does for
/// definitions: equality is full, never a hash alone. The comparisons run outside the lock, so
/// parallel installs queue only for the snapshot and the push; two that race may both push an
/// equal table, which costs a copy and nothing else.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn share_names(
    names: sunrise_mod_runtime::registry_keys::Names,
) -> sunrise_mod_runtime::registry_keys::Names {
    use sunrise_mod_runtime::registry_keys::Table;
    #[cfg(any(test, feature = "testing"))]
    static SHARED: std::sync::Mutex<Vec<Arc<Table>>> = std::sync::Mutex::new(Vec::new());
    let shared = SHARED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if let Some(equal) = shared.iter().find(|shared| ***shared == *names.0) {
        return Names(equal.clone());
    }
    SHARED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(names.0.clone());
    names
}
#[cfg(not(any(test, feature = "testing")))]
fn share_names(
    names: sunrise_mod_runtime::registry_keys::Names,
) -> sunrise_mod_runtime::registry_keys::Names {
    names
}

/// [`share_names`] for every table a finished registration holds.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn share_requests(
    mut requests: sunrise_mod_runtime::registry_keys::Requests,
) -> sunrise_mod_runtime::registry_keys::Requests {
    for names in [
        &mut requests.game_rules,
        &mut requests.blocks,
        &mut requests.fluids,
        &mut requests.items,
        &mut requests.effects,
        &mut requests.particles,
        &mut requests.block_entity_types,
        &mut requests.entity_types,
        &mut requests.game_events,
    ] {
        *names = share_names(std::mem::take(names));
    }
    requests
}

#[cfg(test)]
mod share_tests {
    use super::{share_names, share_requests};
    use std::sync::Arc;
    use sunrise_mod_runtime::registry_keys::{Names, Requests};

    fn build(mod_id: &'static str, ids: &[&str]) -> Names {
        let mut names = Names::default();
        for id in ids {
            names.request(mod_id, id).unwrap();
        }
        names
    }

    #[test]
    fn equal_tables_are_held_once_and_a_changed_one_leaves_the_others() {
        let first = share_names(build("mod", &["share_a:a", "share_a:b"]));
        let mut second = share_names(build("mod", &["share_a:a", "share_a:b"]));
        assert!(Arc::ptr_eq(&first.0, &second.0));
        // A table that differs in the requesting mod is not equal, however alike its ids.
        let mut other = build("mod", &["share_a:a", "share_a:b"]);
        other.request("another", "share_a:a").unwrap();
        assert!(!Arc::ptr_eq(&first.0, &share_names(other).0));
        // Writing through one handle copies; the shared table is unchanged.
        second.request("mod", "share_a:c").unwrap();
        assert_eq!(first.entries.len(), 2);
        assert_eq!(second.entries.len(), 3);
    }

    #[test]
    fn equal_registrations_share_every_table() {
        let requests = || {
            let mut requests = Requests::default();
            for (index, names) in [
                &mut requests.game_rules,
                &mut requests.blocks,
                &mut requests.fluids,
                &mut requests.items,
                &mut requests.effects,
                &mut requests.particles,
                &mut requests.block_entity_types,
                &mut requests.entity_types,
                &mut requests.game_events,
            ]
            .into_iter()
            .enumerate()
            {
                *names = build("mod", &[format!("share_b:table_{index}").as_str()]);
            }
            requests
        };
        let (first, second) = (share_requests(requests()), share_requests(requests()));
        let tables = |requests: &Requests| {
            [
                requests.game_rules.0.clone(),
                requests.blocks.0.clone(),
                requests.fluids.0.clone(),
                requests.items.0.clone(),
                requests.effects.0.clone(),
                requests.particles.0.clone(),
                requests.block_entity_types.0.clone(),
                requests.entity_types.0.clone(),
                requests.game_events.0.clone(),
            ]
        };
        for (first, second) in tables(&first).iter().zip(tables(&second).iter()) {
            assert!(Arc::ptr_eq(first, second));
        }
    }
}

/// Publish completed identities and immutable definitions together before constructing mesh tables.
/// World-number mappings remain owning-thread data.
pub fn resolve_block_keys<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    definition: impl FnMut(
        &str,
    )
        -> Result<sunrise_mod_runtime::block_definitions::BlockDefinition, String>,
) -> Result<(), Vec<String>> {
    let names = crate::parts::requested_keys()
        .blocks
        .complete(ids, |mod_id, id| {
            format!("mod {mod_id} requested unknown block {id}")
        })?;
    let blocks = sunrise_mod_runtime::block_definitions::Blocks::prepare(names, definition)?;
    let _ = resolved_block_keys().set(share_blocks(blocks));
    Ok(())
}

/// Find a block in the completed registry without allocating at a hook boundary.
pub fn block_key(id: &str) -> Option<BlockKey> {
    Some(BlockKey(*resolved_block_keys().get()?.names.slots.get(id)?))
}

/// Borrow the immutable definitions published with completed block identities.
pub fn block_definitions() -> Option<&'static sunrise_mod_runtime::block_definitions::Blocks> {
    resolved_block_keys().get().map(|blocks| &**blocks)
}

/// Find an item in the completed registry without allocating at a hook boundary.
pub fn item_key(id: &str) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.slots.get(id)?))
}

/// Resolve a typed identifier at a load boundary without formatting it as a string.
pub fn item_key_by_identifier(id: &crate::Identifier) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.identifiers.get(id)?))
}

/// Entity names and classification are published together, using the same keys.
pub(crate) fn resolved_entity_types()
-> &'static std::sync::OnceLock<sunrise_mod_runtime::registry_keys::EntityTypes> {
    static EMPTY: std::sync::OnceLock<sunrise_mod_runtime::registry_keys::EntityTypes> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_entity_types)
}
pub(crate) fn resolved_menus()
-> &'static std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>> {
    static EMPTY: std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>> =
        std::sync::RwLock::new(None);
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_menus)
}
pub(crate) static ENCHANTMENTS: std::sync::LazyLock<
    std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>>,
> = std::sync::LazyLock::new(|| std::sync::RwLock::new(None));

fn resolve_ordered<'a>(
    table: &std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>>,
    ids: impl IntoIterator<Item = &'a str>,
    kind: &str,
) -> Result<(), Vec<String>> {
    let mut table = table.write().unwrap_or_else(|error| error.into_inner());
    let names = table.get_or_insert_with(Default::default);
    let mut order = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let slot = names
            .request("sunrise", id)
            .map_err(|error| vec![format!("invalid {kind} registry key {id:?}: {error}")])?;
        order.push(slot);
    }
    names.order = order;
    Ok(())
}

fn ordered_keys<K>(
    table: &std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>>,
    key: impl Fn(u32) -> K,
) -> Vec<K> {
    let order = table
        .read()
        .ok()
        .and_then(|names| names.as_ref().map(|names| names.order.clone()))
        .unwrap_or_default();
    order.into_iter().map(key).collect()
}

fn ordered_id(
    table: &std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>>,
    slot: u32,
) -> Option<RegistryName> {
    let table = table.read().ok()?;
    registry_name_snapshot(table.as_ref()?, slot)
}

#[cfg(test)]
#[test]
fn ordered_name_survives_republication() {
    let table = std::sync::RwLock::new(None);
    resolve_ordered(&table, ["test:zinc", "test:copper"], "test").unwrap();
    let zinc = ordered_keys(&table, |slot| slot)[0];
    let retained = ordered_id(&table, zinc).unwrap();

    // The retained text must not keep a read lock as the same table is published again.
    assert!(table.try_write().is_ok());
    resolve_ordered(&table, ["test:copper", "test:gold"], "test").unwrap();
    let order = ordered_keys(&table, |slot| ordered_id(&table, slot).unwrap());
    let order: Vec<_> = order.iter().map(RegistryName::as_str).collect();
    assert_eq!(order, ["test:copper", "test:gold"]);
    assert_eq!(ordered_id(&table, zinc).as_deref(), Some("test:zinc"));
    assert!(ordered_id(&table, u32::MAX).is_none());
    drop(table);
    assert_eq!(&*retained, "test:zinc");
}

fn ordered_slot_is_live(
    table: &std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>>,
    slot: u32,
) -> bool {
    table
        .read()
        .ok()
        .and_then(|table| table.as_ref().map(|names| names.id(slot).is_some()))
        .unwrap_or(false)
}

/// Complete the entity-type registry in the order supplied by the game, with the registered
/// living classification of each type (None if the resource table did not describe it).
/// The accepted resources must match the existing frozen registry's keys and order.
pub fn resolve_entity_types<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    is_living: impl FnMut(&str) -> Option<bool>,
) -> Result<(), Vec<String>> {
    let names = crate::parts::requested_keys()
        .entity_types
        .complete(ids, |mod_id, id| {
            format!("mod {mod_id} requested unknown entity type {id}")
        })?;
    let names = share_names(names);
    let published = resolved_entity_types().get_or_init(|| {
        sunrise_mod_runtime::registry_keys::EntityTypes::new(names.clone(), is_living)
    });
    if published.names() != &names {
        return Err(vec![
            "the accepted resources' entity types differ from the frozen entity-type registry"
                .to_owned(),
        ]);
    }
    Ok(())
}

/// Complete the built-in menu registry in `ids`' registration order.
pub fn resolve_menus<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), Vec<String>> {
    resolve_ordered(resolved_menus(), ids, "menu")
}

/// Complete the data-loaded enchantment registry in `ids`' registry order.
pub fn resolve_enchantments<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), Vec<String>> {
    resolve_ordered(&ENCHANTMENTS, ids, "enchantment")
}

/// Every block key in vanilla's registry order, mod blocks after vanilla's.
pub fn block_keys() -> Vec<BlockKey> {
    let order = resolved_block_keys()
        .get()
        .map_or_else(Vec::new, |blocks| blocks.names.order.clone());
    order.into_iter().map(BlockKey).collect()
}

/// Every item key in vanilla's registry order, mod items after vanilla's.
pub fn item_keys() -> Vec<ItemKey> {
    resolved_items()
        .get()
        .map_or_else(Vec::new, |names| names.order.clone())
        .into_iter()
        .map(ItemKey)
        .collect()
}

/// Resolve an entity type without materializing an owned name at a read boundary.
pub fn resolved_entity_type_key(id: &str) -> Option<EntityTypeKey> {
    if !id.contains(':') {
        return resolved_entity_type_key_parts(&["minecraft:", id]);
    }
    let types = resolved_entity_types().get()?;
    Some(EntityTypeKey(*types.names().slots.get(id)?))
}

/// Resolve a composed engine name (notably boat/raft types) without heap formatting.
pub fn resolved_entity_type_key_parts(parts: &[&str]) -> Option<EntityTypeKey> {
    let types = resolved_entity_types().get()?;
    types.names().slots.iter().find_map(|(name, slot)| {
        name.bytes()
            .eq(parts.iter().flat_map(|part| part.bytes()))
            .then_some(EntityTypeKey(*slot))
    })
}

/// Every entity-type key in registry order, with native general types last.
pub fn entity_type_keys() -> Vec<EntityTypeKey> {
    resolved_entity_types()
        .get()
        .map_or_else(Vec::new, |types| {
            types
                .names()
                .order
                .iter()
                .map(|slot| EntityTypeKey(*slot))
                .collect()
        })
}

/// Every built-in menu key in registry order.
pub fn menu_keys() -> Vec<MenuKey> {
    ordered_keys(resolved_menus(), MenuKey)
}

/// Every loaded enchantment key in registry order.
pub fn enchantment_keys() -> Vec<EnchantmentKey> {
    ordered_keys(&ENCHANTMENTS, EnchantmentKey)
}

impl EntityTypeKey {
    /// Validate an incoming slot against the built-in registry, not a world's mapping.
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::registration_membership(crate::parts::RegistrationKeyKind::EntityType, slot)
            .unwrap_or_else(|| {
                resolved_entity_types()
                    .get()
                    .is_some_and(|types| types.names().id(slot).is_some())
            })
            .then_some(Self(slot))
            .ok_or("unknown EntityTypeKey slot")
    }
}

impl MenuKey {
    /// Validate an incoming slot against the built-in registry, not a world's mapping.
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        ordered_slot_is_live(resolved_menus(), slot)
            .then_some(Self(slot))
            .ok_or("unknown MenuKey slot")
    }
}

impl EnchantmentKey {
    /// Validate an incoming slot against the loaded registry, not a world's mapping.
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        ordered_slot_is_live(&ENCHANTMENTS, slot)
            .then_some(Self(slot))
            .ok_or("unknown EnchantmentKey slot")
    }
}

/// The canonical load-owned name of a resolved entity type, without allocating.
pub fn entity_type_name(key: EntityTypeKey) -> Option<&'static str> {
    resolved_entity_types().get()?.names().id(key.0)
}

/// The canonical name of an entity-type slot, available from requests during registration.
pub fn entity_type_key_id(key: EntityTypeKey) -> Option<RegistryName> {
    crate::parts::registration_name(crate::parts::RegistrationKeyKind::EntityType, key.0)
        .or_else(|| entity_type_name(key).map(RegistryName::from_static))
}

/// Pin the canonical name retained by a menu slot.
pub fn menu_key_id(key: MenuKey) -> Option<RegistryName> {
    ordered_id(resolved_menus(), key.0)
}

/// Pin the canonical name retained by an enchantment slot.
pub fn enchantment_key_id(key: EnchantmentKey) -> Option<RegistryName> {
    ordered_id(&ENCHANTMENTS, key.0)
}

impl EffectKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Effect, slot)
                .unwrap_or_else(|| {
                    (crate::parts::resolved_effects()
                        .get()
                        .map_or(&crate::parts::requested_keys().effects, |effects| {
                            &effects.names
                        }))
                    .id(slot)
                    .is_some()
                });
        known.then_some(Self(slot)).ok_or("unknown EffectKey slot")
    }
}

impl BlockEntityTypeKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known = crate::parts::registration_membership(
            crate::parts::RegistrationKeyKind::BlockEntityType,
            slot,
        )
        .unwrap_or_else(|| {
            (crate::parts::resolved_block_entity_types()
                .get()
                .unwrap_or(&crate::parts::requested_keys().block_entity_types))
            .id(slot)
            .is_some()
        });
        known
            .then_some(Self(slot))
            .ok_or("unknown BlockEntityTypeKey slot")
    }
}

impl DimensionKey {
    /// `minecraft:overworld`.
    pub const OVERWORLD: Self = Self(0);
    /// `minecraft:the_nether`.
    pub const NETHER: Self = Self(1);
    /// `minecraft:the_end`.
    pub const END: Self = Self(2);

    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let key = Self(slot);
        dimension_id(key).ok_or("unknown DimensionKey slot")?;
        Ok(key)
    }
}

/// The key of dimension `id`, numbering it when the game meets it for the first time.
pub fn dimension_key(id: &str) -> DimensionKey {
    DimensionKey(sunrise_mod_runtime::registry_keys::dimension_slot(id))
}

/// The name of a dimension's key, None for a slot the game never gave out.
pub fn dimension_id(key: DimensionKey) -> Option<&'static str> {
    sunrise_mod_runtime::registry_keys::dimension_id(key.0)
}

/// Look up an already-issued dimension key without minting a slot for unknown names.
pub(crate) fn existing_dimension_slot(id: &crate::Identifier) -> Option<u32> {
    let mut slot = 0;
    while let Some(name) = sunrise_mod_runtime::registry_keys::dimension_id(slot) {
        if name.split_once(':') == Some((id.namespace(), id.path())) {
            return Some(slot);
        }
        slot = slot.checked_add(1)?;
    }
    None
}

// no wire: process-local registration diagnostic retains a static hook or mod identity.
/// A fluid requested by a mod but absent from the completed registry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnknownFluid {
    /// Mod that requested the fluid.
    pub mod_id: &'static str,
    /// Canonical fluid identifier.
    pub id: String,
}

impl std::fmt::Display for UnknownFluid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mod {} requested unknown fluid {}", self.mod_id, self.id)
    }
}

/// Resolve fluid requests against the loaded behavior registry.
pub fn resolve_fluids(ids: &[&str], world_fluids: &[&str]) -> Result<(), Vec<UnknownFluid>> {
    let requests = crate::parts::requested_keys();
    let mut names = requests.fluids.clone();
    let mut errors = Vec::new();
    for entry in &names.entries {
        if !ids.contains(&entry.id.as_str()) && !requests.declared_fluids.contains_key(&entry.id) {
            errors.extend(entry.mods.iter().map(|mod_id| UnknownFluid {
                mod_id,
                id: entry.id.clone(),
            }));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for id in ids {
        names
            .request("sunrise", id)
            .expect("registry owns valid ids");
    }
    let table = &mut *names;
    for (slot, entry) in table.entries.iter().enumerate() {
        table
            .utf16_slots
            .insert(entry.id.encode_utf16().collect(), slot as u32);
        if let Some(path) = entry.id.strip_prefix("minecraft:") {
            table
                .utf16_slots
                .insert(path.encode_utf16().collect(), slot as u32);
        }
    }
    table.world_fluids = world_fluids
        .iter()
        .map(|name| table.slots.get(*name).copied())
        .collect();
    let _ = crate::parts::resolved_fluids().set(share_names(names));
    Ok(())
}

/// Resolve an engine fluid kind through the load-owned mapping.
pub fn world_fluid_key(index: usize) -> Option<FluidKey> {
    crate::parts::resolved_fluids()
        .get()?
        .world_fluids
        .get(index)
        .copied()
        .flatten()
        .map(FluidKey)
}

/// Find a fluid key at a data-loading boundary; a bare name means `minecraft:`.
pub fn fluid_key(id: &str) -> Option<FluidKey> {
    let names = crate::parts::resolved_fluids().get()?;
    let slot = if id.contains(':') {
        names.slots.get(id)
    } else {
        names.slots.get(&format!("minecraft:{id}"))
    };
    Some(FluidKey(*slot?))
}

/// Resolve an NBT UTF-16 fluid name without allocating.
pub fn fluid_key_utf16(id: &[u16]) -> Option<FluidKey> {
    Some(FluidKey(
        *crate::parts::resolved_fluids().get()?.utf16_slots.get(id)?,
    ))
}

/// The canonical registry name of a resolved fluid key.
pub fn fluid_id(key: FluidKey) -> Option<&'static str> {
    crate::parts::resolved_fluids().get()?.id(key.0)
}

/// Resolve all block entity types after registration and content loading.
pub fn resolve_block_entity_types<'a, T>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<T>,
) -> Result<(), Vec<String>> {
    let names = sunrise_mod_runtime::registry_keys::resolve_block_entity_types(
        &crate::parts::requested_keys().block_entity_types,
        ids,
        resolve,
    )?;
    let _ = crate::parts::resolved_block_entity_types().set(share_names(names));
    Ok(())
}

/// Lookup an engine-owned name after the registry is resolved.
pub fn block_entity_type_key(id: &str) -> Option<BlockEntityTypeKey> {
    Some(BlockEntityTypeKey(
        *crate::parts::resolved_block_entity_types()
            .get()?
            .slots
            .get(id)?,
    ))
}

/// Canonical name of a load-owned block entity type slot.
pub fn block_entity_type_name(key: BlockEntityTypeKey) -> Option<&'static str> {
    crate::parts::resolved_block_entity_types()
        .get()
        .unwrap_or(&crate::parts::requested_keys().block_entity_types)
        .id(key.0)
}

pub(crate) fn block_entity_type_display_name(key: BlockEntityTypeKey) -> RegistryName {
    crate::parts::registration_name(crate::parts::RegistrationKeyKind::BlockEntityType, key.0)
        .unwrap_or_else(|| {
            RegistryName::from_static(block_entity_type_name(key).unwrap_or("<invalid>"))
        })
}

/// Resolve requested effects once, after every mod's content has been registered.
pub fn resolve_effects<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<usize>,
) -> Result<(), Vec<String>> {
    let effects = sunrise_mod_runtime::registry_keys::resolve_effects(
        &crate::parts::requested_keys().effects,
        ids,
        resolve,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    })?;
    let _ = crate::parts::resolved_effects().set(effects);
    Ok(())
}

/// Resolve an effect operation to its load-owned engine registry index.
pub fn effect_index(key: EffectKey) -> Option<usize> {
    crate::parts::resolved_effects()
        .get()?
        .indices
        .get(key.0 as usize)
        .copied()
        .flatten()
}

/// Canonical name of a resolved effect slot, if present.
pub fn effect_id(key: EffectKey) -> Option<&'static str> {
    crate::parts::resolved_effects().get()?.names.id(key.0)
}

/// Canonical diagnostic name of a trusted, load-owned slot, independent of world resolution.
pub fn block_id(key: BlockKey) -> Option<&'static str> {
    block_names().id(key.0)
}

/// Canonical diagnostic name retained by an item registration slot.
pub fn item_id(key: ItemKey) -> Option<&'static str> {
    resolved_items()
        .get()
        .unwrap_or(&crate::parts::requested_keys().items)
        .id(key.0)
}

/// Find an effect by its full registry name in the completed registry, without allocating.
pub fn effect_key(id: &str) -> Option<EffectKey> {
    Some(EffectKey(
        *crate::parts::resolved_effects()
            .get()?
            .names
            .slots
            .get(id)?,
    ))
}

/// The API key for an engine registry index, also for entries not explicitly requested by mods.
pub fn effect_key_by_index(index: usize) -> Option<EffectKey> {
    crate::parts::resolved_effects()
        .get()?
        .keys
        .get(index)
        .copied()
        .map(EffectKey)
}

impl crate::BoolGameRuleKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known = crate::parts::registration_membership(
            crate::parts::RegistrationKeyKind::GameRule,
            slot,
        )
        .unwrap_or_else(|| crate::parts::requested_keys().game_rules.id(slot).is_some());
        known.then_some(Self(slot)).ok_or("unknown bool rule slot")
    }
}

impl crate::IntGameRuleKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known = crate::parts::registration_membership(
            crate::parts::RegistrationKeyKind::GameRule,
            slot,
        )
        .unwrap_or_else(|| crate::parts::requested_keys().game_rules.id(slot).is_some());
        known.then_some(Self(slot)).ok_or("unknown int rule slot")
    }
}

/// Complete startup game events in builtin registration order.
pub fn resolve_game_events<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), Vec<String>> {
    let names = crate::parts::requested_keys()
        .game_events
        .complete(ids, |mod_id, id| {
            format!("mod {mod_id} requested unknown game event {id}")
        })?;
    if let Some(installed) = crate::parts::installed() {
        let _ = installed.resolved_game_events.set(share_names(names));
    }
    Ok(())
}
/// Resolve an exact id in the installed load.
pub fn game_event_key(id: &str) -> Option<crate::GameEventKey> {
    let names = crate::parts::installed()?.resolved_game_events.get()?;
    Some(crate::GameEventKey(*names.slots.get(id)?))
}
/// Read an installed slot without trusting a retained mod key.
pub fn game_event_key_id(key: crate::GameEventKey) -> Option<RegistryName> {
    crate::parts::registration_name(crate::parts::RegistrationKeyKind::GameEvent, key.0).or_else(
        || {
            crate::parts::installed()?
                .resolved_game_events
                .get()?
                .id(key.0)
                .map(RegistryName::from_static)
        },
    )
}
impl crate::GameEventKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::registration_membership(crate::parts::RegistrationKeyKind::GameEvent, slot)
            .unwrap_or_else(|| {
                crate::parts::installed()
                    .and_then(|installed| installed.resolved_game_events.get())
                    .is_some_and(|names| names.id(slot).is_some())
            })
            .then_some(Self(slot))
            .ok_or("unknown GameEventKey slot")
    }
}
