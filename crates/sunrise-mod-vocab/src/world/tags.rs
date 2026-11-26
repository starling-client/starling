//! Resolved tag names, built-in convention tag key types, and the read-only merged-tag service.
use crate::{
    BiomeKey, BlockKey, DamageTypeKey, EnchantmentKey, EntityTypeKey, FluidKey, Identifier, ItemKey,
};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// A structure's persistent resource identity (`minecraft:village_plains`).
// exhaustive: a key newtype mods construct directly.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub struct StructureKey(pub Identifier);

impl Hash for StructureKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.namespace().hash(state);
        self.0.path().hash(state);
    }
}

/// A potion's persistent resource identity (`minecraft:swiftness`).
// exhaustive: a key newtype mods construct directly.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub struct PotionKey(pub Identifier);

impl Hash for PotionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.namespace().hash(state);
        self.0.path().hash(state);
    }
}

/// A world clock's persistent resource identity (`minecraft:overworld`).
// exhaustive: a key newtype mods construct directly.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub struct WorldClockKey(pub Identifier);

impl Hash for WorldClockKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.namespace().hash(state);
        self.0.path().hash(state);
    }
}

#[derive(Clone, Copy, Debug)]
enum TagKeyRepr {
    Slot(u32),
    Static(&'static str),
}

/// A process-lifetime tag name, either a compile-time convention constant from
/// [`tags::c`](crate::tags::c) or resolved once through
/// [`Registrar::tag_key`](crate::TagsRegistrar::tag_key) or [`Tags::key`].
///
/// The type parameter `K` binds the key to one registry (`TagKey<ItemKey>`,
/// `TagKey<BlockKey>`, and so on) so a tag from one registry cannot be passed to a
/// query for another. Dynamic keys returned by `Registrar::tag_key` or `Tags::key`
/// are untyped (`TagKey<()>`, written `TagKey`); call [`TagKey::typed`] to bind
/// one to a registry before querying.
///
/// Membership belongs to the current authoritative data reload, not to the key.
///
/// ```compile_fail
/// use mod_api::{Tags, tags::c};
/// fn wrong_registry(tags: &dyn Tags) {
///     let _ = tags.items(c::blocks::ORES);
/// }
/// ```
// no wire: decoding validates the u32 slot with TagKey::from_slot on the host.
// exhaustive: the runtime and built-in convention constants alone create tag keys
pub struct TagKey<K = ()> {
    repr: TagKeyRepr,
    _marker: PhantomData<fn() -> K>,
}

impl<K> Clone for TagKey<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for TagKey<K> {}

impl<K> fmt::Debug for TagKey<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.repr {
            TagKeyRepr::Slot(slot) => f.debug_tuple("TagKey").field(&slot).finish(),
            TagKeyRepr::Static(name) => f.debug_tuple("TagKey").field(&name).finish(),
        }
    }
}

impl<K> PartialEq for TagKey<K> {
    fn eq(&self, other: &Self) -> bool {
        match (self.repr, other.repr) {
            (TagKeyRepr::Slot(a), TagKeyRepr::Slot(b)) => a == b,
            (TagKeyRepr::Static(a), TagKeyRepr::Static(b)) => a == b,
            (TagKeyRepr::Slot(slot), TagKeyRepr::Static(name))
            | (TagKeyRepr::Static(name), TagKeyRepr::Slot(slot)) => {
                sunrise_mod_runtime::registry_keys::with_tag_name(slot, |s| s == Some(name))
            }
        }
    }
}

impl<K> Eq for TagKey<K> {}

impl<K> Hash for TagKey<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        with_tag_name(*self, |name| name.hash(state));
    }
}

impl<K> TagKey<K> {
    /// Construct a compile-time key for a validated canonical built-in tag name.
    pub(crate) const fn from_static(name: &'static str) -> Self {
        Self {
            repr: TagKeyRepr::Static(name),
            _marker: PhantomData,
        }
    }

    pub(crate) const fn slot_unchecked(slot: u32) -> Self {
        Self {
            repr: TagKeyRepr::Slot(slot),
            _marker: PhantomData,
        }
    }

    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        sunrise_mod_runtime::registry_keys::with_tag_name(slot, |id| {
            id.map(|_| Self::slot_unchecked(slot))
                .ok_or("unknown TagKey slot")
        })
    }

    pub(crate) fn wire_slot(&self) -> u32 {
        match self.repr {
            TagKeyRepr::Slot(slot) => slot,
            TagKeyRepr::Static(name) => {
                sunrise_mod_runtime::registry_keys::tag_slot_str(name).unwrap_or(u32::MAX)
            }
        }
    }

    /// Drop the registry type parameter when passing a tag key to an untyped boundary.
    pub const fn untyped(self) -> TagKey<()> {
        TagKey {
            repr: self.repr,
            _marker: PhantomData,
        }
    }
}

impl TagKey<()> {
    /// Bind an untyped dynamic tag key (from [`Tags::key`] or
    /// [`Registrar::tag_key`](crate::TagsRegistrar::tag_key)) to registry `K`.
    pub const fn typed<K>(self) -> TagKey<K> {
        TagKey {
            repr: self.repr,
            _marker: PhantomData,
        }
    }
}

/// The result of reading one registry tag from the current authoritative snapshot.
// no wire: this native lookup result carries process-local registry keys.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TagMembers<K> {
    /// No tag by this name bound, including a definition with an unresolved required entry.
    Missing,
    /// The bound tag's members in vanilla's order. Missing optional entries are skipped; the list may be empty.
    Members(Vec<K>),
    /// The snapshot or a required key mapping is unavailable. No partial list is returned.
    Unavailable,
}

/// Read merged registry tags without borrowing the game's registry structs. A client reads the
/// tags its server sent. Built-in `c:` and `neoforge:` convention tags from
/// [`tags::c`](crate::tags::c) are bound beneath data packs and mods so mods can pass them
/// directly to the typed methods below. The list methods distinguish absent tags from loaded
/// empty tags, and report unavailable until that registry's authoritative tags are applied or
/// if a member key cannot be resolved. A retained tag key reads the current snapshot on each
/// call, including after reload.
pub trait Tags {
    /// Resolve a validated name once. The name need not currently have members;
    /// retain its key across reloads and call [`TagKey::typed`] to bind it to a registry.
    /// `None` if the runtime cannot allocate a name slot.
    fn key(&self, _id: &Identifier) -> Option<TagKey> {
        unimplemented!("Tags::key")
    }
    /// Whether `item` is in the current item tag. An unknown tag is false;
    /// unavailable authoritative tags or item mappings answer None.
    fn is_in_tag(&self, _item: ItemKey, _tag: TagKey<ItemKey>) -> Option<bool> {
        unimplemented!("Tags::is_in_tag")
    }
    /// `DamageSource.is(TagKey)`: whether damage type `damage_type`, a hit's
    /// [`HurtSource::damage_type`](crate::HurtSource::damage_type) (`minecraft:arrow`), is in the
    /// current damage type tag, as bound at the last data reload. An unknown tag is false;
    /// an unknown damage type, or unavailable authoritative damage type tags, answer None.
    /// A tag service with no damage type data, such as a crafting kind's, answers None.
    fn damage_type_in_tag(&self, _damage_type: &str, _tag: TagKey<DamageTypeKey>) -> Option<bool> {
        None
    }
    /// The names of the tags that currently contain `item`, each once, in no specified order.
    /// `None` means the item mapping or authoritative tag names are unavailable.
    /// A known item with no tags returns an empty list. This does not resolve tag keys.
    ///
    /// Each call tests the item against every bound item tag and builds a new list: keep it
    /// out of per-tick and per-entity hooks. Read it when the answer is needed, or after a
    /// data reload, and keep the list; for one known tag, [`is_in_tag`](Self::is_in_tag) is
    /// a single lookup.
    fn item_tags(&self, _item: ItemKey) -> Option<Vec<Identifier>> {
        unimplemented!("Tags::item_tags")
    }
    /// Every currently bound item tag's name, including empty tags, each once in no specified order.
    /// `None` means the authoritative tag names are unavailable.
    /// A loaded item registry with no bound tags returns an empty list.
    /// Call [`key`](Self::key) separately if you need a key for membership reads.
    ///
    /// Each call builds a new list of every bound item tag: keep it out of per-tick and
    /// per-entity hooks, and keep the list between data reloads.
    fn item_tag_names(&self) -> Option<Vec<Identifier>> {
        unimplemented!("Tags::item_tag_names")
    }
    /// Whether `block` is in the current block tag (`BlockState.is(TagKey)`). An unknown tag
    /// is false; unavailable authoritative tags or block mappings answer None.
    fn block_in_tag(&self, _block: BlockKey, _tag: TagKey<BlockKey>) -> Option<bool> {
        unimplemented!("Tags::block_in_tag")
    }
    /// Whether `fluid` is in the current fluid tag (`FluidState.is(TagKey)`). An unknown tag
    /// is false; unavailable authoritative tags or fluid mappings answer None.
    fn fluid_in_tag(&self, _fluid: FluidKey, _tag: TagKey<FluidKey>) -> Option<bool> {
        unimplemented!("Tags::fluid_in_tag")
    }
    /// Read the current item tag in vanilla's order, including unrequested vanilla items.
    fn items(&self, _tag: TagKey<ItemKey>) -> TagMembers<ItemKey> {
        unimplemented!("Tags::items")
    }
    /// Read the current fluid tag in vanilla's order.
    fn fluids(&self, _tag: TagKey<FluidKey>) -> TagMembers<FluidKey> {
        unimplemented!("Tags::fluids")
    }
    /// Read the current block tag in vanilla's order, including unrequested vanilla blocks.
    fn blocks(&self, _tag: TagKey<BlockKey>) -> TagMembers<BlockKey> {
        unimplemented!("Tags::blocks")
    }
    /// Read the current entity type tag in vanilla's order.
    fn entity_types(&self, _tag: TagKey<EntityTypeKey>) -> TagMembers<EntityTypeKey> {
        TagMembers::Unavailable
    }
    /// Read the current biome tag in vanilla's order.
    fn biomes(&self, _tag: TagKey<BiomeKey>) -> TagMembers<BiomeKey> {
        TagMembers::Unavailable
    }
    /// Read the current enchantment tag in vanilla's order.
    fn enchantments(&self, _tag: TagKey<EnchantmentKey>) -> TagMembers<EnchantmentKey> {
        TagMembers::Unavailable
    }
    /// Read the current structure tag in vanilla's order.
    fn structures(&self, _tag: TagKey<StructureKey>) -> TagMembers<StructureKey> {
        TagMembers::Unavailable
    }
    /// Read the current potion tag in vanilla's order.
    fn potions(&self, _tag: TagKey<PotionKey>) -> TagMembers<PotionKey> {
        TagMembers::Unavailable
    }
    /// Read the current damage type tag in vanilla's order.
    fn damage_types(&self, _tag: TagKey<DamageTypeKey>) -> TagMembers<Identifier> {
        TagMembers::Unavailable
    }
    /// Read the current world clock tag in vanilla's order.
    fn world_clocks(&self, _tag: TagKey<WorldClockKey>) -> TagMembers<WorldClockKey> {
        TagMembers::Unavailable
    }
}

/// Resolve a tag name at the game boundary.
pub fn tag_key(id: &Identifier) -> Option<TagKey> {
    sunrise_mod_runtime::registry_keys::tag_slot(id).map(TagKey::slot_unchecked)
}

/// Borrow a tag's canonical name within the runtime's read lock, without formatting or cloning.
pub fn with_tag_name<K, R>(key: TagKey<K>, read: impl FnOnce(Option<&str>) -> R) -> R {
    match key.repr {
        TagKeyRepr::Static(name) => read(Some(name)),
        TagKeyRepr::Slot(slot) => sunrise_mod_runtime::registry_keys::with_tag_name(slot, read),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash_of<T: Hash>(value: T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn static_and_dynamic_tag_keys_compare_equal_and_hash_identically() {
        let static_key: TagKey<ItemKey> = crate::tags::c::items::INGOTS;
        let id = Identifier::parse("c:ingots").unwrap();
        let dynamic_key: TagKey<ItemKey> = tag_key(&id).unwrap().typed();
        assert_eq!(static_key, dynamic_key);
        assert_eq!(dynamic_key, static_key);
        assert_eq!(hash_of(static_key), hash_of(dynamic_key));

        let other = crate::tags::c::items::GEMS;
        assert_ne!(static_key, other);
        assert_ne!(dynamic_key, other);

        let untyped = static_key.untyped();
        assert_eq!(untyped, tag_key(&id).unwrap());
    }
}
