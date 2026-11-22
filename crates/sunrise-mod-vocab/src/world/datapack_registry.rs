//! Datapack registries a native mod declares: a registry
//! of the mod's own, whose entries are JSON files in data packs. The mod
//! declares a record schema once at startup (scalars, and lists, maps and
//! records of them, nested as deep as the schema says); the game decodes every
//! entry against it without running mod code. See `docs/design/mod-api/mod-datapack-registries.md`.
use serde_json::Value;
use sunrise_core::identifier::Identifier;

/// How a registry's entries reach joining clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackRegistrySync {
    /// Entries stay on the server; a client needs nothing of the mod for them.
    #[default]
    ServerOnly,
    /// Clients receive the entries, and a client without the mod is refused.
    Required,
    /// Clients with the mod receive the entries; others join without them.
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
}

impl DatapackValue {
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

    /// The field `name` of a record, or a map's value at the key `name`; `None` for anything
    /// else, and for an optional field the entry left out.
    pub fn get(&self, name: &str) -> Option<&DatapackValue> {
        match self {
            Self::Record(pairs) | Self::Map(pairs) => pairs
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
    /// Only both-side packages may declare datapack registries.
    WrongSide,
    /// The registry's key is malformed, empty or in a namespace the
    /// declaring mod does not own (a vanilla registry's included).
    InvalidKey,
    /// Another declaration already covers the registry key.
    Duplicate,
    /// A field name is empty or repeated.
    InvalidFieldName,
    /// A field's kind is unusable: bounds that are not finite or `min > max`,
    /// a scalar text field's `max_len` of 0, a list's `min_len` over its `max_len`, an
    /// optional kind nested in another, or a reference to a malformed registry key, at any depth.
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
pub(crate) fn valid_namespaced(text: &str) -> bool {
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
                pairs.len() == fields.len()
                    && fields
                        .iter()
                        .zip(pairs)
                        .all(|(field, (name, v))| *name == field.name && field.kind.accepts(v))
            }
            (Self::Optional { .. }, DatapackValue::Absent) => true,
            (Self::Optional { of }, value) => of.accepts(value),
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
            Self::Map { .. } | Self::Record { .. } => "object",
            Self::Optional { of } => of.expected(),
        }
    }
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
        DatapackValue::Record(pairs) => pairs.iter().any(|(_, v)| repeats_a_key(v)),
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
        _ => Ok(()),
    }
}

impl DatapackRegistryDeclaration {
    /// The file an entry `entry` (`addon:fire`) of this registry (`example:spell`)
    /// sits in, inside a data pack: `data/addon/example/spell/fire.json`.
    pub fn entry_file(&self, entry: &str) -> Option<String> {
        let (entry_namespace, entry_path) = entry.split_once(':')?;
        let (registry_namespace, registry_path) = self.key.split_once(':')?;
        Some(format!(
            "data/{entry_namespace}/{registry_namespace}/{registry_path}/{entry_path}.json"
        ))
    }

    /// Decodes one entry file against the schema. A key the schema does not
    /// declare, a missing required key, a wrong type or an out-of-range value
    /// fails the whole entry, at any depth, and no default stands in for invalid
    /// data. The error names the value's full path.
    /// Nothing here runs mod code. Nesting is bounded by the JSON parser's
    /// recursion limit; the file's size is bounded before this, by
    /// [`DatapackCandidate::build`](super::datapack_registry_set::DatapackCandidate::build).
    pub fn decode_entry(&self, text: &str) -> Result<DatapackEntry, DatapackDecodeError> {
        self.decode_entry_ignoring(text, &[])
    }

    /// [`Self::decode_entry`] with the top-level keys `ignored` belonging to the caller
    /// (`neoforge:conditions`, the engine's), not to the schema. A map's pairs keep the order the
    /// file wrote them in.
    pub(super) fn decode_entry_ignoring(
        &self,
        text: &str,
        ignored: &[&str],
    ) -> Result<DatapackEntry, DatapackDecodeError> {
        // serde_json's own recursion limit (128) bounds nesting before any value is built.
        let json: Json = serde_json::from_str(text)
            .map_err(|error| DatapackDecodeError::Malformed(error.to_string()))?;
        let Json::Object(mut object) = json else {
            return Err(DatapackDecodeError::NotAnObject);
        };
        if !ignored.is_empty() {
            object.retain(|(key, _)| !ignored.contains(&key.as_str()));
        }
        decode_fields(&self.fields, &object, &[], "").map(|values| DatapackEntry { values })
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

/// Decodes a JSON object against a flat record schema, the same way for every user of it (a
/// datapack registry's entries, a mod feature type's configs): an undeclared key (other than
/// the `ignored` ones the caller owns, a configured feature's `type`), a missing required key, a
/// wrong type or an out-of-range value fails the whole record. Nothing here runs mod code.
pub(crate) fn decode_record(
    fields: &[DatapackField],
    object: &serde_json::Map<String, Value>,
    ignored: &[&str],
) -> Result<DatapackEntry, DatapackDecodeError> {
    let object: Vec<_> = object
        .iter()
        .map(|(k, v)| (k.clone(), Json::of(v)))
        .collect();
    decode_fields(fields, &object, ignored, "").map(|values| DatapackEntry { values })
}

/// A record's fields at `path` (empty for an entry's own).
fn decode_fields(
    fields: &[DatapackField],
    object: &[(String, Json)],
    ignored: &[&str],
    path: &str,
) -> Result<Vec<(String, DatapackValue)>, DatapackDecodeError> {
    if let Some((unknown, _)) = object.iter().find(|(key, _)| {
        !ignored.contains(&key.as_str()) && !fields.iter().any(|field| field.name == *key)
    }) {
        return Err(DatapackDecodeError::UnknownField {
            field: joined(path, unknown),
        });
    }
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        let at = joined(path, &field.name);
        let json = object
            .iter()
            .find_map(|(key, json)| (*key == field.name).then_some(json));
        let value = match (json, &field.kind) {
            (Some(json), kind) => decode_value(kind, json, &at)?,
            (None, DatapackFieldKind::Optional { .. }) => DatapackValue::Absent,
            (None, _) => field
                .default
                .clone()
                .ok_or(DatapackDecodeError::MissingField { field: at })?,
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
                .map(|(n, item)| decode_value(of, item, &format!("{path}[{n}]")))
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
                pairs.push((k, decode_value(value, v, &at)?));
            }
            Ok(DatapackValue::Map(pairs))
        }
        DatapackFieldKind::Record { fields } => {
            let object = json.as_object().ok_or_else(wrong)?;
            decode_fields(fields, object, &[], path).map(DatapackValue::Record)
        }
        DatapackFieldKind::Optional { of } => decode_value(of, json, path),
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

    /// NeoForge's rule for a datapack registry outside `minecraft` (its Registries guide, "Datapack
    /// Registries"): `data/<entry namespace>/<registry namespace>/<registry path>/<entry>.json`.
    /// It is not vanilla's, which uses the registry's path only; registration refuses a
    /// `minecraft` key, so no vanilla-path case exists.
    #[test]
    fn entry_files_follow_neoforges_namespaced_registry_folder() {
        assert_eq!(
            spell().entry_file("addon:fire").as_deref(),
            Some("data/addon/example/spell/fire.json")
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
    fn nesting_is_bounded_by_the_json_parser() {
        // Size is the candidate's limit (`datapack_registry_set` tests), not the decoder's.
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
        let entry = decl
            .decode_entry_ignoring(
                r#"{"neoforge:conditions": [], "tiers": {"z": {"data_per_kill": 1}, "a": {"data_per_kill": 2}}}"#,
                &["neoforge:conditions"],
            )
            .unwrap();
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
        let entry = overlapping
            .decode_entry_ignoring(
                r#"{"neoforge:conditions": [], "tiers": {}}"#,
                &["neoforge:conditions"],
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
}
