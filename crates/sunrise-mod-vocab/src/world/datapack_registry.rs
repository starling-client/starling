//! Datapack registries a native mod declares: a registry
//! of the mod's own, whose entries are JSON files in data packs. The mod
//! declares a record schema once at startup (scalars, and lists, maps and
//! records of them, nested as deep as the schema says); the game decodes every
//! admitted entry against it without running mod code.
use serde_json::Value;
use std::borrow::Cow;
use sunrise_core::identifier::Identifier;

/// How a registry's entries reach joining clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackRegistrySync {
    /// Entries and tags stay on the server; they do not require clients to install the mod.
    #[default]
    ServerOnly,
    /// A joining client must offer the registry with the same schema to receive
    /// it and join. The declaring package must be both-side.
    Required,
    /// Send entries and tags only to clients that offer the registry with the same schema.
    /// This declaration does not require clients to install the mod.
    Optional,
}

/// What a schema field holds, with the bounds the game enforces.
///
/// Recursive over the Wire (with [`DatapackField`], [`DatapackFieldKind`] and [`DatapackValue`]):
/// the generic envelope's depth limit of 32 bounds it, on encode and decode alike, and a schema
/// or value nested past it does not cross (`wire/depth_tests.rs`). A data pack's own JSON is
/// bounded first, by the parser's recursion limit.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackFieldKind {
    /// `true` or `false`.
    Bool,
    /// An integral JSON number in `[min, max]`.
    Int {
        /// The lowest accepted value.
        min: i64,
        /// The highest accepted value.
        max: i64,
    },
    /// A finite JSON number in `[min, max]`; an integral number is accepted.
    Float {
        /// The lowest accepted value.
        min: f64,
        /// The highest accepted value.
        max: f64,
    },
    /// A string of at most `max_len` bytes.
    Text {
        /// The most bytes accepted.
        max_len: u32,
    },
    /// A resource id (`minecraft:` when it names no namespace).
    Identifier,
    /// An id naming an entry of the registry `registry`; the game resolves it
    /// against the whole candidate set, so it may name an entry no pack
    /// has defined yet.
    Reference {
        /// The registry the id names an entry of (`example:spell`).
        registry: String,
    },
    /// A JSON array of `of` values (`Codec.list`, `sizeLimitedListOf`): at least `min_len`
    /// items, with a per-list count limit only when the schema sets `max_len`. The entry and
    /// snapshot budgets still apply.
    List {
        /// What each item holds.
        of: Box<DatapackFieldKind>,
        /// The fewest items accepted.
        min_len: u32,
        /// The most items accepted, if any.
        max_len: Option<u32>,
    },
    /// A JSON object whose keys are `key` and whose values are `value` (`Codec.unboundedMap`),
    /// with the file's pairs in file order (a default's in its declared order), preserved on the
    /// server and on clients.
    Map {
        /// What each key is.
        key: DatapackKeyKind,
        /// What each value holds.
        value: Box<DatapackFieldKind>,
    },
    /// A JSON object with these fields (a nested `RecordCodecBuilder`): required, defaulted and
    /// optional fields follow the same rules as the entry's own.
    Record {
        /// The fields, in the order the decoded record lists them.
        fields: Vec<DatapackField>,
    },
    /// A field that may be left out with no default (`optionalFieldOf(name)`): built by
    /// [`DatapackField::optional`]. An absent one decodes to [`DatapackValue::Absent`], which
    /// `get` reports as `None`.
    Optional {
        /// What the field holds when it is present.
        of: Box<DatapackFieldKind>,
    },
    /// A JSON object whose `key` field names one of `variants`, with that variant's fields beside
    /// it (vanilla's `Codec.dispatch`, `KeyDispatchCodec`). The id reads as `byNameCodec` reads
    /// one (`growing` is `minecraft:growing`); an id no variant declares, or an absent key, fails
    /// the entry. The variants are the declaring mod's alone: a dispatch other mods extend is a
    /// custom registry of variant types, as vanilla's `BLOCK_PREDICATE_TYPE` is.
    Dispatch {
        /// The key naming the variant (`type`).
        key: String,
        /// The variants, each an id and the fields that follow it.
        variants: Vec<DatapackVariant>,
    },
    /// A vanilla `IntProvider` (`IntProviders.codec(min, max)`): an integer constant or a typed
    /// provider (`constant`, `uniform`, `biased_to_bottom`, `very_biased_to_bottom`, `clamped`,
    /// `weighted_list`, `clamped_normal`, `trapezoid`) whose minimum and maximum inclusive bounds
    /// lie in `[min, max]`. Decodes to [`DatapackValue::Vanilla`]
    /// ([`DatapackVanillaValue::IntProvider`]), read with [`DatapackValue::as_int_provider`] and
    /// sampled through [`IntProviderValue::sample`].
    IntProvider {
        /// The lowest permitted provider minimum (`IntProviders.codec(min, max)`).
        min: i32,
        /// The highest permitted provider maximum (`IntProviders.codec(min, max)`).
        max: i32,
    },
    /// A vanilla `BlockStateProvider` (`BlockStateProvider.CODEC`): a full block state object
    /// (`BlockState.FULL_CODEC`), a typed provider (`simple`, `weighted`, `random_block`,
    /// `rotated`, `copy_properties`, `randomized_int`, `rule_based`, `noise`, `noise_threshold`,
    /// `dual_noise`), or a named provider holder in `worldgen/block_state_provider`. Decodes to
    /// [`DatapackValue::Vanilla`] ([`DatapackVanillaValue::BlockStateProvider`]), read with
    /// [`DatapackValue::as_block_state_provider`] and evaluated through
    /// [`BlockStateProviderValue::state`].
    BlockStateProvider,
    /// A vanilla `BlockPredicate` (`BlockPredicate.CODEC`): `true`, `all_of`, `any_of`, `not`,
    /// `matching_blocks`, `matching_block_tag`, `matching_fluids`, `matching_biomes`, `solid`,
    /// `replaceable`, `would_survive`, `has_sturdy_face`, `unobstructed`, `height_range`,
    /// `volume_match`, `inside_world_bounds`. Decodes to [`DatapackValue::Vanilla`]
    /// ([`DatapackVanillaValue::BlockPredicate`]), read with [`DatapackValue::as_block_predicate`]
    /// and evaluated through [`BlockPredicateValue::test`].
    BlockPredicate,
    /// A vanilla `HolderSet` (`RegistryCodecs.homogeneousList`): a single id, a list of ids, or
    /// `#tag` (plus NeoForge compound holder sets). Currently only `minecraft:block` is accepted;
    /// any other registry is refused at registration as
    /// [`DatapackRegistrationError::InvalidFieldKind`]. Decodes to [`DatapackValue::Vanilla`]
    /// ([`DatapackVanillaValue::HolderSet`]), read with [`DatapackValue::as_holder_set`] and
    /// queried through [`HolderSetValue::contains`].
    HolderSet {
        /// The registry whose entries the holder set names (`minecraft:block`).
        registry: String,
    },
}

/// Why reading or evaluating a decoded vanilla datapack value handle failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackValueError {
    /// The handle came from an earlier datapack load (before `/reload` or a client snapshot
    /// replacement), or was never bound to a live load.
    Stale,
    /// No host backend answered, the level handle is stale or unloaded, or a level query failed.
    Unavailable,
    /// The block key is not known to this game.
    UnknownBlock,
    /// Evaluation failed inside the vanilla provider (for example an invalid random bound or
    /// randomized property value).
    EvaluationFailed,
}

impl crate::NoBackend for DatapackValueError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// The engine-bound plumbing every vanilla value handle shares: the unbound handle, its
/// construction from a load generation and slot, and the two reads.
macro_rules! vanilla_value_handle {
    ($handle:ident) => {
        impl $handle {
            pub(crate) const UNBOUND: Self = Self {
                generation: 0,
                index: 0,
            };

            /// Construct a handle for the engine's active decoded table.
            // hidden: only the engine constructs bound handles for its decoded vanilla value table.
            #[doc(hidden)]
            pub const fn from_raw(generation: u64, index: u32) -> Self {
                Self { generation, index }
            }

            /// The load generation this handle belongs to (`0` when unbound).
            // hidden: only the engine checks a handle's load generation against its active table.
            #[doc(hidden)]
            pub const fn generation(self) -> u64 {
                self.generation
            }

            /// The slot index within the load's decoded table.
            // hidden: only the engine indexes its decoded vanilla value table by slot.
            #[doc(hidden)]
            pub const fn index(self) -> u32 {
                self.index
            }
        }
    };
}

/// An opaque handle to a decoded vanilla `IntProvider` (`IntProviders.codec(min, max)`) from one
/// datapack load.
///
/// Copy this out of [`LevelRef::datapack_read`](crate::LevelRef::datapack_read) and call
/// [`Self::sample`]. After `/reload` (or a client snapshot swap), handles from the earlier load
/// return [`DatapackValueError::Stale`] rather than panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: opaque handle with private fields; only the game binds live handles
pub struct IntProviderValue {
    generation: u64,
    index: u32,
}

vanilla_value_handle!(IntProviderValue);

impl IntProviderValue {
    /// Sample one integer from this provider using `random`, in vanilla's draw order
    /// (`IntProvider.sample(RandomSource)`). Works on both sides where the registry syncs.
    pub fn sample(
        self,
        host: &dyn crate::Host,
        random: &mut dyn crate::RandomSource,
    ) -> Result<i32, DatapackValueError> {
        crate::backend_of(host)
            .ok_or(DatapackValueError::Unavailable)?
            .datapack_int_provider_sample(self, random)
    }
}

/// An opaque handle to a decoded vanilla `BlockStateProvider` (`BlockStateProvider.CODEC`) from
/// one datapack load.
///
/// Copy this out of [`LevelRef::datapack_read`](crate::LevelRef::datapack_read) and call
/// [`Self::state`]. After `/reload` (or a client snapshot swap), handles from the earlier load
/// return [`DatapackValueError::Stale`] rather than panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: opaque handle with private fields; only the game binds live handles
pub struct BlockStateProviderValue {
    generation: u64,
    index: u32,
}

vanilla_value_handle!(BlockStateProviderValue);

impl BlockStateProviderValue {
    /// Evaluate this provider at `pos` in `level` using `random`
    /// (`BlockStateProvider.getState(LevelAccessor, RandomSource, BlockPos)`). Works on both
    /// sides where the registry syncs.
    pub fn state(
        self,
        host: &dyn crate::Host,
        level: crate::LevelRef,
        random: &mut dyn crate::RandomSource,
        pos: [i32; 3],
    ) -> Result<crate::BlockState, DatapackValueError> {
        crate::backend_of(host)
            .ok_or(DatapackValueError::Unavailable)?
            .datapack_block_state_provider_state(self, level, random, pos)
    }
}

/// An opaque handle to a decoded vanilla `BlockPredicate` (`BlockPredicate.CODEC`) from one
/// datapack load.
///
/// Copy this out of [`LevelRef::datapack_read`](crate::LevelRef::datapack_read) and call
/// [`Self::test`]. After `/reload` (or a client snapshot swap), handles from the earlier load
/// return [`DatapackValueError::Stale`] rather than panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: opaque handle with private fields; only the game binds live handles
pub struct BlockPredicateValue {
    generation: u64,
    index: u32,
}

vanilla_value_handle!(BlockPredicateValue);

impl BlockPredicateValue {
    /// Test this predicate at `pos` in `level` (`BlockPredicate.test(WorldGenLevel, BlockPos)`).
    /// Works on both sides where the registry syncs.
    pub fn test(
        self,
        host: &dyn crate::Host,
        level: crate::LevelRef,
        pos: [i32; 3],
    ) -> Result<bool, DatapackValueError> {
        crate::backend_of(host)
            .ok_or(DatapackValueError::Unavailable)?
            .datapack_block_predicate_test(self, level, pos)
    }
}

/// An opaque handle to a decoded vanilla `HolderSet` (`RegistryCodecs.homogeneousList`) from one
/// datapack load.
///
/// Copy this out of [`LevelRef::datapack_read`](crate::LevelRef::datapack_read) and call
/// [`Self::contains`]. After `/reload` (or a client snapshot swap), handles from the earlier load
/// return [`DatapackValueError::Stale`] rather than panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: opaque handle with private fields; only the game binds live handles
pub struct HolderSetValue {
    generation: u64,
    index: u32,
}

vanilla_value_handle!(HolderSetValue);

impl HolderSetValue {
    /// Whether `block` is a member of this block holder set (`HolderSet.contains`). Works on both
    /// sides where the registry syncs.
    pub fn contains(
        self,
        host: &dyn crate::Host,
        block: crate::BlockKey,
    ) -> Result<bool, DatapackValueError> {
        crate::backend_of(host)
            .ok_or(DatapackValueError::Unavailable)?
            .datapack_holder_set_contains(self, block)
    }
}

/// Engine-supplied decoder for vanilla datapack value kinds, exported through `__private`
/// for `DatapackCandidate::build` and `rebind_datapack_vanilla_values`.
pub trait VanillaValueDecoder {
    /// Decode `IntProviders.codec(min, max)`.
    fn decode_int_provider(
        &mut self,
        json: &str,
        min: i32,
        max: i32,
    ) -> Result<IntProviderValue, String>;
    /// Decode `BlockStateProvider.CODEC`.
    fn decode_block_state_provider(
        &mut self,
        json: &str,
    ) -> Result<BlockStateProviderValue, String>;
    /// Decode `BlockPredicate.CODEC`.
    fn decode_block_predicate(&mut self, json: &str) -> Result<BlockPredicateValue, String>;
    /// Decode `RegistryCodecs.homogeneousList(registry)` (`minecraft:block`).
    fn decode_holder_set(&mut self, registry: &str, json: &str) -> Result<HolderSetValue, String>;
}

/// A decoder for schemas with no vanilla value kind: every decode is refused.
// exhaustive: a unit marker with no fields to add
pub struct NoVanillaValueDecoder;
impl VanillaValueDecoder for NoVanillaValueDecoder {
    fn decode_int_provider(&mut self, _: &str, _: i32, _: i32) -> Result<IntProviderValue, String> {
        Err("vanilla value kinds require an engine decoder".into())
    }
    fn decode_block_state_provider(&mut self, _: &str) -> Result<BlockStateProviderValue, String> {
        Err("vanilla value kinds require an engine decoder".into())
    }
    fn decode_block_predicate(&mut self, _: &str) -> Result<BlockPredicateValue, String> {
        Err("vanilla value kinds require an engine decoder".into())
    }
    fn decode_holder_set(&mut self, _: &str, _: &str) -> Result<HolderSetValue, String> {
        Err("vanilla value kinds require an engine decoder".into())
    }
}

/// One variant of a [`DatapackFieldKind::Dispatch`]: the id its key names and the record of
/// fields beside the key, with the rules of [`DatapackFieldKind::Record`]'s fields.
///
/// Recursive over the Wire with [`DatapackField`], bounded as [`DatapackFieldKind`] is.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a variant is its id and fields; a mod builds it directly
#[derive(sunrise_mod_api_macros::Wire)]
pub struct DatapackVariant {
    /// The variant's namespaced id (`example:growing_plant`).
    pub id: String,
    /// The fields beside the key, in the order the decoded variant lists them.
    pub fields: Vec<DatapackField>,
}

impl DatapackVariant {
    /// The variant `id` with `fields`.
    pub fn new(id: &str, fields: Vec<DatapackField>) -> Self {
        Self {
            id: id.to_owned(),
            fields,
        }
    }
}

/// What a [`DatapackFieldKind::Map`]'s keys are.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackKeyKind {
    /// A string of at most `max_len` bytes. A zero limit admits only the empty key.
    Text {
        /// The most bytes accepted.
        max_len: u32,
    },
    /// A resource id (`minecraft:` when it names no namespace), normalised to `namespace:path`.
    Identifier,
    /// An id naming an entry of the registry `registry`, resolved like a
    /// [`DatapackFieldKind::Reference`].
    Reference {
        /// The registry the id names an entry of (`example:spell`).
        registry: String,
    },
}

/// A decoded field value. Ids and references are `Text`, normalised to
/// `namespace:path`.
///
/// Recursive over the Wire (with [`DatapackField`], [`DatapackFieldKind`] and [`DatapackValue`]):
/// the generic envelope's depth limit of 32 bounds it, on encode and decode alike, and a schema
/// or value nested past it does not cross (`wire/depth_tests.rs`). A data pack's own JSON is
/// bounded first, by the parser's recursion limit.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackValue {
    /// A [`DatapackFieldKind::Bool`].
    Bool(bool),
    /// A [`DatapackFieldKind::Int`].
    Int(i64),
    /// A [`DatapackFieldKind::Float`].
    Float(f64),
    /// A [`DatapackFieldKind::Text`], [`DatapackFieldKind::Identifier`] or
    /// [`DatapackFieldKind::Reference`].
    Text(String),
    /// A [`DatapackFieldKind::List`], in file order.
    List(Vec<DatapackValue>),
    /// A [`DatapackFieldKind::Map`]: `(key, value)` in file order (a default's in its declared order),
    /// keys normalised as their kind says.
    Map(Vec<(String, DatapackValue)>),
    /// A [`DatapackFieldKind::Record`]: `(field name, value)` in schema order.
    Record(Vec<(String, DatapackValue)>),
    /// A [`DatapackFieldKind::Optional`] field the entry left out; `get` reports it as `None`.
    Absent,
    /// A [`DatapackFieldKind::Dispatch`]: the variant's id, normalised to `namespace:path`, and
    /// its fields' `(field name, value)` in the variant's schema order.
    Variant {
        /// The variant's id.
        id: String,
        /// The variant's fields.
        fields: Vec<(String, DatapackValue)>,
    },
    /// A vanilla codec value ([`DatapackFieldKind::IntProvider`],
    /// [`DatapackFieldKind::BlockStateProvider`], [`DatapackFieldKind::BlockPredicate`] or
    /// [`DatapackFieldKind::HolderSet`]), boxed so recursive `DatapackValue` trees keep a small
    /// stack frame.
    Vanilla(Box<DatapackVanillaValue>),
}

/// Payload of [`DatapackValue::Vanilla`]: the value's canonical JSON text (synced over snapshots
/// and re-decoded on clients) paired with its per-load handle.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackVanillaValue {
    /// A [`DatapackFieldKind::IntProvider`].
    IntProvider {
        /// Canonical JSON text (synced over snapshots and re-decoded on clients).
        json: String,
        /// The handle into this load's decoded table.
        value: IntProviderValue,
    },
    /// A [`DatapackFieldKind::BlockStateProvider`].
    BlockStateProvider {
        /// Canonical JSON text (synced over snapshots and re-decoded on clients).
        json: String,
        /// The handle into this load's decoded table.
        value: BlockStateProviderValue,
    },
    /// A [`DatapackFieldKind::BlockPredicate`].
    BlockPredicate {
        /// Canonical JSON text (synced over snapshots and re-decoded on clients).
        json: String,
        /// The handle into this load's decoded table.
        value: BlockPredicateValue,
    },
    /// A [`DatapackFieldKind::HolderSet`].
    HolderSet {
        /// The registry key (`minecraft:block`).
        registry: String,
        /// Canonical JSON text (synced over snapshots and re-decoded on clients).
        json: String,
        /// The handle into this load's decoded table.
        value: HolderSetValue,
    },
}

impl DatapackValue {
    /// A schema default for [`DatapackFieldKind::IntProvider`], given as JSON text; bound into
    /// each load's decoded table when an entry uses the default.
    pub fn int_provider(json: &str) -> Self {
        Self::Vanilla(Box::new(DatapackVanillaValue::IntProvider {
            json: json.to_owned(),
            value: IntProviderValue::UNBOUND,
        }))
    }

    /// A schema default for [`DatapackFieldKind::BlockStateProvider`], given as JSON text; bound
    /// into each load's decoded table when an entry uses the default.
    pub fn block_state_provider(json: &str) -> Self {
        Self::Vanilla(Box::new(DatapackVanillaValue::BlockStateProvider {
            json: json.to_owned(),
            value: BlockStateProviderValue::UNBOUND,
        }))
    }

    /// A schema default for [`DatapackFieldKind::BlockPredicate`], given as JSON text; bound into
    /// each load's decoded table when an entry uses the default.
    pub fn block_predicate(json: &str) -> Self {
        Self::Vanilla(Box::new(DatapackVanillaValue::BlockPredicate {
            json: json.to_owned(),
            value: BlockPredicateValue::UNBOUND,
        }))
    }

    /// A schema default for [`DatapackFieldKind::HolderSet`], given as a registry key and JSON
    /// text; bound into each load's decoded table when an entry uses the default.
    pub fn holder_set(registry: &str, json: &str) -> Self {
        Self::Vanilla(Box::new(DatapackVanillaValue::HolderSet {
            registry: registry.to_owned(),
            json: json.to_owned(),
            value: HolderSetValue::UNBOUND,
        }))
    }

    /// The handle of a [`DatapackFieldKind::IntProvider`].
    pub fn as_int_provider(&self) -> Option<IntProviderValue> {
        match self {
            Self::Vanilla(vanilla) => match vanilla.as_ref() {
                DatapackVanillaValue::IntProvider { value, .. } => Some(*value),
                _ => None,
            },
            _ => None,
        }
    }

    /// The handle of a [`DatapackFieldKind::BlockStateProvider`].
    pub fn as_block_state_provider(&self) -> Option<BlockStateProviderValue> {
        match self {
            Self::Vanilla(vanilla) => match vanilla.as_ref() {
                DatapackVanillaValue::BlockStateProvider { value, .. } => Some(*value),
                _ => None,
            },
            _ => None,
        }
    }

    /// The handle of a [`DatapackFieldKind::BlockPredicate`].
    pub fn as_block_predicate(&self) -> Option<BlockPredicateValue> {
        match self {
            Self::Vanilla(vanilla) => match vanilla.as_ref() {
                DatapackVanillaValue::BlockPredicate { value, .. } => Some(*value),
                _ => None,
            },
            _ => None,
        }
    }

    /// The handle of a [`DatapackFieldKind::HolderSet`].
    pub fn as_holder_set(&self) -> Option<HolderSetValue> {
        match self {
            Self::Vanilla(vanilla) => match vanilla.as_ref() {
                DatapackVanillaValue::HolderSet { value, .. } => Some(*value),
                _ => None,
            },
            _ => None,
        }
    }
    /// The items of a list.
    pub fn as_list(&self) -> Option<&[DatapackValue]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    /// The `(key, value)` pairs of a map: the file's pairs in file order, or a default's in its
    /// declared order when the file leaves the field out.
    pub fn as_map(&self) -> Option<&[(String, DatapackValue)]> {
        match self {
            Self::Map(pairs) => Some(pairs),
            _ => None,
        }
    }

    /// The id of a [`DatapackFieldKind::Dispatch`]'s variant.
    pub fn variant_id(&self) -> Option<&str> {
        match self {
            Self::Variant { id, .. } => Some(id),
            _ => None,
        }
    }

    /// The field `name` of a record or a variant, or a map's value at the key `name`; `None`
    /// for anything else, and for an optional field the entry left out.
    pub fn get(&self, name: &str) -> Option<&DatapackValue> {
        match self {
            Self::Record(pairs) | Self::Map(pairs) | Self::Variant { fields: pairs, .. } => pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value)
                .filter(|value| **value != Self::Absent),
            _ => None,
        }
    }

    /// The boolean of a [`DatapackFieldKind::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// The integer of a [`DatapackFieldKind::Int`].
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// The number of a [`DatapackFieldKind::Float`].
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// The text of a [`DatapackFieldKind::Text`], an id or a reference.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(v) => Some(v),
            _ => None,
        }
    }
}

/// One field of a registry's entry record.
///
/// Recursive over the Wire (with [`DatapackField`], [`DatapackFieldKind`] and [`DatapackValue`]):
/// the generic envelope's depth limit of 32 bounds it, on encode and decode alike, and a schema
/// or value nested past it does not cross (`wire/depth_tests.rs`). A data pack's own JSON is
/// bounded first, by the parser's recursion limit.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a field is its name, kind and default; a mod builds it directly
#[derive(sunrise_mod_api_macros::Wire)]
pub struct DatapackField {
    /// The JSON key (`power`).
    pub name: String,
    /// What the field holds.
    pub kind: DatapackFieldKind,
    /// The value of an absent field. With `None`, an optional kind gives [`DatapackValue::Absent`]
    /// and any other kind requires the field.
    pub default: Option<DatapackValue>,
}

impl DatapackField {
    /// A required field.
    pub fn required(name: &str, kind: DatapackFieldKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            default: None,
        }
    }

    /// A field that is `default` when an entry leaves it out.
    pub fn with_default(name: &str, kind: DatapackFieldKind, default: DatapackValue) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            default: Some(default),
        }
    }

    /// A field an entry may leave out with no default (`optionalFieldOf(name)`): `get` is
    /// `None` when it is absent.
    pub fn optional(name: &str, kind: DatapackFieldKind) -> Self {
        Self {
            name: name.to_owned(),
            kind: DatapackFieldKind::Optional { of: Box::new(kind) },
            default: None,
        }
    }
}

/// Why a mod's datapack registry declaration was refused
/// ([`Registrar::declare_datapack_registry`](crate::DatapackRegistriesRegistrar::declare_datapack_registry)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackRegistrationError {
    /// A client-only package cannot declare a datapack registry, and a both-side
    /// package cannot declare a required registry for a server-only namespace it serves.
    WrongSide,
    /// The registry's key is malformed or empty.
    InvalidKey,
    /// A built-in registry or another declaration already covers the registry key.
    Duplicate,
    /// A field name is empty or repeated, or a dispatch variant's field is named like the
    /// dispatch's key.
    InvalidFieldName,
    /// A field's kind is unusable: bounds that are not finite or `min > max`,
    /// a scalar text field's `max_len` of 0, a list's `min_len` over its `max_len`, an
    /// optional kind nested in another, a reference to a malformed registry key, a dispatch
    /// with an empty key, no variants, or a variant id that is malformed or repeated, or a
    /// holder set for a registry other than `minecraft:block`, at any depth.
    InvalidFieldKind,
    /// A default is not a value of its field's kind, or is out of its bounds.
    InvalidDefault,
    /// A default's map has two keys naming the same key once ids are normalised.
    DuplicateKey,
}

/// A registry the mod declared, as the game reads it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct DatapackRegistryDeclaration {
    /// The declaring mod's id.
    pub mod_id: String,
    /// The registry's namespaced key (`mymod:spell`).
    pub key: String,
    /// The entry record, in the order the decoded entry lists it.
    pub fields: Vec<DatapackField>,
    /// How clients receive the entries.
    pub sync: DatapackRegistrySync,
}

/// Why an entry file did not decode. `field` is the path of the offending value: the key of a
/// top-level field (`power`), then `.key` into a record or map and `[n]` into a list
/// (`drops[3].item`, `tiers.basic.data_per_kill`), as vanilla's `DataResult` errors name it. A
/// key with any character but ASCII letters, digits, `_`, `:`, `/` and `-` (a `.` or `[`, or an
/// empty key) is written as a quoted JSON string in brackets instead
/// (`tiers["basic.data_per_kill"]`), as the file spells it, so every path reads one way.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: decoding runs in the game; mods read decoded values, never failures
#[non_exhaustive]
pub enum DatapackDecodeError {
    /// The file is not JSON.
    Malformed(String),
    /// The JSON is not an object.
    NotAnObject,
    /// A required field is absent.
    MissingField {
        /// The absent key.
        field: String,
    },
    /// A key the schema does not declare.
    UnknownField {
        /// The undeclared key.
        field: String,
    },
    /// A value of the wrong JSON type.
    WrongType {
        /// The key.
        field: String,
        /// What the schema asked for (`integer`, `string`...).
        expected: &'static str,
    },
    /// A number outside the field's bounds.
    OutOfRange {
        /// The key.
        field: String,
    },
    /// A string over the field's `max_len`.
    TooLong {
        /// The key.
        field: String,
    },
    /// An id that does not parse.
    InvalidIdentifier {
        /// The key.
        field: String,
    },
    /// A list with fewer items than its `min_len` or more than its `max_len`.
    WrongLength {
        /// The list's path.
        field: String,
    },
    /// A map key naming the same key as an earlier one of its map once ids are normalised
    /// (`zombie` after `minecraft:zombie`).
    DuplicateKey {
        /// The repeated key's path, as the file spells it.
        field: String,
    },
    /// A dispatch key naming no variant the schema declares.
    UnknownVariant {
        /// The key's path (`behavior.type`).
        field: String,
        /// The id it names, normalised.
        id: String,
    },
    /// A vanilla value kind (`IntProvider`, `BlockStateProvider`, `BlockPredicate` or
    /// `HolderSet`) whose JSON the engine's vanilla codec refused.
    InvalidVanillaValue {
        /// The offending value's path (`behavior.blocks_to_grow`).
        field: String,
        /// Why the vanilla codec refused it.
        reason: String,
    },
}

/// A decoded entry: one value per schema field, in the schema's order.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game's decoded candidate; a mod reads it through registry handles
// exhaustive: the game builds it with a literal; mods only read it
pub struct DatapackEntry {
    /// `(field name, value)` in schema order.
    pub values: Vec<(String, DatapackValue)>,
}

impl DatapackEntry {
    /// The value of the field `name`; `None` for an optional field the entry left out.
    pub fn get(&self, name: &str) -> Option<&DatapackValue> {
        self.values
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value)
            .filter(|value| **value != DatapackValue::Absent)
    }
}

/// Whether `text` is a lowercase `namespace:path` id the way registrations name theirs.
pub fn valid_namespaced(text: &str) -> bool {
    let Some((namespace, path)) = text.split_once(':') else {
        return false;
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.bytes().all(valid)
        && path.bytes().all(|c| valid(c) || c == b'/')
}

fn finite_bounds(min: f64, max: f64) -> bool {
    min.is_finite() && max.is_finite() && min <= max
}

fn canonical_id(v: &str) -> bool {
    Identifier::parse(v).is_ok_and(|id| id.to_string() == *v)
}

impl DatapackKeyKind {
    fn check(&self) -> bool {
        match self {
            // `max_len: 0` is a schema's own choice: only the empty key.
            Self::Text { .. } | Self::Identifier => true,
            Self::Reference { registry } => valid_namespaced(registry),
        }
    }

    fn accepts(&self, key: &str) -> bool {
        match self {
            Self::Text { max_len } => key.len() <= *max_len as usize,
            Self::Identifier | Self::Reference { .. } => canonical_id(key),
        }
    }
}

impl DatapackFieldKind {
    /// Whether the kind itself is usable, at every depth (`Err` is a
    /// [`DatapackRegistrationError::InvalidFieldKind`], or the nested record's own error).
    fn check(&self) -> Result<(), DatapackRegistrationError> {
        let ok = match self {
            Self::Bool | Self::Identifier => true,
            Self::Int { min, max } => min <= max,
            Self::Float { min, max } => finite_bounds(*min, *max),
            Self::Text { max_len } => *max_len > 0,
            Self::Reference { registry } => valid_namespaced(registry),
            Self::List {
                of,
                min_len,
                max_len,
            } => {
                of.check_nested()?;
                max_len.is_none_or(|max| *min_len <= max)
            }
            Self::Map { key, value } => {
                value.check_nested()?;
                key.check()
            }
            Self::Record { fields } => return check_schema(fields),
            Self::Optional { of } => {
                of.check_nested()?;
                true
            }
            Self::Dispatch { key, variants } => {
                for (index, variant) in variants.iter().enumerate() {
                    if !canonical_id(&variant.id)
                        || variants[..index].iter().any(|seen| seen.id == variant.id)
                    {
                        return Err(DatapackRegistrationError::InvalidFieldKind);
                    }
                    if variant.fields.iter().any(|field| field.name == *key) {
                        return Err(DatapackRegistrationError::InvalidFieldName);
                    }
                    check_schema(&variant.fields)?;
                }
                !key.is_empty() && !variants.is_empty()
            }
            Self::IntProvider { min, max } => min <= max,
            Self::BlockStateProvider | Self::BlockPredicate => true,
            Self::HolderSet { registry } => registry == "minecraft:block",
        };
        if ok {
            Ok(())
        } else {
            Err(DatapackRegistrationError::InvalidFieldKind)
        }
    }

    /// [`Self::check`] for a kind inside a list, a map or an optional field: an optional kind
    /// only stands for a whole field.
    fn check_nested(&self) -> Result<(), DatapackRegistrationError> {
        if matches!(self, Self::Optional { .. }) {
            return Err(DatapackRegistrationError::InvalidFieldKind);
        }
        self.check()
    }

    /// Whether `value` is a value of this kind inside its bounds, at every depth.
    pub(super) fn accepts(&self, value: &DatapackValue) -> bool {
        match (self, value) {
            (Self::Bool, DatapackValue::Bool(_)) => true,
            (Self::Int { min, max }, DatapackValue::Int(v)) => (min..=max).contains(&v),
            (Self::Float { min, max }, DatapackValue::Float(v)) => {
                v.is_finite() && (min..=max).contains(&v)
            }
            (Self::Text { max_len }, DatapackValue::Text(v)) => v.len() <= *max_len as usize,
            (Self::Identifier | Self::Reference { .. }, DatapackValue::Text(v)) => canonical_id(v),
            (
                Self::List {
                    of,
                    min_len,
                    max_len,
                },
                DatapackValue::List(items),
            ) => {
                items.len() >= *min_len as usize
                    && max_len.is_none_or(|max| items.len() <= max as usize)
                    && items.iter().all(|item| of.accepts(item))
            }
            (Self::Map { key, value }, DatapackValue::Map(pairs)) => {
                let mut seen = std::collections::BTreeSet::new();
                pairs
                    .iter()
                    .all(|(k, v)| key.accepts(k) && seen.insert(k.as_str()) && value.accepts(v))
            }
            (Self::Record { fields }, DatapackValue::Record(pairs)) => {
                record_accepts(fields, pairs)
            }
            (Self::Optional { .. }, DatapackValue::Absent) => true,
            (Self::Optional { of }, value) => of.accepts(value),
            (Self::Dispatch { variants, .. }, DatapackValue::Variant { id, fields }) => variants
                .iter()
                .find(|variant| variant.id == *id)
                .is_some_and(|variant| record_accepts(&variant.fields, fields)),
            (Self::IntProvider { .. }, DatapackValue::Vanilla(vanilla)) => matches!(
                vanilla.as_ref(),
                DatapackVanillaValue::IntProvider { json, .. }
                    if serde_json::from_str::<Value>(json).is_ok()
            ),
            (Self::BlockStateProvider, DatapackValue::Vanilla(vanilla)) => matches!(
                vanilla.as_ref(),
                DatapackVanillaValue::BlockStateProvider { json, .. }
                    if serde_json::from_str::<Value>(json).is_ok()
            ),
            (Self::BlockPredicate, DatapackValue::Vanilla(vanilla)) => matches!(
                vanilla.as_ref(),
                DatapackVanillaValue::BlockPredicate { json, .. }
                    if serde_json::from_str::<Value>(json).is_ok()
            ),
            (Self::HolderSet { registry }, DatapackValue::Vanilla(vanilla)) => matches!(
                vanilla.as_ref(),
                DatapackVanillaValue::HolderSet {
                    registry: value_registry,
                    json,
                    ..
                } if value_registry == registry && serde_json::from_str::<Value>(json).is_ok()
            ),
            _ => false,
        }
    }

    fn expected(&self) -> &'static str {
        match self {
            Self::Bool => "boolean",
            Self::Int { .. } => "integer",
            Self::Float { .. } => "number",
            Self::Text { .. } => "string",
            Self::Identifier | Self::Reference { .. } => "identifier string",
            Self::List { .. } => "array",
            Self::Map { .. } | Self::Record { .. } | Self::Dispatch { .. } => "object",
            Self::Optional { of } => of.expected(),
            Self::IntProvider { .. } => "integer or int provider object",
            Self::BlockStateProvider => "block state or block state provider",
            Self::BlockPredicate => "block predicate object",
            Self::HolderSet { .. } => "block id, tag, or list",
        }
    }
}

/// Whether `pairs` are a record of `fields`: each field's value, by name and in schema order.
fn record_accepts(fields: &[DatapackField], pairs: &[(String, DatapackValue)]) -> bool {
    pairs.len() == fields.len()
        && fields
            .iter()
            .zip(pairs)
            .all(|(field, (name, v))| *name == field.name && field.kind.accepts(v))
}

/// Every reference in `value` of `kind`: `(path, registry, target)`, at any depth, so a nested
/// reference resolves against the whole candidate set as a top-level one does.
pub(super) fn references<'v>(
    kind: &'v DatapackFieldKind,
    value: &'v DatapackValue,
    path: &str,
    out: &mut Vec<(String, &'v str, &'v str)>,
) {
    match (kind, value) {
        (DatapackFieldKind::Reference { registry }, DatapackValue::Text(target)) => {
            out.push((path.to_owned(), registry, target));
        }
        (DatapackFieldKind::Optional { of }, value) => references(of, value, path, out),
        (DatapackFieldKind::List { of, .. }, DatapackValue::List(items)) => {
            for (n, item) in items.iter().enumerate() {
                references(of, item, &format!("{path}[{n}]"), out);
            }
        }
        (DatapackFieldKind::Map { key, value }, DatapackValue::Map(pairs)) => {
            for (k, v) in pairs {
                let at = joined(path, k);
                if let DatapackKeyKind::Reference { registry } = key {
                    out.push((at.clone(), registry, k));
                }
                references(value, v, &at, out);
            }
        }
        (DatapackFieldKind::Record { fields }, DatapackValue::Record(pairs)) => {
            for (field, (_, v)) in fields.iter().zip(pairs) {
                references(&field.kind, v, &joined(path, &field.name), out);
            }
        }
        (DatapackFieldKind::Dispatch { variants, .. }, DatapackValue::Variant { id, fields }) => {
            let Some(variant) = variants.iter().find(|variant| variant.id == *id) else {
                return;
            };
            for (field, (_, v)) in variant.fields.iter().zip(fields) {
                references(&field.kind, v, &joined(path, &field.name), out);
            }
        }
        _ => {}
    }
}

/// Checks a schema at registration: field count and names, kinds and defaults, at every depth.
pub(crate) fn check_schema(fields: &[DatapackField]) -> Result<(), DatapackRegistrationError> {
    for (index, field) in fields.iter().enumerate() {
        if field.name.is_empty() || fields[..index].iter().any(|seen| seen.name == field.name) {
            return Err(DatapackRegistrationError::InvalidFieldName);
        }
        field.kind.check()?;
        if matches!(field.kind, DatapackFieldKind::Optional { .. }) && field.default.is_some() {
            // An optional field has no default: `with_default` is the field with one.
            return Err(DatapackRegistrationError::InvalidDefault);
        }
        if field.default.as_ref().is_some_and(repeats_a_key) {
            return Err(DatapackRegistrationError::DuplicateKey);
        }
        if field
            .default
            .as_ref()
            .is_some_and(|default| !field.kind.accepts(default))
        {
            return Err(DatapackRegistrationError::InvalidDefault);
        }
    }
    Ok(())
}

/// Whether a map anywhere in `value` has a key twice.
fn repeats_a_key(value: &DatapackValue) -> bool {
    match value {
        DatapackValue::List(items) => items.iter().any(repeats_a_key),
        DatapackValue::Record(pairs) | DatapackValue::Variant { fields: pairs, .. } => {
            pairs.iter().any(|(_, v)| repeats_a_key(v))
        }
        DatapackValue::Map(pairs) => {
            let mut seen = std::collections::BTreeSet::new();
            pairs
                .iter()
                .any(|(k, v)| !seen.insert(k.as_str()) || repeats_a_key(v))
        }
        _ => false,
    }
}

/// Normalises the schema's id and reference defaults at every depth, as `Identifier.parse`
/// reads them (`stone` is `minecraft:stone`): a default's own value, the items of a list, a
/// record's fields, a map's id and reference keys and its values, and the defaults of nested
/// records' fields. A default that is no id is an `InvalidDefault`; a default that does not fit
/// its kind is left for [`check_schema`].
pub(crate) fn normalize_defaults(
    fields: &mut [DatapackField],
) -> Result<(), DatapackRegistrationError> {
    for field in fields {
        normalize_kind(&mut field.kind)?;
        if let Some(default) = &mut field.default {
            normalize_value(&field.kind, default)?;
        }
    }
    Ok(())
}

fn normalize_kind(kind: &mut DatapackFieldKind) -> Result<(), DatapackRegistrationError> {
    match kind {
        DatapackFieldKind::List { of, .. } | DatapackFieldKind::Optional { of } => {
            normalize_kind(of)
        }
        DatapackFieldKind::Map { value, .. } => normalize_kind(value),
        DatapackFieldKind::Record { fields } => normalize_defaults(fields),
        DatapackFieldKind::Dispatch { variants, .. } => variants
            .iter_mut()
            .try_for_each(|variant| normalize_defaults(&mut variant.fields)),
        _ => Ok(()),
    }
}

fn normalize_id(value: &mut String) -> Result<(), DatapackRegistrationError> {
    *value = Identifier::parse(value)
        .map_err(|_| DatapackRegistrationError::InvalidDefault)?
        .to_string();
    Ok(())
}

fn normalize_value(
    kind: &DatapackFieldKind,
    value: &mut DatapackValue,
) -> Result<(), DatapackRegistrationError> {
    match (kind, value) {
        (
            DatapackFieldKind::Identifier | DatapackFieldKind::Reference { .. },
            DatapackValue::Text(id),
        ) => normalize_id(id),
        (DatapackFieldKind::List { of, .. }, DatapackValue::List(items)) => items
            .iter_mut()
            .try_for_each(|item| normalize_value(of, item)),
        (DatapackFieldKind::Map { key, value }, DatapackValue::Map(pairs)) => {
            pairs.iter_mut().try_for_each(|(k, v)| {
                if !matches!(key, DatapackKeyKind::Text { .. }) {
                    normalize_id(k)?;
                }
                normalize_value(value, v)
            })
        }
        (DatapackFieldKind::Record { fields }, DatapackValue::Record(pairs)) => fields
            .iter()
            .zip(pairs.iter_mut())
            .try_for_each(|(field, (_, v))| normalize_value(&field.kind, v)),
        (DatapackFieldKind::Optional { .. }, DatapackValue::Absent) => Ok(()),
        (DatapackFieldKind::Optional { of }, value) => normalize_value(of, value),
        (DatapackFieldKind::Dispatch { variants, .. }, DatapackValue::Variant { id, fields }) => {
            normalize_id(id)?;
            let Some(variant) = variants.iter().find(|variant| variant.id == *id) else {
                return Ok(());
            };
            variant
                .fields
                .iter()
                .zip(fields.iter_mut())
                .try_for_each(|(field, (_, v))| normalize_value(&field.kind, v))
        }
        (DatapackFieldKind::IntProvider { .. }, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::IntProvider { json, value } = vanilla.as_mut() {
                canonicalize_json_default(json)?;
                *value = IntProviderValue::UNBOUND;
            }
            Ok(())
        }
        (DatapackFieldKind::BlockStateProvider, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::BlockStateProvider { json, value } = vanilla.as_mut() {
                canonicalize_json_default(json)?;
                *value = BlockStateProviderValue::UNBOUND;
            }
            Ok(())
        }
        (DatapackFieldKind::BlockPredicate, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::BlockPredicate { json, value } = vanilla.as_mut() {
                canonicalize_json_default(json)?;
                *value = BlockPredicateValue::UNBOUND;
            }
            Ok(())
        }
        (DatapackFieldKind::HolderSet { .. }, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::HolderSet { json, value, .. } = vanilla.as_mut() {
                canonicalize_json_default(json)?;
                *value = HolderSetValue::UNBOUND;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn canonicalize_json_default(json: &mut String) -> Result<(), DatapackRegistrationError> {
    let parsed: Json =
        serde_json::from_str(json).map_err(|_| DatapackRegistrationError::InvalidDefault)?;
    *json = parsed.to_canonical_json();
    Ok(())
}

/// Whether any field in `fields` (at any depth) uses a vanilla value kind.
pub(super) fn schema_has_vanilla_kind(fields: &[DatapackField]) -> bool {
    fields.iter().any(|field| kind_has_vanilla(&field.kind))
}

fn kind_has_vanilla(kind: &DatapackFieldKind) -> bool {
    match kind {
        DatapackFieldKind::IntProvider { .. }
        | DatapackFieldKind::BlockStateProvider
        | DatapackFieldKind::BlockPredicate
        | DatapackFieldKind::HolderSet { .. } => true,
        DatapackFieldKind::List { of, .. } | DatapackFieldKind::Optional { of } => {
            kind_has_vanilla(of)
        }
        DatapackFieldKind::Map { value, .. } => kind_has_vanilla(value),
        DatapackFieldKind::Record { fields } => schema_has_vanilla_kind(fields),
        DatapackFieldKind::Dispatch { variants, .. } => variants
            .iter()
            .any(|variant| schema_has_vanilla_kind(&variant.fields)),
        _ => false,
    }
}

/// Decode (or re-decode) every vanilla value inside `value` against `decoder`, binding its
/// handle to `decoder`'s generation.
pub(super) fn bind_vanilla_values(
    kind: &DatapackFieldKind,
    value: &mut DatapackValue,
    path: &str,
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<(), DatapackDecodeError> {
    match (kind, value) {
        (DatapackFieldKind::IntProvider { min, max }, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::IntProvider { json, value } = vanilla.as_mut() {
                *value = decoder
                    .decode_int_provider(json, *min, *max)
                    .map_err(|reason| DatapackDecodeError::InvalidVanillaValue {
                        field: path.to_owned(),
                        reason,
                    })?;
            }
            Ok(())
        }
        (DatapackFieldKind::BlockStateProvider, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::BlockStateProvider { json, value } = vanilla.as_mut() {
                *value = decoder
                    .decode_block_state_provider(json)
                    .map_err(|reason| DatapackDecodeError::InvalidVanillaValue {
                        field: path.to_owned(),
                        reason,
                    })?;
            }
            Ok(())
        }
        (DatapackFieldKind::BlockPredicate, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::BlockPredicate { json, value } = vanilla.as_mut() {
                *value = decoder.decode_block_predicate(json).map_err(|reason| {
                    DatapackDecodeError::InvalidVanillaValue {
                        field: path.to_owned(),
                        reason,
                    }
                })?;
            }
            Ok(())
        }
        (DatapackFieldKind::HolderSet { registry }, DatapackValue::Vanilla(vanilla)) => {
            if let DatapackVanillaValue::HolderSet { json, value, .. } = vanilla.as_mut() {
                *value = decoder
                    .decode_holder_set(registry, json)
                    .map_err(|reason| DatapackDecodeError::InvalidVanillaValue {
                        field: path.to_owned(),
                        reason,
                    })?;
            }
            Ok(())
        }
        (DatapackFieldKind::Optional { .. }, DatapackValue::Absent) => Ok(()),
        (DatapackFieldKind::Optional { of }, value) => {
            bind_vanilla_values(of, value, path, decoder)
        }
        (DatapackFieldKind::List { of, .. }, DatapackValue::List(items)) => {
            items.iter_mut().enumerate().try_for_each(|(n, item)| {
                bind_vanilla_values(of, item, &format!("{path}[{n}]"), decoder)
            })
        }
        (DatapackFieldKind::Map { value: kind, .. }, DatapackValue::Map(pairs)) => pairs
            .iter_mut()
            .try_for_each(|(k, v)| bind_vanilla_values(kind, v, &joined(path, k), decoder)),
        (DatapackFieldKind::Record { fields }, DatapackValue::Record(pairs)) => fields
            .iter()
            .zip(pairs.iter_mut())
            .try_for_each(|(field, (_, v))| {
                bind_vanilla_values(&field.kind, v, &joined(path, &field.name), decoder)
            }),
        (DatapackFieldKind::Dispatch { variants, .. }, DatapackValue::Variant { id, fields }) => {
            let Some(variant) = variants.iter().find(|variant| variant.id == *id) else {
                return Ok(());
            };
            variant
                .fields
                .iter()
                .zip(fields.iter_mut())
                .try_for_each(|(field, (_, v))| {
                    bind_vanilla_values(&field.kind, v, &joined(path, &field.name), decoder)
                })
        }
        _ => Ok(()),
    }
}

/// Clear every vanilla value handle inside `value` back to `UNBOUND` before wire serialization,
/// so snapshots carry only canonical JSON and clients re-bind against their own tables.
pub(super) fn unbind_vanilla_values(value: &mut DatapackValue) {
    match value {
        DatapackValue::Vanilla(vanilla) => match vanilla.as_mut() {
            DatapackVanillaValue::IntProvider { value, .. } => *value = IntProviderValue::UNBOUND,
            DatapackVanillaValue::BlockStateProvider { value, .. } => {
                *value = BlockStateProviderValue::UNBOUND;
            }
            DatapackVanillaValue::BlockPredicate { value, .. } => {
                *value = BlockPredicateValue::UNBOUND;
            }
            DatapackVanillaValue::HolderSet { value, .. } => *value = HolderSetValue::UNBOUND,
        },
        DatapackValue::List(items) => items.iter_mut().for_each(unbind_vanilla_values),
        DatapackValue::Map(pairs)
        | DatapackValue::Record(pairs)
        | DatapackValue::Variant { fields: pairs, .. } => {
            pairs.iter_mut().for_each(|(_, v)| unbind_vanilla_values(v));
        }
        _ => {}
    }
}

/// NeoForge's `Registries.registryDirPath` (`CommonHooks.prefixNamespace`),
/// relative to the entry's namespace directory.
pub fn datapack_registry_directory(registry: &str) -> Option<Cow<'_, str>> {
    let (namespace, path) = registry.split_once(':')?;
    Some(if namespace == "minecraft" {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(format!("{namespace}/{path}"))
    })
}

impl DatapackRegistryDeclaration {
    /// The file an entry (`addon:fire`) sits in, inside a data pack. For
    /// `example:spell` it is `data/addon/example/spell/fire.json`; for
    /// `minecraft:spell` it is `data/addon/spell/fire.json`. Tags use the same
    /// registry directory under `tags/`, and nested paths keep their subfolders.
    pub fn entry_file(&self, entry: &str) -> Option<String> {
        let (entry_namespace, entry_path) = entry.split_once(':')?;
        let directory = datapack_registry_directory(&self.key)?;
        Some(format!(
            "data/{entry_namespace}/{directory}/{entry_path}.json"
        ))
    }

    /// Decodes one entry file against the schema. A key the schema does not
    /// declare, a missing required key, a wrong type or an out-of-range value
    /// fails the whole entry, at any depth, and no default stands in for invalid
    /// data. The error names the value's full path.
    /// Pass the bare schema object, without load condition metadata. This method neither
    /// runs mod code nor applies a network snapshot limit. Map pairs keep their file order.
    pub fn decode_entry(&self, text: &str) -> Result<DatapackEntry, DatapackDecodeError> {
        let json: Json = serde_json::from_str(text)
            .map_err(|error| DatapackDecodeError::Malformed(error.to_string()))?;
        let object = json.as_object().ok_or(DatapackDecodeError::NotAnObject)?;
        decode_fields(
            &self.fields,
            object,
            RecordMetadata::None,
            "",
            &mut NoVanillaValueDecoder,
        )
        .map(|values| DatapackEntry { values })
    }
}

/// `key` of the value at `path`, as [`DatapackDecodeError`]'s paths write it: `.key`, or
/// `["key"]` for a key with any other character than `[A-Za-z0-9_:/-]` (or none).
pub(super) fn joined(path: &str, key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_:/-".contains(&c));
    match (plain, path.is_empty()) {
        (true, true) => key.to_owned(),
        (true, false) => format!("{path}.{key}"),
        (false, _) => format!("{path}[{}]", Value::String(key.to_owned())),
    }
}

/// A JSON value as the file wrote it. An object keeps its keys in file order, as Gson's
/// `JsonObject` does: a key written twice keeps its first place and its last value.
#[derive(Clone, Debug)]
enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// A parsed `serde_json` value (a configured feature's), its objects in key order.
    fn of(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(*b),
            Value::Number(n) => Self::Number(n.clone()),
            Value::String(s) => Self::String(s.clone()),
            Value::Array(items) => Self::Array(items.iter().map(Self::of).collect()),
            Value::Object(object) => Self::Object(
                object
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::of(v)))
                    .collect(),
            ),
        }
    }
    fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(n) => n.as_i64(),
            _ => None,
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => n.as_f64(),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    fn as_array(&self) -> Option<&[Json]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }
    fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Self::Object(pairs) => Some(pairs),
            _ => None,
        }
    }
    /// Compact canonical JSON text, preserving object key order.
    fn to_canonical_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }
    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Number(n) => out.push_str(&n.to_string()),
            Self::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            Self::Object(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push(':');
                    v.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for Json {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Json;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
                Ok(Json::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
                Ok(Json::Number(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
                Ok(Json::Number(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Json, E> {
                serde_json::Number::from_f64(v)
                    .map(Json::Number)
                    .ok_or_else(|| E::custom("a number that is not finite"))
            }
            fn visit_str<E>(self, v: &str) -> Result<Json, E> {
                Ok(Json::String(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Json, E> {
                Ok(Json::String(v))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Json::Array(items))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
                let mut pairs: Vec<(String, Json)> = Vec::new();
                let mut at: std::collections::HashMap<String, usize> =
                    std::collections::HashMap::new();
                while let Some((key, value)) = map.next_entry::<String, Json>()? {
                    match at.get(&key) {
                        Some(&n) => pairs[n].1 = value,
                        None => {
                            at.insert(key.clone(), pairs.len());
                            pairs.push((key, value));
                        }
                    }
                }
                Ok(Json::Object(pairs))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Decode a resource whose load conditions have already passed, following
/// ConditionalDecoder: unwrap `neoforge:value` or hide the outer conditions key. `decoder` is
/// the engine's decoder for vanilla value kinds ([`NoVanillaValueDecoder`] when the schema has
/// none).
pub fn decode_datapack_entry(
    declaration: &DatapackRegistryDeclaration,
    text: &str,
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<DatapackEntry, DatapackDecodeError> {
    let json: Json = serde_json::from_str(text)
        .map_err(|error| DatapackDecodeError::Malformed(error.to_string()))?;
    let object = json.as_object().ok_or(DatapackDecodeError::NotAnObject)?;
    let (object, metadata) = if !object.iter().any(|(key, _)| key == "neoforge:conditions") {
        (object, RecordMetadata::None)
    } else if let Some((_, inner)) = object.iter().find(|(key, _)| key == "neoforge:value") {
        (
            inner.as_object().ok_or(DatapackDecodeError::NotAnObject)?,
            RecordMetadata::None,
        )
    } else {
        (object, RecordMetadata::DatapackConditions)
    };
    decode_fields(&declaration.fields, object, metadata, "", decoder)
        .map(|values| DatapackEntry { values })
}

pub(crate) enum RecordMetadata<'a> {
    None,
    /// The feature dispatch key is allowed, and a schema can also read it.
    FeatureType,
    /// The inlined condition wrapper's key is not part of the schema input.
    DatapackConditions,
    /// A dispatched variant's fields sit beside the key that named the variant.
    DispatchKey(&'a str),
}

/// Decodes a JSON object against a flat record schema. An undeclared key (other than
/// the caller's metadata), a missing required key, a wrong type or an out-of-range
/// value fails the whole record. Nothing here runs mod code.
pub(crate) fn decode_record(
    fields: &[DatapackField],
    object: &serde_json::Map<String, Value>,
    metadata: RecordMetadata<'_>,
) -> Result<DatapackEntry, DatapackDecodeError> {
    let object: Vec<_> = object
        .iter()
        .map(|(k, v)| (k.clone(), Json::of(v)))
        .collect();
    decode_fields(fields, &object, metadata, "", &mut NoVanillaValueDecoder)
        .map(|values| DatapackEntry { values })
}

/// A record's fields at `path` (empty for an entry's own).
fn decode_fields(
    fields: &[DatapackField],
    object: &[(String, Json)],
    metadata: RecordMetadata<'_>,
    path: &str,
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<Vec<(String, DatapackValue)>, DatapackDecodeError> {
    let (extra, hidden) = match metadata {
        RecordMetadata::None => (None, None),
        RecordMetadata::FeatureType => (Some("type"), None),
        RecordMetadata::DatapackConditions => {
            (Some("neoforge:conditions"), Some("neoforge:conditions"))
        }
        RecordMetadata::DispatchKey(key) => (Some(key), None),
    };
    if let Some((unknown, _)) = object.iter().find(|(key, _)| {
        Some(key.as_str()) != extra && !fields.iter().any(|field| field.name == *key)
    }) {
        return Err(DatapackDecodeError::UnknownField {
            field: joined(path, unknown),
        });
    }
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        let at = joined(path, &field.name);
        let json = object.iter().find_map(|(key, json)| {
            (*key == field.name && Some(key.as_str()) != hidden).then_some(json)
        });
        let value = match (json, &field.kind) {
            (Some(json), kind) => decode_value(kind, json, &at, decoder)?,
            (None, DatapackFieldKind::Optional { .. }) => DatapackValue::Absent,
            (None, kind) => {
                let mut value = field
                    .default
                    .clone()
                    .ok_or_else(|| DatapackDecodeError::MissingField { field: at.clone() })?;
                bind_vanilla_values(kind, &mut value, &at, decoder)?;
                value
            }
        };
        values.push((field.name.clone(), value));
    }
    Ok(values)
}

/// A map's key `key`, at `at` (the key's own path, as the file spells it).
fn decode_key(kind: &DatapackKeyKind, key: &str, at: &str) -> Result<String, DatapackDecodeError> {
    match kind {
        DatapackKeyKind::Text { max_len } if key.len() > *max_len as usize => {
            Err(DatapackDecodeError::TooLong { field: at.into() })
        }
        DatapackKeyKind::Text { .. } => Ok(key.to_owned()),
        DatapackKeyKind::Identifier | DatapackKeyKind::Reference { .. } => Identifier::parse(key)
            .map(|id| id.to_string())
            .map_err(|_| DatapackDecodeError::InvalidIdentifier { field: at.into() }),
    }
}

fn decode_value(
    kind: &DatapackFieldKind,
    json: &Json,
    path: &str,
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<DatapackValue, DatapackDecodeError> {
    let name = || path.to_owned();
    let wrong = || DatapackDecodeError::WrongType {
        field: name(),
        expected: kind.expected(),
    };
    let out_of_range = || DatapackDecodeError::OutOfRange { field: name() };
    match kind {
        DatapackFieldKind::Bool => json.as_bool().map(DatapackValue::Bool).ok_or_else(wrong),
        DatapackFieldKind::Int { min, max } => {
            // `as_i64` refuses 3.0 and anything past i64: vanilla's integer codec does too.
            let v = json.as_i64().ok_or_else(wrong)?;
            if (min..=max).contains(&&v) {
                Ok(DatapackValue::Int(v))
            } else {
                Err(out_of_range())
            }
        }
        DatapackFieldKind::Float { min, max } => {
            let v = json.as_f64().filter(|v| v.is_finite()).ok_or_else(wrong)?;
            if (min..=max).contains(&&v) {
                Ok(DatapackValue::Float(v))
            } else {
                Err(out_of_range())
            }
        }
        DatapackFieldKind::Text { max_len } => {
            let v = json.as_str().ok_or_else(wrong)?;
            if v.len() > *max_len as usize {
                Err(DatapackDecodeError::TooLong { field: name() })
            } else {
                Ok(DatapackValue::Text(v.to_owned()))
            }
        }
        DatapackFieldKind::Identifier | DatapackFieldKind::Reference { .. } => {
            let v = json.as_str().ok_or_else(wrong)?;
            Identifier::parse(v)
                .map(|id| DatapackValue::Text(id.to_string()))
                .map_err(|_| DatapackDecodeError::InvalidIdentifier { field: name() })
        }
        DatapackFieldKind::List {
            of,
            min_len,
            max_len,
        } => {
            let items = json.as_array().ok_or_else(wrong)?;
            if items.len() < *min_len as usize
                || max_len.is_some_and(|max| items.len() > max as usize)
            {
                return Err(DatapackDecodeError::WrongLength { field: name() });
            }
            items
                .iter()
                .enumerate()
                .map(|(n, item)| decode_value(of, item, &format!("{path}[{n}]"), decoder))
                .collect::<Result<_, _>>()
                .map(DatapackValue::List)
        }
        DatapackFieldKind::Map { key, value } => {
            let object = json.as_object().ok_or_else(wrong)?;
            // The pairs in file order; ids normalise (`fire` is `minecraft:fire`), and two keys
            // naming the same one are a duplicate.
            let mut pairs = Vec::with_capacity(object.len());
            let mut seen = std::collections::BTreeSet::new();
            for (raw, v) in object {
                let at = joined(path, raw);
                let k = decode_key(key, raw, &at)?;
                if !seen.insert(k.clone()) {
                    return Err(DatapackDecodeError::DuplicateKey { field: at });
                }
                pairs.push((k, decode_value(value, v, &at, decoder)?));
            }
            Ok(DatapackValue::Map(pairs))
        }
        DatapackFieldKind::Record { fields } => {
            let object = json.as_object().ok_or_else(wrong)?;
            decode_fields(fields, object, RecordMetadata::None, path, decoder)
                .map(DatapackValue::Record)
        }
        DatapackFieldKind::Optional { of } => decode_value(of, json, path, decoder),
        DatapackFieldKind::Dispatch { key, variants } => {
            let object = json.as_object().ok_or_else(wrong)?;
            let at = joined(path, key);
            let named = object
                .iter()
                .find_map(|(k, v)| (k == key).then_some(v))
                .ok_or_else(|| DatapackDecodeError::MissingField { field: at.clone() })?;
            let named = named
                .as_str()
                .ok_or_else(|| DatapackDecodeError::WrongType {
                    field: at.clone(),
                    expected: "identifier string",
                })?;
            let id = Identifier::parse(named)
                .map_err(|_| DatapackDecodeError::InvalidIdentifier { field: at.clone() })?
                .to_string();
            let Some(variant) = variants.iter().find(|variant| variant.id == id) else {
                return Err(DatapackDecodeError::UnknownVariant { field: at, id });
            };
            let fields = decode_fields(
                &variant.fields,
                object,
                RecordMetadata::DispatchKey(key),
                path,
                decoder,
            )?;
            Ok(DatapackValue::Variant { id, fields })
        }
        DatapackFieldKind::IntProvider { min, max } => {
            let text = json.to_canonical_json();
            let value = decoder
                .decode_int_provider(&text, *min, *max)
                .map_err(|reason| DatapackDecodeError::InvalidVanillaValue {
                    field: name(),
                    reason,
                })?;
            Ok(DatapackValue::Vanilla(Box::new(
                DatapackVanillaValue::IntProvider { json: text, value },
            )))
        }
        DatapackFieldKind::BlockStateProvider => {
            let text = json.to_canonical_json();
            let value = decoder
                .decode_block_state_provider(&text)
                .map_err(|reason| DatapackDecodeError::InvalidVanillaValue {
                    field: name(),
                    reason,
                })?;
            Ok(DatapackValue::Vanilla(Box::new(
                DatapackVanillaValue::BlockStateProvider { json: text, value },
            )))
        }
        DatapackFieldKind::BlockPredicate => {
            let text = json.to_canonical_json();
            let value = decoder.decode_block_predicate(&text).map_err(|reason| {
                DatapackDecodeError::InvalidVanillaValue {
                    field: name(),
                    reason,
                }
            })?;
            Ok(DatapackValue::Vanilla(Box::new(
                DatapackVanillaValue::BlockPredicate { json: text, value },
            )))
        }
        DatapackFieldKind::HolderSet { registry } => {
            let text = json.to_canonical_json();
            let value = decoder
                .decode_holder_set(registry, &text)
                .map_err(|reason| DatapackDecodeError::InvalidVanillaValue {
                    field: name(),
                    reason,
                })?;
            Ok(DatapackValue::Vanilla(Box::new(
                DatapackVanillaValue::HolderSet {
                    registry: registry.clone(),
                    json: text,
                    value,
                },
            )))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `example:spell`: `{power: integer 0..100, element: id, chain: reference, silent: bool = false}`.
    fn spell() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![
                DatapackField::required("power", DatapackFieldKind::Int { min: 0, max: 100 }),
                DatapackField::with_default(
                    "element",
                    DatapackFieldKind::Identifier,
                    DatapackValue::Text("example:fire".into()),
                ),
                DatapackField::with_default(
                    "silent",
                    DatapackFieldKind::Bool,
                    DatapackValue::Bool(false),
                ),
                DatapackField::with_default(
                    "next",
                    DatapackFieldKind::Reference {
                        registry: "example:spell".into(),
                    },
                    DatapackValue::Text("example:fire".into()),
                ),
            ],
            sync: DatapackRegistrySync::ServerOnly,
        }
    }

    #[test]
    fn entry_files_follow_neoforges_namespaced_registry_folder() {
        assert_eq!(
            spell().entry_file("addon:fire").as_deref(),
            Some("data/addon/example/spell/fire.json")
        );
        let vanilla = DatapackRegistryDeclaration {
            key: "minecraft:magic/spell".into(),
            ..spell()
        };
        assert_eq!(
            vanilla.entry_file("addon:nested/fire").as_deref(),
            Some("data/addon/magic/spell/nested/fire.json")
        );
        assert_eq!(spell().entry_file("nocolon"), None);
    }

    #[test]
    fn a_record_decodes_in_schema_order_with_defaults() {
        let entry = spell()
            .decode_entry(r#"{"power": 7, "next": "ice"}"#)
            .unwrap();
        assert_eq!(
            entry.values,
            vec![
                ("power".to_owned(), DatapackValue::Int(7)),
                (
                    "element".to_owned(),
                    DatapackValue::Text("example:fire".into())
                ),
                ("silent".to_owned(), DatapackValue::Bool(false)),
                // An id with no namespace is vanilla's, as everywhere else.
                (
                    "next".to_owned(),
                    DatapackValue::Text("minecraft:ice".into())
                ),
            ]
        );
        assert_eq!(entry.get("power"), Some(&DatapackValue::Int(7)));
    }

    #[test]
    fn invalid_data_fails_the_entry_and_names_the_key() {
        let decl = spell();
        let err = |text: &str| decl.decode_entry(text).unwrap_err();
        assert_eq!(
            err(r#"{"power": 101}"#),
            DatapackDecodeError::OutOfRange {
                field: "power".into()
            }
        );
        assert_eq!(
            err(r#"{"power": -1}"#),
            DatapackDecodeError::OutOfRange {
                field: "power".into()
            }
        );
        // 3.0 is a float, not the integer the schema asks for.
        assert_eq!(
            err(r#"{"power": 3.0}"#),
            DatapackDecodeError::WrongType {
                field: "power".into(),
                expected: "integer"
            }
        );
        assert_eq!(
            err(r#"{"power": "7"}"#),
            DatapackDecodeError::WrongType {
                field: "power".into(),
                expected: "integer"
            }
        );
        assert_eq!(
            err(r#"{"silent": true}"#),
            DatapackDecodeError::MissingField {
                field: "power".into()
            }
        );
        assert_eq!(
            err(r#"{"power": 1, "powr": 2}"#),
            DatapackDecodeError::UnknownField {
                field: "powr".into()
            }
        );
        assert_eq!(
            err(r#"{"power": 1, "element": "Not An Id"}"#),
            DatapackDecodeError::InvalidIdentifier {
                field: "element".into()
            }
        );
        // A boolean field takes only `true` or `false`.
        for wrong in [r#""yes""#, "1", "null"] {
            assert_eq!(
                err(&format!(r#"{{"power": 1, "silent": {wrong}}}"#)),
                DatapackDecodeError::WrongType {
                    field: "silent".into(),
                    expected: "boolean"
                }
            );
        }
        assert_eq!(err("[1]"), DatapackDecodeError::NotAnObject);
        assert!(matches!(err("{"), DatapackDecodeError::Malformed(_)));
        // A nested value is never a scalar field.
        assert!(matches!(
            err(r#"{"power": {"a": 1}}"#),
            DatapackDecodeError::WrongType { .. }
        ));
    }

    #[test]
    fn wrapper_metadata_and_configured_feature_dispatch_have_distinct_visibility() {
        use crate::world::feature_type::decode_feature_config;
        use serde_json::json;

        let declaration = DatapackRegistryDeclaration {
            fields: vec![DatapackField::with_default(
                "neoforge:conditions",
                DatapackFieldKind::Bool,
                DatapackValue::Bool(false),
            )],
            ..spell()
        };
        let wrapped = json!({"neoforge:conditions":[{"type":"neoforge:always"}]});
        assert_eq!(
            decode_datapack_entry(
                &declaration,
                &wrapped.to_string(),
                &mut NoVanillaValueDecoder
            )
            .unwrap()
            .get("neoforge:conditions"),
            Some(&DatapackValue::Bool(false))
        );

        let feature = json!({"type":"example:crystal"});
        assert!(decode_feature_config(&[], &feature).is_ok());
        let schema = [DatapackField::required(
            "type",
            DatapackFieldKind::Identifier,
        )];
        assert_eq!(
            decode_feature_config(&schema, &feature)
                .unwrap()
                .get("type"),
            Some(&DatapackValue::Text("example:crystal".into()))
        );
    }

    #[test]
    fn nesting_is_bounded_by_the_json_parser() {
        let decl = spell();
        let deep = format!("{}1{}", "[".repeat(65), "]".repeat(65));
        assert!(matches!(
            decl.decode_entry(&deep),
            Err(DatapackDecodeError::NotAnObject)
        ));
        let very_deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
        assert!(matches!(
            decl.decode_entry(&very_deep),
            Err(DatapackDecodeError::Malformed(_))
        ));
    }

    #[test]
    fn floats_and_text_obey_their_bounds() {
        let decl = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:aura".into(),
            fields: vec![
                DatapackField::required("scale", DatapackFieldKind::Float { min: 0.0, max: 2.0 }),
                DatapackField::required("name", DatapackFieldKind::Text { max_len: 4 }),
            ],
            sync: DatapackRegistrySync::Optional,
        };
        // An integral number is a fine float.
        assert!(decl.decode_entry(r#"{"scale": 2, "name": "abcd"}"#).is_ok());
        assert_eq!(
            decl.decode_entry(r#"{"scale": 2.5, "name": "a"}"#)
                .unwrap_err(),
            DatapackDecodeError::OutOfRange {
                field: "scale".into()
            }
        );
        // 1e999 is not a JSON number serde_json yields: refused as malformed, never infinity.
        assert!(matches!(
            decl.decode_entry(r#"{"scale": 1e999, "name": "a"}"#),
            Err(DatapackDecodeError::Malformed(_))
        ));
        assert_eq!(
            decl.decode_entry(r#"{"scale": 1, "name": "abcde"}"#)
                .unwrap_err(),
            DatapackDecodeError::TooLong {
                field: "name".into()
            }
        );
    }

    #[test]
    fn schemas_are_checked_at_registration() {
        use DatapackRegistrationError as E;
        let int = || DatapackFieldKind::Int { min: 0, max: 1 };
        assert_eq!(check_schema(&spell().fields), Ok(()));
        let named = |name: &str| vec![DatapackField::required(name, int())];
        assert_eq!(check_schema(&named("")), Err(E::InvalidFieldName));
        // Any other key is a field, as vanilla's codecs take any JSON key.
        assert_eq!(check_schema(&named("Power")), Ok(()));
        assert_eq!(check_schema(&named("a b")), Ok(()));
        assert_eq!(
            check_schema(&[
                DatapackField::required("a", int()),
                DatapackField::required("a", int())
            ]),
            Err(E::InvalidFieldName)
        );
        let many: Vec<_> = (0..200)
            .map(|i| DatapackField::required(&format!("f{i}"), int()))
            .collect();
        assert_eq!(check_schema(&many), Ok(()));
        let kind = |kind| vec![DatapackField::required("a", kind)];
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Int { min: 2, max: 1 })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Float {
                min: 0.0,
                max: f64::NAN
            })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Float {
                min: f64::NEG_INFINITY,
                max: 1.0
            })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Text { max_len: 0 })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Reference {
                registry: "Bad Key".into()
            })),
            Err(E::InvalidFieldKind)
        );
        // A default must be a value of the kind, inside its bounds.
        let default = |value| vec![DatapackField::with_default("a", int(), value)];
        assert_eq!(check_schema(&default(DatapackValue::Int(1))), Ok(()));
        assert_eq!(
            check_schema(&default(DatapackValue::Int(2))),
            Err(E::InvalidDefault)
        );
        assert_eq!(
            check_schema(&default(DatapackValue::Bool(true))),
            Err(E::InvalidDefault)
        );
    }

    /// `example:model`, a small data model: a list of drop records, a map of tier records, and
    /// an optional field.
    pub(crate) fn model() -> DatapackRegistryDeclaration {
        let int = |min, max| DatapackFieldKind::Int { min, max };
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:model".into(),
            fields: vec![
                DatapackField::required("entity", DatapackFieldKind::Identifier),
                DatapackField::required(
                    "drops",
                    DatapackFieldKind::List {
                        of: Box::new(DatapackFieldKind::Record {
                            fields: vec![
                                DatapackField::required("item", DatapackFieldKind::Identifier),
                                DatapackField::with_default(
                                    "count",
                                    int(1, 64),
                                    DatapackValue::Int(1),
                                ),
                            ],
                        }),
                        min_len: 1,
                        max_len: Some(4),
                    },
                ),
                DatapackField::with_default(
                    "tiers",
                    DatapackFieldKind::Map {
                        key: DatapackKeyKind::Text { max_len: 16 },
                        value: Box::new(DatapackFieldKind::Record {
                            fields: vec![DatapackField::required("data_per_kill", int(0, 1000))],
                        }),
                    },
                    DatapackValue::Map(Vec::new()),
                ),
                DatapackField::optional("color", DatapackFieldKind::Text { max_len: 7 }),
            ],
            sync: DatapackRegistrySync::Required,
        }
    }

    #[test]
    fn a_nested_entry_decodes_and_reads_back() {
        let entry = model()
            .decode_entry(
                r#"{"entity": "zombie",
                    "drops": [{"item": "rotten_flesh", "count": 3}, {"item": "iron_ingot"}],
                    "tiers": {"superior": {"data_per_kill": 10}, "basic": {"data_per_kill": 1}}}"#,
            )
            .unwrap();
        assert_eq!(
            entry.get("entity").and_then(DatapackValue::as_text),
            Some("minecraft:zombie")
        );
        let drops = entry.get("drops").and_then(DatapackValue::as_list).unwrap();
        assert_eq!(drops.len(), 2);
        assert_eq!(
            drops[0].get("item").and_then(DatapackValue::as_text),
            Some("minecraft:rotten_flesh")
        );
        assert_eq!(
            drops[0].get("count").and_then(DatapackValue::as_int),
            Some(3)
        );
        // A nested default stands in as a top-level one does.
        assert_eq!(
            drops[1].get("count").and_then(DatapackValue::as_int),
            Some(1)
        );
        // Map pairs are in the file's order (Gson's `JsonObject` keeps it).
        let tiers = entry.get("tiers").unwrap();
        let keys: Vec<_> = tiers
            .as_map()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, ["superior", "basic"]);
        assert_eq!(
            tiers
                .get("superior")
                .and_then(|t| t.get("data_per_kill"))
                .and_then(DatapackValue::as_int),
            Some(10)
        );
        // An optional field left out is `None`, not a value.
        assert_eq!(entry.get("color"), None);
        assert_eq!(entry.values[3].1, DatapackValue::Absent);
        let colored = model()
            .decode_entry(r#"{"entity": "a", "drops": [{"item": "b"}], "color": "red"}"#)
            .unwrap();
        assert_eq!(
            colored.get("color").and_then(DatapackValue::as_text),
            Some("red")
        );
        // Every decoded value is one its kind accepts, as a client checks a snapshot.
        let decl = model();
        for (field, (_, value)) in decl.fields.iter().zip(&entry.values) {
            assert!(field.kind.accepts(value), "{}", field.name);
        }
    }

    #[test]
    fn nested_errors_name_the_full_path() {
        let decl = model();
        let err = |text: &str| decl.decode_entry(text).unwrap_err();
        assert_eq!(
            err(r#"{"entity": "a", "drops": []}"#),
            DatapackDecodeError::WrongLength {
                field: "drops".into()
            }
        );
        let five = r#"{"item": "b"}, "#.repeat(4) + r#"{"item": "b"}"#;
        assert_eq!(
            err(&format!(r#"{{"entity": "a", "drops": [{five}]}}"#)),
            DatapackDecodeError::WrongLength {
                field: "drops".into()
            }
        );
        assert_eq!(
            err(r#"{"entity": "a", "drops": [{"item": "b"}, {"item": "c", "count": 65}]}"#),
            DatapackDecodeError::OutOfRange {
                field: "drops[1].count".into()
            }
        );
        assert_eq!(
            err(r#"{"entity": "a", "drops": [{"item": "b"}, {"count": 2}]}"#),
            DatapackDecodeError::MissingField {
                field: "drops[1].item".into()
            }
        );
        assert_eq!(
            err(r#"{"entity": "a", "drops": [{"item": "b", "extra": 1}]}"#),
            DatapackDecodeError::UnknownField {
                field: "drops[0].extra".into()
            }
        );
        assert_eq!(
            err(
                r#"{"entity": "a", "drops": [{"item": "b"}], "tiers": {"basic": {"data_per_kill": "x"}}}"#
            ),
            DatapackDecodeError::WrongType {
                field: "tiers.basic.data_per_kill".into(),
                expected: "integer"
            }
        );
        assert_eq!(
            err(r#"{"entity": "a", "drops": {"item": "b"}}"#),
            DatapackDecodeError::WrongType {
                field: "drops".into(),
                expected: "array"
            }
        );
        assert_eq!(
            err(
                r#"{"entity": "a", "drops": [{"item": "b"}], "tiers": {"a_very_long_tier_name": {"data_per_kill": 1}}}"#
            ),
            DatapackDecodeError::TooLong {
                field: "tiers.a_very_long_tier_name".into()
            }
        );
        assert_eq!(
            err(r#"{"entity": "a", "drops": [{"item": "b"}], "color": 3}"#),
            DatapackDecodeError::WrongType {
                field: "color".into(),
                expected: "string"
            }
        );
    }

    #[test]
    fn id_keys_normalise_and_may_not_repeat() {
        let decl = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:weights".into(),
            fields: vec![DatapackField::required(
                "by_mob",
                DatapackFieldKind::Map {
                    key: DatapackKeyKind::Identifier,
                    value: Box::new(DatapackFieldKind::Int { min: 0, max: 9 }),
                },
            )],
            sync: DatapackRegistrySync::ServerOnly,
        };
        let entry = decl
            .decode_entry(r#"{"by_mob": {"zombie": 1, "a:b": 2}}"#)
            .unwrap();
        let keys: Vec<_> = entry
            .get("by_mob")
            .unwrap()
            .as_map()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        // The file's order, not the keys' (`Codec.unboundedMap` over Gson's `JsonObject`).
        assert_eq!(keys, ["minecraft:zombie", "a:b"]);
        let order = |text| -> Vec<String> {
            decl.decode_entry(text)
                .unwrap()
                .get("by_mob")
                .unwrap()
                .as_map()
                .unwrap()
                .iter()
                .map(|(k, _)| k.clone())
                .collect()
        };
        assert_eq!(
            order(r#"{"by_mob": {"z:z": 1, "a:a": 2, "m:m": 3}}"#),
            ["z:z", "a:a", "m:m"]
        );
        // A key written twice keeps its first place and its last value, as Gson's object does.
        let twice = decl
            .decode_entry(r#"{"by_mob": {"z:z": 1, "a:a": 2, "z:z": 3}}"#)
            .unwrap();
        assert_eq!(
            twice.get("by_mob").unwrap().as_map().unwrap(),
            [
                ("z:z".to_string(), DatapackValue::Int(3)),
                ("a:a".to_string(), DatapackValue::Int(2))
            ]
        );
        // Two spellings of one id are a duplicate, named as the file spells the second.
        assert_eq!(
            decl.decode_entry(r#"{"by_mob": {"minecraft:zombie": 1, "zombie": 2}}"#)
                .unwrap_err(),
            DatapackDecodeError::DuplicateKey {
                field: "by_mob.zombie".into()
            }
        );
        assert_eq!(
            decl.decode_entry(r#"{"by_mob": {"Not An Id": 1}}"#)
                .unwrap_err(),
            DatapackDecodeError::InvalidIdentifier {
                field: r#"by_mob["Not An Id"]"#.into()
            }
        );
    }

    /// A key with a `.`, a bracket, a quote or nothing in it is written as a quoted JSON string
    /// in brackets, so `tiers["basic.data_per_kill"]` and `tiers.basic.data_per_kill` are two
    /// paths; a top-level `neoforge:conditions` is the engine's, and skipping it keeps the order.
    #[test]
    fn error_paths_quote_keys_that_could_read_two_ways() {
        let decl = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:tiers".into(),
            fields: vec![DatapackField::required(
                "tiers",
                DatapackFieldKind::Map {
                    key: DatapackKeyKind::Text { max_len: 32 },
                    value: Box::new(DatapackFieldKind::Record {
                        fields: vec![DatapackField::required(
                            "data_per_kill",
                            DatapackFieldKind::Int { min: 0, max: 9 },
                        )],
                    }),
                },
            )],
            sync: DatapackRegistrySync::ServerOnly,
        };
        let error = |text| decl.decode_entry(text).unwrap_err();
        assert_eq!(
            error(r#"{"tiers": {"basic.data_per_kill": {"data_per_kill": 10}}}"#),
            DatapackDecodeError::OutOfRange {
                field: r#"tiers["basic.data_per_kill"].data_per_kill"#.into()
            }
        );
        assert_eq!(
            error(r#"{"tiers": {"basic": {"data_per_kill": 10}}}"#),
            DatapackDecodeError::OutOfRange {
                field: "tiers.basic.data_per_kill".into()
            }
        );
        assert_eq!(
            error(r#"{"tiers": {"a\"]b": {"data_per_kill": 10}}}"#),
            DatapackDecodeError::OutOfRange {
                field: r#"tiers["a\"]b"].data_per_kill"#.into()
            }
        );
        assert_eq!(
            error(r#"{"tiers": {"": {"data_per_kill": 10}}}"#),
            DatapackDecodeError::OutOfRange {
                field: r#"tiers[""].data_per_kill"#.into()
            }
        );
        let entry = decode_datapack_entry(
            &decl,
            r#"{"neoforge:conditions": [], "tiers": {"z": {"data_per_kill": 1}, "a": {"data_per_kill": 2}}}"#,
            &mut NoVanillaValueDecoder,
        ).unwrap();
        let keys: Vec<_> = entry
            .get("tiers")
            .unwrap()
            .as_map()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, ["z", "a"]);
        assert_eq!(
            decl.decode_entry(r#"{"neoforge:conditions": [], "tiers": {}}"#)
                .unwrap_err(),
            DatapackDecodeError::UnknownField {
                field: "neoforge:conditions".into()
            }
        );

        let mut overlapping = decl;
        overlapping.fields.push(DatapackField::with_default(
            "neoforge:conditions",
            DatapackFieldKind::Bool,
            DatapackValue::Bool(true),
        ));
        let entry = decode_datapack_entry(
            &overlapping,
            r#"{"neoforge:conditions": [], "tiers": {}}"#,
            &mut NoVanillaValueDecoder,
        )
        .unwrap();
        assert_eq!(
            entry.get("neoforge:conditions"),
            Some(&DatapackValue::Bool(true))
        );
    }

    /// Defaults are normalised at every depth as at the top, map keys included, and a default
    /// whose map names one key twice is refused as a duplicate.
    #[test]
    fn nested_defaults_normalise_and_may_not_repeat_a_key() {
        use DatapackRegistrationError as E;
        let ids = || DatapackFieldKind::Map {
            key: DatapackKeyKind::Identifier,
            value: Box::new(DatapackFieldKind::Identifier),
        };
        let mut fields = vec![DatapackField::required(
            "drop",
            DatapackFieldKind::Record {
                fields: vec![
                    DatapackField::with_default(
                        "item",
                        DatapackFieldKind::Identifier,
                        DatapackValue::Text("stone".into()),
                    ),
                    DatapackField::with_default(
                        "by_mob",
                        ids(),
                        DatapackValue::Map(vec![(
                            "zombie".into(),
                            DatapackValue::Text("rotten_flesh".into()),
                        )]),
                    ),
                ],
            },
        )];
        assert_eq!(normalize_defaults(&mut fields), Ok(()));
        assert_eq!(check_schema(&fields), Ok(()));
        let DatapackFieldKind::Record { fields: inner } = &fields[0].kind else {
            unreachable!()
        };
        assert_eq!(
            inner[0].default,
            Some(DatapackValue::Text("minecraft:stone".into()))
        );
        assert_eq!(
            inner[1].default,
            Some(DatapackValue::Map(vec![(
                "minecraft:zombie".into(),
                DatapackValue::Text("minecraft:rotten_flesh".into())
            )]))
        );
        let mut twice = vec![DatapackField::with_default(
            "by_mob",
            ids(),
            DatapackValue::Map(vec![
                ("zombie".into(), DatapackValue::Text("a".into())),
                ("minecraft:zombie".into(), DatapackValue::Text("b".into())),
            ]),
        )];
        assert_eq!(normalize_defaults(&mut twice), Ok(()));
        assert_eq!(check_schema(&twice), Err(E::DuplicateKey));
        let mut bad = vec![DatapackField::with_default(
            "item",
            DatapackFieldKind::List {
                of: Box::new(DatapackFieldKind::Identifier),
                min_len: 0,
                max_len: None,
            },
            DatapackValue::List(vec![DatapackValue::Text("Not An Id".into())]),
        )];
        assert_eq!(normalize_defaults(&mut bad), Err(E::InvalidDefault));
    }

    #[test]
    fn nested_schemas_are_checked_at_registration() {
        use DatapackRegistrationError as E;
        let int = || Box::new(DatapackFieldKind::Int { min: 0, max: 1 });
        let one = |kind| vec![DatapackField::required("a", kind)];
        assert_eq!(check_schema(&model().fields), Ok(()));
        let list = |min_len, max_len| DatapackFieldKind::List {
            of: int(),
            min_len,
            max_len,
        };
        assert_eq!(
            check_schema(&one(list(2, Some(1)))),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(check_schema(&one(list(2, None))), Ok(()));
        // A bad kind deep inside is refused as at the top.
        assert_eq!(
            check_schema(&one(DatapackFieldKind::List {
                of: Box::new(DatapackFieldKind::Int { min: 2, max: 1 }),
                min_len: 0,
                max_len: None
            })),
            Err(E::InvalidFieldKind)
        );
        // A text key of at most 0 bytes is the schema's choice: only the empty key.
        let empty_key = DatapackRegistryDeclaration {
            fields: one(DatapackFieldKind::Map {
                key: DatapackKeyKind::Text { max_len: 0 },
                value: int(),
            }),
            ..spell()
        };
        assert_eq!(check_schema(&empty_key.fields), Ok(()));
        let entry = empty_key.decode_entry(r#"{"a":{"":1}}"#).unwrap();
        assert_eq!(
            entry.get("a").unwrap().get(""),
            Some(&DatapackValue::Int(1))
        );
        assert_eq!(
            empty_key.decode_entry(r#"{"a":{"x":1}}"#).unwrap_err(),
            DatapackDecodeError::TooLong {
                field: "a.x".into()
            }
        );
        assert_eq!(
            check_schema(&one(DatapackFieldKind::Record {
                fields: vec![
                    DatapackField::required("x", DatapackFieldKind::Bool),
                    DatapackField::required("x", DatapackFieldKind::Bool)
                ]
            })),
            Err(E::InvalidFieldName)
        );
        // An optional kind stands only for a whole field, and has no default.
        assert_eq!(
            check_schema(&one(DatapackFieldKind::List {
                of: Box::new(DatapackFieldKind::Optional { of: int() }),
                min_len: 0,
                max_len: None
            })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&[DatapackField {
                name: "a".into(),
                kind: DatapackFieldKind::Optional { of: int() },
                default: Some(DatapackValue::Int(0)),
            }]),
            Err(E::InvalidDefault)
        );
        // A nested default must fit its kind.
        assert_eq!(
            check_schema(&[DatapackField::with_default(
                "a",
                DatapackFieldKind::List {
                    of: int(),
                    min_len: 1,
                    max_len: None
                },
                DatapackValue::List(Vec::new())
            )]),
            Err(E::InvalidDefault)
        );
    }

    /// `example:behavior`: a `behavior` field dispatched on `type` (`Codec.dispatch`), as Universal
    /// Bone Meal's growth rules are: `growing_plant` with an int and a reference, `random_tick`
    /// with a defaulted int.
    fn behaviors() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            fields: vec![DatapackField::required(
                "behavior",
                DatapackFieldKind::Dispatch {
                    key: "type".into(),
                    variants: vec![
                        DatapackVariant::new(
                            "example:growing_plant",
                            vec![
                                DatapackField::required(
                                    "max_height",
                                    DatapackFieldKind::Int { min: 1, max: 64 },
                                ),
                                DatapackField::required(
                                    "next",
                                    DatapackFieldKind::Reference {
                                        registry: "example:spell".into(),
                                    },
                                ),
                            ],
                        ),
                        DatapackVariant::new(
                            "example:random_tick",
                            vec![DatapackField::with_default(
                                "growth_attempts",
                                DatapackFieldKind::Int { min: 1, max: 16 },
                                DatapackValue::Int(1),
                            )],
                        ),
                    ],
                },
            )],
            ..spell()
        }
    }

    #[test]
    fn a_dispatched_field_decodes_the_variant_its_key_names() {
        let schema = behaviors();
        assert_eq!(check_schema(&schema.fields), Ok(()));
        let entry = schema
            .decode_entry(
                r#"{"behavior":{"type":"example:growing_plant","max_height":16,"next":"fire"}}"#,
            )
            .unwrap();
        let behavior = entry.get("behavior").unwrap();
        assert_eq!(behavior.variant_id(), Some("example:growing_plant"));
        assert_eq!(behavior.get("max_height"), Some(&DatapackValue::Int(16)));
        assert_eq!(
            behavior.get("next"),
            Some(&DatapackValue::Text("minecraft:fire".into()))
        );
        let mut refs = Vec::new();
        references(&schema.fields[0].kind, behavior, "behavior", &mut refs);
        assert_eq!(
            refs,
            [(
                "behavior.next".to_owned(),
                "example:spell",
                "minecraft:fire"
            )]
        );
        // The other variant, its field defaulted.
        let entry = schema
            .decode_entry(r#"{"behavior":{"type":"example:random_tick"}}"#)
            .unwrap();
        let behavior = entry.get("behavior").unwrap();
        assert_eq!(behavior.variant_id(), Some("example:random_tick"));
        assert_eq!(
            behavior.get("growth_attempts"),
            Some(&DatapackValue::Int(1))
        );
        assert!(schema.fields[0].kind.accepts(behavior));
    }

    #[test]
    fn a_dispatch_refuses_unknown_missing_and_foreign_keys() {
        let schema = behaviors();
        let fails = |json: &str| schema.decode_entry(json).unwrap_err();
        assert_eq!(
            fails(r#"{"behavior":{"type":"example:wither","max_height":1}}"#),
            DatapackDecodeError::UnknownVariant {
                field: "behavior.type".into(),
                id: "example:wither".into()
            }
        );
        // `byNameCodec` reads a bare id in `minecraft`, which declares no variant here.
        assert_eq!(
            fails(r#"{"behavior":{"type":"random_tick"}}"#),
            DatapackDecodeError::UnknownVariant {
                field: "behavior.type".into(),
                id: "minecraft:random_tick".into()
            }
        );
        assert_eq!(
            fails(r#"{"behavior":{"growth_attempts":2}}"#),
            DatapackDecodeError::MissingField {
                field: "behavior.type".into()
            }
        );
        assert_eq!(
            fails(r#"{"behavior":{"type":3}}"#),
            DatapackDecodeError::WrongType {
                field: "behavior.type".into(),
                expected: "identifier string"
            }
        );
        // A field of the other variant is not this one's.
        assert_eq!(
            fails(r#"{"behavior":{"type":"example:random_tick","max_height":1}}"#),
            DatapackDecodeError::UnknownField {
                field: "behavior.max_height".into()
            }
        );
        assert_eq!(
            fails(r#"{"behavior":{"type":"example:growing_plant","max_height":99,"next":"a"}}"#),
            DatapackDecodeError::OutOfRange {
                field: "behavior.max_height".into()
            }
        );
    }

    #[test]
    fn dispatch_schemas_are_checked_at_registration() {
        use DatapackRegistrationError as E;
        let dispatch = |key: &str, variants| {
            vec![DatapackField::required(
                "a",
                DatapackFieldKind::Dispatch {
                    key: key.into(),
                    variants,
                },
            )]
        };
        let bool_field = || vec![DatapackField::required("x", DatapackFieldKind::Bool)];
        assert_eq!(
            check_schema(&dispatch("type", Vec::new())),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&dispatch(
                "",
                vec![DatapackVariant::new("example:a", bool_field())]
            )),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&dispatch(
                "type",
                vec![
                    DatapackVariant::new("example:a", bool_field()),
                    DatapackVariant::new("example:a", Vec::new()),
                ]
            )),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&dispatch(
                "type",
                vec![DatapackVariant::new("a", bool_field())]
            )),
            Err(E::InvalidFieldKind)
        );
        // A variant field named like the key could never be read.
        assert_eq!(
            check_schema(&dispatch(
                "x",
                vec![DatapackVariant::new("example:a", bool_field())]
            )),
            Err(E::InvalidFieldName)
        );
        // A variant's own schema is checked as a record's.
        assert_eq!(
            check_schema(&dispatch(
                "type",
                vec![DatapackVariant::new(
                    "example:a",
                    vec![DatapackField::required(
                        "x",
                        DatapackFieldKind::Int { min: 2, max: 1 }
                    )]
                )]
            )),
            Err(E::InvalidFieldKind)
        );
        // A default must name a declared variant with fitting fields.
        let defaulted = |default| {
            vec![DatapackField::with_default(
                "a",
                DatapackFieldKind::Dispatch {
                    key: "type".into(),
                    variants: vec![DatapackVariant::new("example:a", bool_field())],
                },
                default,
            )]
        };
        assert_eq!(
            check_schema(&defaulted(DatapackValue::Variant {
                id: "example:a".into(),
                fields: vec![("x".into(), DatapackValue::Bool(true))],
            })),
            Ok(())
        );
        assert_eq!(
            check_schema(&defaulted(DatapackValue::Variant {
                id: "example:b".into(),
                fields: vec![("x".into(), DatapackValue::Bool(true))],
            })),
            Err(E::InvalidDefault)
        );
    }

    #[test]
    fn vanilla_value_kinds_validate_registration_and_defaults() {
        use DatapackRegistrationError as E;
        let field = |kind| vec![DatapackField::required("x", kind)];
        assert_eq!(
            check_schema(&field(DatapackFieldKind::IntProvider { min: 0, max: 128 })),
            Ok(())
        );
        assert_eq!(
            check_schema(&field(DatapackFieldKind::IntProvider { min: 5, max: 1 })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&field(DatapackFieldKind::BlockStateProvider)),
            Ok(())
        );
        assert_eq!(
            check_schema(&field(DatapackFieldKind::BlockPredicate)),
            Ok(())
        );
        assert_eq!(
            check_schema(&field(DatapackFieldKind::HolderSet {
                registry: "minecraft:block".into(),
            })),
            Ok(())
        );
        // Any registry other than `minecraft:block` is refused at registration.
        for other in [
            "minecraft:item",
            "minecraft:worldgen/biome",
            "example:spell",
            "block",
            "",
        ] {
            assert_eq!(
                check_schema(&field(DatapackFieldKind::HolderSet {
                    registry: other.into(),
                })),
                Err(E::InvalidFieldKind),
                "holder set registry {other:?} must be refused"
            );
        }
        // Valid JSON defaults pass and canonicalize; malformed JSON or wrong registry fail.
        let mut with_defaults = vec![
            DatapackField::with_default(
                "attempts",
                DatapackFieldKind::IntProvider { min: 1, max: 64 },
                DatapackValue::int_provider(" 1 "),
            ),
            DatapackField::with_default(
                "state",
                DatapackFieldKind::BlockStateProvider,
                DatapackValue::block_state_provider(r#"{ "id" : "minecraft:stone" }"#),
            ),
            DatapackField::with_default(
                "can_grow_into",
                DatapackFieldKind::BlockPredicate,
                DatapackValue::block_predicate(
                    r#"{"type": "minecraft:matching_block_tag", "tag": "minecraft:air"}"#,
                ),
            ),
            DatapackField::with_default(
                "targets",
                DatapackFieldKind::HolderSet {
                    registry: "minecraft:block".into(),
                },
                DatapackValue::holder_set("minecraft:block", r##""#minecraft:air""##),
            ),
        ];
        normalize_defaults(&mut with_defaults).unwrap();
        assert_eq!(check_schema(&with_defaults), Ok(()));
        assert_eq!(
            with_defaults[1].default,
            Some(DatapackValue::block_state_provider(
                r#"{"id":"minecraft:stone"}"#
            ))
        );
        let mut bad_json = vec![DatapackField::with_default(
            "attempts",
            DatapackFieldKind::IntProvider { min: 0, max: 10 },
            DatapackValue::int_provider("{not json"),
        )];
        assert_eq!(normalize_defaults(&mut bad_json), Err(E::InvalidDefault));
        let wrong_registry = vec![DatapackField::with_default(
            "targets",
            DatapackFieldKind::HolderSet {
                registry: "minecraft:block".into(),
            },
            DatapackValue::holder_set("minecraft:item", r#""minecraft:stone""#),
        )];
        assert_eq!(check_schema(&wrong_registry), Err(E::InvalidDefault));
    }

    struct StubDecoder {
        generation: u64,
        next: u32,
        reject: Option<&'static str>,
    }

    impl StubDecoder {
        fn new(generation: u64) -> Self {
            Self {
                generation,
                next: 0,
                reject: None,
            }
        }
        fn rejecting(needle: &'static str) -> Self {
            Self {
                generation: 1,
                next: 0,
                reject: Some(needle),
            }
        }
        fn take_index(&mut self, json: &str) -> Result<u32, String> {
            if let Some(needle) = self.reject
                && json.contains(needle)
            {
                return Err(format!("refused {needle}"));
            }
            let idx = self.next;
            self.next += 1;
            Ok(idx)
        }
    }

    impl VanillaValueDecoder for StubDecoder {
        fn decode_int_provider(
            &mut self,
            json: &str,
            min: i32,
            max: i32,
        ) -> Result<IntProviderValue, String> {
            if let Ok(v) = json.parse::<i32>()
                && (v < min || v > max)
            {
                return Err("out of bounds".into());
            }
            Ok(IntProviderValue::from_raw(
                self.generation,
                self.take_index(json)?,
            ))
        }
        fn decode_block_state_provider(
            &mut self,
            json: &str,
        ) -> Result<BlockStateProviderValue, String> {
            Ok(BlockStateProviderValue::from_raw(
                self.generation,
                self.take_index(json)?,
            ))
        }
        fn decode_block_predicate(&mut self, json: &str) -> Result<BlockPredicateValue, String> {
            Ok(BlockPredicateValue::from_raw(
                self.generation,
                self.take_index(json)?,
            ))
        }
        fn decode_holder_set(
            &mut self,
            registry: &str,
            json: &str,
        ) -> Result<HolderSetValue, String> {
            assert_eq!(registry, "minecraft:block");
            Ok(HolderSetValue::from_raw(
                self.generation,
                self.take_index(json)?,
            ))
        }
    }

    fn ubm_schema() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "universalbonemeal".into(),
            key: "universalbonemeal:bone_meal_behavior".into(),
            fields: vec![
                DatapackField::required(
                    "behavior",
                    DatapackFieldKind::Dispatch {
                        key: "type".into(),
                        variants: vec![
                            DatapackVariant::new(
                                "universalbonemeal:growing_plant",
                                vec![
                                    DatapackField::required(
                                        "blocks_to_grow",
                                        DatapackFieldKind::IntProvider { min: 0, max: 128 },
                                    ),
                                    DatapackField::with_default(
                                        "can_grow_into",
                                        DatapackFieldKind::BlockPredicate,
                                        DatapackValue::block_predicate(
                                            r#"{"type":"minecraft:matching_block_tag","tag":"minecraft:air"}"#,
                                        ),
                                    ),
                                    DatapackField::required(
                                        "vegetation",
                                        DatapackFieldKind::BlockStateProvider,
                                    ),
                                ],
                            ),
                            DatapackVariant::new(
                                "universalbonemeal:neighbor_conversion",
                                vec![DatapackField::required(
                                    "spread_sources",
                                    DatapackFieldKind::HolderSet {
                                        registry: "minecraft:block".into(),
                                    },
                                )],
                            ),
                        ],
                    },
                ),
                DatapackField::optional("predicate", DatapackFieldKind::BlockPredicate),
            ],
            sync: DatapackRegistrySync::Required,
        }
    }

    #[test]
    fn vanilla_value_kinds_decode_ubm_examples_and_refuse_bad_values_with_paths() {
        let schema = ubm_schema();
        assert_eq!(check_schema(&schema.fields), Ok(()));
        let mut decoder = StubDecoder::new(7);
        // UBM cactus.json shape (uniform IntProvider, weighted BlockStateProvider, defaulted BlockPredicate).
        let cactus = decode_datapack_entry(
            &schema,
            r#"{
                    "behavior": {
                        "type": "universalbonemeal:growing_plant",
                        "blocks_to_grow": {"type": "minecraft:uniform", "min_inclusive": 1, "max_inclusive": 2},
                        "vegetation": {
                            "type": "minecraft:weighted",
                            "entries": [
                                {"data": "minecraft:cactus", "weight": 9},
                                {"data": "minecraft:cactus_flower", "weight": 1}
                            ]
                        }
                    },
                    "predicate": {"type": "minecraft:matching_block_tag", "offset": [0, 1, 0], "tag": "minecraft:air"}
                }"#,
            &mut decoder,
        )
        .unwrap();
        let behavior = cactus.get("behavior").unwrap();
        let blocks_to_grow = behavior
            .get("blocks_to_grow")
            .unwrap()
            .as_int_provider()
            .unwrap();
        let can_grow_into = behavior
            .get("can_grow_into")
            .unwrap()
            .as_block_predicate()
            .unwrap();
        let vegetation = behavior
            .get("vegetation")
            .unwrap()
            .as_block_state_provider()
            .unwrap();
        let predicate = cactus
            .get("predicate")
            .unwrap()
            .as_block_predicate()
            .unwrap();
        assert_eq!(blocks_to_grow.generation(), 7);
        assert_eq!(can_grow_into.generation(), 7);
        assert_eq!(vegetation.generation(), 7);
        assert_eq!(predicate.generation(), 7);
        assert_ne!(can_grow_into.index(), predicate.index());

        // UBM dirt.json shape (block HolderSet list).
        let dirt = decode_datapack_entry(
            &schema,
            r#"{
                    "behavior": {
                        "type": "universalbonemeal:neighbor_conversion",
                        "spread_sources": ["minecraft:grass_block", "minecraft:mycelium"]
                    }
                }"#,
            &mut decoder,
        )
        .unwrap();
        let spread_sources = dirt
            .get("behavior")
            .unwrap()
            .get("spread_sources")
            .unwrap()
            .as_holder_set()
            .unwrap();
        assert_eq!(spread_sources.generation(), 7);

        // Bad value for each kind reports its full nested path.
        assert_eq!(
            decode_datapack_entry(
                &schema,
                r#"{"behavior":{"type":"universalbonemeal:growing_plant","blocks_to_grow":999,"vegetation":{"id":"minecraft:cactus"}}}"#,
                &mut StubDecoder::new(1),
            )
            .unwrap_err(),
            DatapackDecodeError::InvalidVanillaValue {
                field: "behavior.blocks_to_grow".into(),
                reason: "out of bounds".into(),
            }
        );
        assert_eq!(
            decode_datapack_entry(
                &schema,
                r#"{"behavior":{"type":"universalbonemeal:growing_plant","blocks_to_grow":1,"vegetation":{"id":"bad"}}}"#,
                &mut StubDecoder::rejecting("bad"),
            )
            .unwrap_err(),
            DatapackDecodeError::InvalidVanillaValue {
                field: "behavior.vegetation".into(),
                reason: "refused bad".into(),
            }
        );
        assert_eq!(
            decode_datapack_entry(
                &schema,
                r#"{"behavior":{"type":"universalbonemeal:neighbor_conversion","spread_sources":"minecraft:dirt"},"predicate":{"type":"bad"}}"#,
                &mut StubDecoder::rejecting("bad"),
            )
            .unwrap_err(),
            DatapackDecodeError::InvalidVanillaValue {
                field: "predicate".into(),
                reason: "refused bad".into(),
            }
        );
        assert_eq!(
            decode_datapack_entry(
                &schema,
                r##"{"behavior":{"type":"universalbonemeal:neighbor_conversion","spread_sources":"#bad:tag"}}"##,
                &mut StubDecoder::rejecting("#bad:tag"),
            )
            .unwrap_err(),
            DatapackDecodeError::InvalidVanillaValue {
                field: "behavior.spread_sources".into(),
                reason: "refused #bad:tag".into(),
            }
        );
    }

    #[test]
    fn vanilla_value_snapshot_unbinds_on_wire_and_rebinds_on_client() {
        use super::super::datapack_registry_set::{DatapackCandidate, DatapackRegistrySet};
        use super::super::datapack_snapshot::{DatapackSnapshot, DatapackSnapshotError};
        let schema = ubm_schema();
        let declarations = std::slice::from_ref(&schema);
        let files = vec![(
            "data/universalbonemeal/universalbonemeal/bone_meal_behavior/dirt.json".to_owned(),
            vec![(
                "pack".to_owned(),
                br#"{"behavior":{"type":"universalbonemeal:neighbor_conversion","spread_sources":["minecraft:grass_block","minecraft:mycelium"]}}"#.to_vec(),
            )],
        )];
        let candidate =
            DatapackCandidate::build(declarations, &files, &mut StubDecoder::new(11)).unwrap();
        let server = DatapackRegistrySet::from_candidate(candidate);
        assert_eq!(
            server
                .get(
                    "universalbonemeal:bone_meal_behavior",
                    "universalbonemeal:dirt"
                )
                .unwrap()
                .get("behavior")
                .unwrap()
                .get("spread_sources")
                .unwrap()
                .as_holder_set()
                .unwrap()
                .generation(),
            11
        );
        let snapshot = server.snapshot_for(declarations, declarations).unwrap();
        // Wire snapshot strips the server's generation number.
        let decoded_snapshot = DatapackSnapshot::decode(&snapshot.encode().unwrap()).unwrap();
        let wire_candidate = decoded_snapshot.candidate(declarations).unwrap();
        let wire_set = DatapackRegistrySet::received(wire_candidate);
        assert_eq!(
            wire_set
                .get(
                    "universalbonemeal:bone_meal_behavior",
                    "universalbonemeal:dirt"
                )
                .unwrap()
                .get("behavior")
                .unwrap()
                .get("spread_sources")
                .unwrap()
                .as_holder_set()
                .unwrap()
                .generation(),
            0
        );
        // Client re-binds against its own generation.
        let bound = wire_set
            .rebind_vanilla_values(declarations, &mut StubDecoder::new(22))
            .unwrap();
        assert_eq!(
            bound
                .get(
                    "universalbonemeal:bone_meal_behavior",
                    "universalbonemeal:dirt"
                )
                .unwrap()
                .get("behavior")
                .unwrap()
                .get("spread_sources")
                .unwrap()
                .as_holder_set()
                .unwrap()
                .generation(),
            22
        );
        // Invalid vanilla value on client re-bind fails atomically as InvalidEntry.
        assert_eq!(
            wire_set
                .rebind_vanilla_values(declarations, &mut StubDecoder::rejecting("mycelium"))
                .map(|_| ())
                .unwrap_err(),
            DatapackSnapshotError::InvalidEntry {
                registry: "universalbonemeal:bone_meal_behavior".into(),
                entry: "universalbonemeal:dirt".into(),
            }
        );
    }

    #[test]
    fn vanilla_value_handles_refuse_without_backend() {
        struct BareHost;
        impl crate::Host for BareHost {}
        crate::impl_host_areas!(BareHost);
        let host = BareHost;
        let level = crate::LevelRef::new(
            crate::WorldEpoch(1),
            crate::DimensionKey::from_slot(0).unwrap(),
        );
        let block = crate::BlockKey(1);
        let mut rng = crate::LegacyRandomSource::new(1);
        assert_eq!(
            IntProviderValue::from_raw(1, 0).sample(&host, &mut rng),
            Err(DatapackValueError::Unavailable)
        );
        assert_eq!(
            BlockStateProviderValue::from_raw(1, 0).state(&host, level, &mut rng, [0, 0, 0]),
            Err(DatapackValueError::Unavailable)
        );
        assert_eq!(
            BlockPredicateValue::from_raw(1, 0).test(&host, level, [0, 0, 0]),
            Err(DatapackValueError::Unavailable)
        );
        assert_eq!(
            HolderSetValue::from_raw(1, 0).contains(&host, block),
            Err(DatapackValueError::Unavailable)
        );
    }
}
