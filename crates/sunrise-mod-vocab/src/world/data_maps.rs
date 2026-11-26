//! Typed reads of data maps (NeoForge's `DataMapType` and `IWithData.getData`): the value the
//! server's data packs and mods' files gave a registry entry, after every pack was applied.
use crate::{
    Backend, BiomeKey, BiomeView, BlockEntityTypeKey, BlockKey, DataMapType, EffectKey,
    EnchantmentKey, EntityTypeKey, FluidKey, Host, ItemKey, NoBackend, ProfessionKey, RegistryName,
    Tag,
};
use entry::EntryName;

/// A data map a mod registers and reads, typed (NeoForge's `DataMapType<R, T>`): its id, the
/// registry its entries are in (through [`DataMap::Entry`]), and how a value decodes.
///
/// Builtin maps are already registered. They return canonical codec NBT compounds,
/// including Float distances (with infinity preserved) and Int frequencies. This store
/// is server-only and is not synced to clients.
/// Mod-defined maps retain the JSON-to-NBT mapping below.
///
/// Register a mod-defined map in `data_map_types` with [`DataMap::ty`], then read an entry's value with its
/// key's `data` ([`ItemKey::data`], [`EntityTypeKey::data`]...), or by name with
/// [`data_map_value`] and [`data_map_entries`]. Typed key reads return owned decoded values;
/// reads by name borrow raw NBT from the host.
/// A value arrives as the NBT form of the JSON a file held: a whole number is an `Int`, or
/// a `Long` past `i32`, a fraction a `Double`, a boolean a `Byte`, an object a `Compound`
/// and an array a `List`. Files containing integers outside signed 64-bit range or `null`
/// at any nesting depth cannot be converted to NBT and are skipped atomically at load time
/// with an error log, preserving earlier layers.
/// Fractional/exponent JSON tokens use the parser's
/// `f64` value; empty and heterogeneous arrays retain their elements.
///
/// ```
/// use mod_api::{DataMap, ItemKey, Tag};
/// /// `projecte:emc`: an item's EMC value.
/// struct Emc;
/// impl DataMap for Emc {
///     type Entry = ItemKey;
///     type Value = i64;
///     fn id(&self) -> &str {
///         "projecte:emc"
///     }
///     fn decode(&self, value: &Tag) -> Option<i64> {
///         match value {
///             Tag::Int(value) => Some(i64::from(*value)),
///             Tag::Long(value) => Some(*value),
///             _ => None,
///         }
///     }
/// }
/// assert_eq!(Emc.ty().registry, "minecraft:item");
/// ```
pub trait DataMap {
    /// The registry entries the map attaches values to: one of the [`DataMapEntry`] keys.
    type Entry: DataMapEntry;
    /// The owned result of decoding an entry's borrowed tag.
    type Value;
    /// The map's namespaced id (`namespace:path`), as registered.
    fn id(&self) -> &str;
    /// Decode an entry's value, refusing one of another shape.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// The type to push in `data_map_types`: this map's id, for its entries' registry.
    fn ty(&self) -> DataMapType {
        DataMapType {
            id: self.id().to_owned(),
            registry: <Self::Entry as entry::Entry>::REGISTRY.to_owned(),
        }
    }
}

/// A registry entry a data map can attach a value to through a typed key. Sealed: a registry
/// without one is read by name with [`data_map_value`].
pub trait DataMapEntry: entry::Entry {}

mod entry {
    use super::*;

    // exhaustive: the sealed lookup represents only resident, pinned or owned registry names.
    pub enum EntryName<'a> {
        Borrowed(&'a str),
        Pinned(RegistryName),
        Owned(String),
    }

    impl EntryName<'_> {
        pub(super) fn as_str(&self) -> &str {
            match self {
                Self::Borrowed(id) => id,
                Self::Pinned(id) => id.as_str(),
                Self::Owned(id) => id,
            }
        }
    }

    /// Common read-only lookup surface shared by [`Backend`] and [`crate::ReloadContext`].
    pub trait DataMapReader {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&crate::Tag>, DataMapError>;
        fn game_event_key_id(
            &self,
            key: crate::GameEventKey,
        ) -> Result<RegistryName, crate::GameEventKeyError>;
        fn item_key_id(&self, key: ItemKey) -> Option<&str>;
        fn block_key_id(&self, key: BlockKey) -> Option<&str>;
        fn fluid_key_id(&self, key: FluidKey) -> Option<&str>;
        fn effect_key_id(&self, key: EffectKey) -> Option<&str>;
        fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName>;
        fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName>;
        fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str>;
        fn biome_key_id(&self, key: &BiomeKey) -> Result<&str, crate::BiomeKeyError>;
    }

    impl DataMapReader for &(dyn Backend + '_) {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&crate::Tag>, DataMapError> {
            crate::prelude::DataMapsBackend::data_map_value(*self, registry, entry, map)
        }
        fn game_event_key_id(
            &self,
            key: crate::GameEventKey,
        ) -> Result<RegistryName, crate::GameEventKeyError> {
            crate::prelude::KeysBackend::game_event_key_id(*self, key)
        }
        fn item_key_id(&self, key: ItemKey) -> Option<&str> {
            crate::prelude::KeysBackend::item_key_id(*self, key)
        }
        fn block_key_id(&self, key: BlockKey) -> Option<&str> {
            crate::prelude::KeysBackend::block_key_id(*self, key)
        }
        fn fluid_key_id(&self, key: FluidKey) -> Option<&str> {
            crate::prelude::KeysBackend::fluid_key_id(*self, key)
        }
        fn effect_key_id(&self, key: EffectKey) -> Option<&str> {
            crate::prelude::KeysBackend::effect_key_id(*self, key)
        }
        fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName> {
            crate::prelude::KeysBackend::entity_type_key_id(*self, key)
        }
        fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName> {
            crate::prelude::KeysBackend::enchantment_key_id(*self, key)
        }
        fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str> {
            crate::prelude::KeysBackend::block_entity_type_key_id(*self, key)
        }
        fn biome_key_id(&self, key: &BiomeKey) -> Result<&str, crate::BiomeKeyError> {
            crate::prelude::KeysBackend::biome_view(*self, key).map(|view| view.id())
        }
    }

    impl DataMapReader for &(dyn crate::ReloadContext + '_) {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&crate::Tag>, DataMapError> {
            crate::ReloadContext::data_map_value(*self, registry, entry, map)
        }
        fn game_event_key_id(
            &self,
            key: crate::GameEventKey,
        ) -> Result<RegistryName, crate::GameEventKeyError> {
            crate::ReloadContext::game_event_key_id(*self, key)
        }
        fn item_key_id(&self, key: ItemKey) -> Option<&str> {
            crate::ReloadContext::item_key_id(*self, key)
        }
        fn block_key_id(&self, key: BlockKey) -> Option<&str> {
            crate::ReloadContext::block_key_id(*self, key)
        }
        fn fluid_key_id(&self, key: FluidKey) -> Option<&str> {
            crate::ReloadContext::fluid_key_id(*self, key)
        }
        fn effect_key_id(&self, key: EffectKey) -> Option<&str> {
            crate::ReloadContext::effect_key_id(*self, key)
        }
        fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName> {
            crate::ReloadContext::entity_type_key_id(*self, key)
        }
        fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName> {
            crate::ReloadContext::enchantment_key_id(*self, key)
        }
        fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str> {
            crate::ReloadContext::block_entity_type_key_id(*self, key)
        }
        fn biome_key_id(&self, key: &BiomeKey) -> Result<&str, crate::BiomeKeyError> {
            crate::ReloadContext::biome_key_id(*self, key)
        }
    }

    /// The registry an entry is in, and its id there through a read-only data-map source.
    pub trait Entry {
        /// The registry's id.
        const REGISTRY: &'static str;
        /// The entry's full registry id.
        fn id<'a>(&self, reader: &'a dyn DataMapReader) -> Result<EntryName<'a>, DataMapError>;
    }
}

/// `DataMapEntry` and `data` for a key whose id the keys backend answers.
macro_rules! typed_entries {
    ($($key:ty => $registry:literal, $id:ident;)+) => {$(
        impl DataMapEntry for $key {}
        impl entry::Entry for $key {
            const REGISTRY: &'static str = $registry;
            fn id<'a>(
                &self,
                reader: &'a dyn entry::DataMapReader,
            ) -> Result<EntryName<'a>, DataMapError> {
                reader.$id(*self).into_result()
            }
        }
        impl $key {
            /// This entry's value in data map `map` (`Holder.getData`), as the server's data
            /// packs and mods' files left it: the newest file wins, a `#tag` entry gives every
            /// entry in the tag its value, `replace` and `remove` apply, and conditional files
            /// and values count only when their conditions hold. `Ok(None)` where the map
            /// gives this entry no value. A present value is decoded into an owned `M::Value`
            /// that can outlive the host borrow. A refused decode is [`DataMapError::Undecodable`],
            /// not an absent value.
            /// A key that this host cannot name returns `DataMapError::InvalidKey`
            /// when its registry is available to the host.
            ///
            /// Server side: the server merges the maps when its data loads, on a world's first
            /// open and after every `/reload`, and a read sees the last load whole, never a mix
            /// of two. Readable from `data_maps_updated` on, in the world and while data
            /// reloads. A reload that drops a value drops it here too.
            pub fn data<M: DataMap<Entry = $key>>(
                self,
                host: &dyn Host,
                map: &M,
            ) -> Result<Option<M::Value>, DataMapError> {
                data(&self, host, map)
            }
        }
    )+};
}
trait KeyIdResult<'a> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError>;
}
impl<'a> KeyIdResult<'a> for Option<&'a str> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError> {
        self.map(EntryName::Borrowed)
            .ok_or(DataMapError::InvalidKey)
    }
}
impl<'a> KeyIdResult<'a> for Option<RegistryName> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError> {
        self.map(EntryName::Pinned).ok_or(DataMapError::InvalidKey)
    }
}
impl<'a> KeyIdResult<'a> for Result<RegistryName, crate::GameEventKeyError> {
    fn into_result(self) -> Result<EntryName<'a>, DataMapError> {
        self.map(EntryName::Pinned).map_err(|error| match error {
            crate::GameEventKeyError::Unavailable => DataMapError::Unavailable,
            crate::GameEventKeyError::InvalidKey => DataMapError::InvalidKey,
        })
    }
}
typed_entries! {
    crate::GameEventKey => "minecraft:game_event", game_event_key_id;
    ItemKey => "minecraft:item", item_key_id;
    BlockKey => "minecraft:block", block_key_id;
    FluidKey => "minecraft:fluid", fluid_key_id;
    EffectKey => "minecraft:mob_effect", effect_key_id;
    EntityTypeKey => "minecraft:entity_type", entity_type_key_id;
    EnchantmentKey => "minecraft:enchantment", enchantment_key_id;
    BlockEntityTypeKey => "minecraft:block_entity_type", block_entity_type_key_id;
}
impl DataMapEntry for BiomeKey {}
impl entry::Entry for BiomeKey {
    const REGISTRY: &'static str = "minecraft:worldgen/biome";
    fn id<'a>(&self, reader: &'a dyn entry::DataMapReader) -> Result<EntryName<'a>, DataMapError> {
        reader
            .biome_key_id(self)
            .map(EntryName::Borrowed)
            .map_err(|error| match error {
                crate::BiomeKeyError::Unavailable => DataMapError::Unavailable,
                crate::BiomeKeyError::InvalidIdentifier
                | crate::BiomeKeyError::UnknownBiome
                | crate::BiomeKeyError::WrongGeneration => DataMapError::InvalidKey,
            })
    }
}
impl BiomeKey {
    /// Read this biome's data as for [`ItemKey::data`], validating the key against the
    /// currently installed server world biome registry. Unknown biomes return
    /// [`DataMapError::InvalidKey`]; no installed registry returns `Unavailable`.
    pub fn data<M: DataMap<Entry = BiomeKey>>(
        &self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(self, host, map)
    }
}
impl BiomeView<'_> {
    /// Read this scoped biome view's data as for [`ItemKey::data`]. A view from another world
    /// or world-data generation returns [`DataMapError::InvalidKey`]; no installed registry
    /// returns `Unavailable`.
    pub fn data<M: DataMap<Entry = BiomeKey>>(
        self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
        let id = backend.biome_view_id(self).map_err(|error| match error {
            crate::BiomeKeyError::Unavailable => DataMapError::Unavailable,
            crate::BiomeKeyError::InvalidIdentifier
            | crate::BiomeKeyError::UnknownBiome
            | crate::BiomeKeyError::WrongGeneration => DataMapError::InvalidKey,
        })?;
        match backend.data_map_value(<BiomeKey as entry::Entry>::REGISTRY, id, map.id())? {
            Some(value) => map.decode(value).map(Some).ok_or(DataMapError::Undecodable),
            None => Ok(None),
        }
    }
}
impl DataMapEntry for ProfessionKey {}
impl entry::Entry for ProfessionKey {
    const REGISTRY: &'static str = "minecraft:villager_profession";
    fn id<'a>(&self, _: &'a dyn entry::DataMapReader) -> Result<EntryName<'a>, DataMapError> {
        Ok(EntryName::Owned(self.0.to_string()))
    }
}
impl ProfessionKey {
    /// This profession's value in data map `map`, as for [`ItemKey::data`].
    pub fn data<M: DataMap<Entry = ProfessionKey>>(
        &self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(self, host, map)
    }
}

/// Why a data map read has no answer. An entry the map gives no value is `Ok(None)`, not one of
/// these. An empty map yields an empty slice from [`data_map_entries`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DataMapError {
    /// No data maps are loaded here: a client (data maps are not synced to clients), a server
    /// before its world's data first loads, or a host with no game behind it.
    Unavailable,
    /// Neither the engine nor a mod registered the map for this registry in the current load
    /// (`data_map_types`).
    UnknownType,
    /// A typed read's [`DataMap::decode`] refused the entry's NBT value.
    Undecodable,
    /// The key has no registry name in this host, or belongs to another world or generation.
    InvalidKey,
}
impl NoBackend for DataMapError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// `entry`'s value in `map`, decoded through any read-only data-map source.
fn read_data<M: DataMap>(
    entry: &M::Entry,
    reader: &dyn entry::DataMapReader,
    map: &M,
) -> Result<Option<M::Value>, DataMapError> {
    let id = entry::Entry::id(entry, reader)?;
    let registry = <M::Entry as entry::Entry>::REGISTRY;
    match reader.data_map_value(registry, id.as_str(), map.id())? {
        Some(value) => map.decode(value).map(Some).ok_or(DataMapError::Undecodable),
        None => Ok(None),
    }
}

/// `entry`'s value in `map`, decoded.
fn data<M: DataMap>(
    entry: &M::Entry,
    host: &dyn Host,
    map: &M,
) -> Result<Option<M::Value>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    read_data(entry, &backend, map)
}

/// Entry `entry` of registry `registry`: its raw NBT value in data map `map`
/// (`IRegistryExtension.getData`). A bare entry id uses the `minecraft` namespace.
/// `Ok(None)` means the map has no value for this entry. A host without loaded maps or an
/// unknown map type returns a [`DataMapError`].
///
/// The tag borrows from the host's published data, not the lookup IDs. Use `Option::cloned`
/// only if you need the raw value after the host borrow ends.
pub fn data_map_value<'host>(
    host: &'host dyn Host,
    registry: &str,
    entry: &str,
    map: &str,
) -> Result<Option<&'host Tag>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    backend.data_map_value(registry, entry, map)
}

/// The whole published data map `map` of `registry`, sorted by entry ID
/// (`IRegistryExtension.getDataMap`). A known empty map yields an empty slice; unavailable
/// maps or an unknown type return a [`DataMapError`], not an empty slice.
///
/// The slice borrows from the host's published data, not the lookup IDs. Iterate by
/// reference, or call `to_vec` only if you need all the rows after that borrow ends.
pub fn data_map_entries<'host>(
    host: &'host dyn Host,
    registry: &str,
    map: &str,
) -> Result<&'host [(String, Tag)], DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    backend.data_map_entries(registry, map)
}

impl dyn crate::ReloadContext + '_ {
    /// Entry `entry`'s decoded value in data map `map`, for reload hooks such as
    /// `data_maps_updated`.
    pub fn data_map<M: DataMap>(
        &self,
        entry: &M::Entry,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        read_data(entry, &self, map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with two registered maps. The stick's EMC has the wrong NBT type.
    struct Maps {
        emc: [(String, Tag); 2],
        hardness: [(String, Tag); 1],
    }
    impl Maps {
        fn new() -> Self {
            Self {
                emc: [
                    ("test:coal".into(), Tag::Int(10)),
                    ("test:stick".into(), Tag::String("ten".into())),
                ],
                hardness: [("test:stone".into(), Tag::Long(3))],
            }
        }
    }
    impl crate::host::data_maps::DataMapsBackend for Maps {
        fn data_map_value(
            &self,
            registry: &str,
            entry: &str,
            map: &str,
        ) -> Result<Option<&Tag>, DataMapError> {
            Ok(self
                .data_map_entries(registry, map)?
                .iter()
                .find_map(|(id, value)| (id.as_str() == entry).then_some(value)))
        }
        fn data_map_entries(
            &self,
            registry: &str,
            map: &str,
        ) -> Result<&[(String, Tag)], DataMapError> {
            match (registry, map) {
                ("minecraft:item", "test:emc") => Ok(&self.emc),
                ("minecraft:block", "test:hardness") => Ok(&self.hardness),
                _ => Err(DataMapError::UnknownType),
            }
        }
    }
    impl crate::host::keys::KeysBackend for Maps {
        fn item_key_id(&self, key: ItemKey) -> Option<&str> {
            ["test:coal", "test:stick", "test:air"]
                .get(key.0 as usize)
                .copied()
        }
        fn block_key_id(&self, key: BlockKey) -> Option<&str> {
            ["test:stone", "test:dirt"].get(key.0 as usize).copied()
        }
    }
    impl Host for Maps {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            Some(crate::BackendRef::of(self))
        }
    }
    crate::impl_host_areas!(Maps);
    crate::impl_backend_areas!(Maps, except(data_maps, keys));
    impl Backend for Maps {}

    /// No game behind it.
    struct Bare;
    impl Host for Bare {}
    crate::impl_host_areas!(Bare);

    fn number(value: &Tag) -> Option<i64> {
        match value {
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
    struct Emc;
    impl DataMap for Emc {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:emc"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    struct Hardness;
    impl DataMap for Hardness {
        type Entry = BlockKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:hardness"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    /// A map no mod registered.
    struct Unregistered;
    impl DataMap for Unregistered {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:unregistered"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }

    #[test]
    fn a_read_decodes_the_value_and_tells_no_value_from_a_refusal() {
        let maps = Maps::new();
        assert_eq!(ItemKey(0).data(&maps, &Emc), Ok(Some(10)));
        assert_eq!(BlockKey(0).data(&maps, &Hardness), Ok(Some(3)));
        assert_eq!(ItemKey(2).data(&maps, &Emc), Ok(None));
        assert_eq!(BlockKey(1).data(&maps, &Hardness), Ok(None));
        assert_eq!(ItemKey(1).data(&maps, &Emc), Err(DataMapError::Undecodable));
        assert_eq!(ItemKey(3).data(&maps, &Emc), Err(DataMapError::InvalidKey));
        assert_eq!(
            ItemKey(0).data(&maps, &Unregistered),
            Err(DataMapError::UnknownType)
        );
        assert_eq!(ItemKey(0).data(&Bare, &Emc), Err(DataMapError::Unavailable));
        assert_eq!(
            BlockKey(0).data(&Bare, &Hardness),
            Err(DataMapError::Unavailable)
        );
    }

    #[test]
    fn a_maps_type_names_its_entries_registry() {
        assert_eq!(
            Emc.ty(),
            DataMapType {
                id: "test:emc".into(),
                registry: "minecraft:item".into(),
            }
        );
        assert_eq!(Hardness.ty().registry, "minecraft:block");
    }

    #[test]
    fn a_read_by_name_reaches_the_whole_map() {
        let maps = Maps::new();
        let (value, entries) = {
            let registry = String::from("minecraft:block");
            let entry = String::from("test:stone");
            let map = String::from("test:hardness");
            (
                data_map_value(&maps, &registry, &entry, &map),
                data_map_entries(&maps, &registry, &map).unwrap(),
            )
        };
        assert_eq!(value, Ok(Some(&Tag::Long(3))));
        assert_eq!(entries, &[("test:stone".to_owned(), Tag::Long(3))]);
        assert_eq!(
            data_map_entries(&Bare, "minecraft:block", "test:hardness"),
            Err(DataMapError::Unavailable)
        );
    }
}
