//! Declarative, load-only codec shapes for built-in loot data.
//! Unknown object fields are deliberately ignored, just as map codecs ignore them.
use super::*;
use crate::loot::number::int_value;
mod component_predicates;
mod components;
mod conditions;
mod dialogs;
mod entities;
mod entries;
mod environment;
mod functions;
mod particles;
mod predicates;
mod providers;
mod text;

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Number,
    Any,
    Transient,
    NbtPath,
    NonAirItemId,
    PlayerName,
    TypedEntityData,
    AttributeNumber(bool),
    EnvironmentCheck,
    MapMax(&'static Kind, usize),
    EnumMap(&'static [&'static str], &'static Kind),
    PartialWeightedList(&'static Kind),
    DifferenceLimit(&'static str, &'static str, &'static str),
    WithinEndpoints(&'static str, &'static str, &'static str),
    StringLimit(&'static str, &'static str, i64),
    AtMostOne(&'static str, &'static str),
    NotBoolEnum(&'static str, bool, &'static str, &'static str),
    ChatString,
    VariableName,
    MacroTemplate,
    HttpScheme,
    Uuid,
    Text,
    SlotRange,
    Table,
    IntRange(i64, i64),
    String(usize),
    Id,
    Enum(&'static [&'static str]),
    Object(&'static [Field]),
    All(&'static [Kind]),
    FloatRange(f64, f64),
    Ordered(&'static str, &'static str, bool),
    OrderedFloat(&'static str, &'static str, bool),
    OrderedInt(&'static str, &'static str),
    PositiveWeights,
    Tag,
    RegistryEnum(&'static [&'static str]),
    RegistryMap(&'static [Row]),
    List(&'static Kind, usize, usize),
    Map(&'static Kind),
    IdMap(&'static Kind),
    SlotMap(&'static Kind),
    ComponentMap(bool),
    ComponentPredicates,
    HexColor(usize),
    Either(&'static [Kind]),
    Named(&'static str),
    Switch(&'static str, bool, &'static [Row]),
    FuzzySwitch(&'static str, &'static [Row]),
    NotBoth(&'static str, &'static str),
    TextColor,
    Snbt,
    Function,
    HolderSet(bool),
    Condition,
    Entry,
}
#[derive(Clone, Copy)]
struct Field {
    name: &'static str,
    kind: Kind,
    required: bool,
}
struct Row {
    name: &'static str,
    kind: Kind,
}
const fn req(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        required: true,
    }
}
const fn opt(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        required: false,
    }
}
const fn row(name: &'static str, fields: &'static [Field]) -> Row {
    Row {
        name,
        kind: Kind::Object(fields),
    }
}
fn named(name: &str) -> Option<Kind> {
    predicates::ROWS
        .iter()
        .chain(providers::ROWS)
        .chain(entities::ROWS)
        .chain(component_predicates::ROWS)
        .chain(components::ROWS)
        .chain(environment::ROWS)
        .chain(text::ROWS)
        .chain(particles::ROWS)
        .chain(dialogs::ROWS)
        .chain(entries::ROWS)
        .find(|row| row.name == name)
        .map(|row| row.kind)
}
fn builtin(kind: &str, function: bool) -> Option<Kind> {
    let kind = kind.strip_prefix("minecraft:")?;
    let rows = if function {
        functions::ROWS
    } else {
        conditions::ROWS
    };
    rows.iter().find(|row| row.name == kind).map(|row| row.kind)
}
pub(super) fn contains(kind: &str, function: bool) -> bool {
    builtin(kind, function).is_some()
}
#[derive(Clone, Copy)]
pub(super) enum ChildKind {
    Function,
    Condition,
    Set(bool),
    Table,
}
pub(super) struct Child<'v> {
    pub value: &'v Value,
    pub kind: ChildKind,
    pub path: String,
    pub pointer: String,
}
#[derive(Clone)]
struct Location {
    diagnostic: String,
    pointer: String,
}
impl Location {
    fn field(&self, key: &str) -> Self {
        Self {
            diagnostic: format!("{}.{key}", self.diagnostic),
            pointer: format!(
                "{}/{}",
                self.pointer,
                key.replace('~', "~0").replace('/', "~1")
            ),
        }
    }
    fn index(&self, index: usize) -> Self {
        Self {
            diagnostic: format!("{}[{index}]", self.diagnostic),
            pointer: format!("{}/{index}", self.pointer),
        }
    }
}
impl std::ops::Deref for Location {
    type Target = str;
    fn deref(&self) -> &str {
        &self.diagnostic
    }
}
impl std::fmt::Display for Location {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.diagnostic.fmt(formatter)
    }
}
struct Task<'v> {
    value: &'v Value,
    kind: Kind,
    path: Location,
    owner: String,
}
enum Work<'v> {
    Check(Task<'v>),
    EndAlternative,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Validate,
    Locate,
}
enum Variants {
    Kinds(std::slice::Iter<'static, Kind>),
    Rows(std::slice::Iter<'static, Row>),
}
impl Variants {
    fn next(&mut self) -> Option<Kind> {
        match self {
            Self::Kinds(kinds) => kinds.next().copied(),
            Self::Rows(rows) => rows.next().map(|row| row.kind),
        }
    }
}
struct Alternative<'v> {
    value: &'v Value,
    remaining: Variants,
    path: Location,
    pending: usize,
    children: usize,
    owner: String,
    failure: Option<(Location, String, String)>,
}

fn slot_range(value: &str) -> bool {
    if [
        "contents",
        "weapon",
        "weapon.mainhand",
        "weapon.offhand",
        "weapon.*",
        "armor.head",
        "armor.chest",
        "armor.legs",
        "armor.feet",
        "armor.body",
        "armor.*",
        "saddle",
        "horse.chest",
        "player.cursor",
    ]
    .contains(&value)
    {
        return true;
    }
    let Some((prefix, suffix)) = value.rsplit_once('.') else {
        return false;
    };
    let Some((_, maximum)) = [
        ("container", 54),
        ("hotbar", 9),
        ("inventory", 27),
        ("enderchest", 27),
        ("mob.inventory", 8),
        ("horse", 15),
        ("player.crafting", 4),
    ]
    .iter()
    .find(|(name, _)| *name == prefix) else {
        return false;
    };
    suffix == "*"
        || suffix
            .parse::<usize>()
            .is_ok_and(|n| n < *maximum && n.to_string() == suffix)
}
fn identifier(value: &str) -> bool {
    sunrise_core::identifier::Identifier::parse(value).is_ok()
}
fn matching_row(rows: &'static [Row], value: &str, qualified: bool) -> Option<&'static Row> {
    if qualified {
        let id = sunrise_core::identifier::Identifier::parse(value).ok()?;
        (id.namespace() == "minecraft")
            .then(|| rows.iter().find(|row| row.name == id.path()))
            .flatten()
    } else {
        rows.iter().find(|row| row.name == value)
    }
}

fn double_value(number: &serde_json::Number) -> f64 {
    number.as_f64().unwrap_or_else(|| {
        // A valid JSON number outside binary64 becomes infinity in Number.doubleValue.
        if number.to_string().starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }
    })
}

fn variable_name(value: &str) -> bool {
    value.encode_utf16().all(|unit| {
        unit == u16::from(b'_') || sunrise_core::java_character::is_letter_or_digit(unit)
    })
}

fn macro_template(value: &str) -> bool {
    let mut remaining = value;
    let mut found = false;
    while let Some(start) = remaining.find("$(") {
        remaining = &remaining[start + 2..];
        let Some(end) = remaining.find(')') else {
            return false;
        };
        if !variable_name(&remaining[..end]) {
            return false;
        }
        found = true;
        remaining = &remaining[end + 1..];
    }
    found
}

/// Use explicit work and alternative stacks, including for recursive provider shapes.
/// A malformed deeply nested native value must not overflow the Rust call stack.
pub(super) fn validate<'v>(
    value: &'v Value,
    function: bool,
    kind: &str,
    path: &str,
) -> io::Result<Vec<Child<'v>>> {
    let Some(shape) = builtin(kind, function) else {
        return Ok(Vec::new());
    };
    validate_shape(value, shape, kind, path, Purpose::Validate)
}

pub(super) fn children<'v>(value: &'v Value, function: bool, kind: &str) -> Vec<Child<'v>> {
    let Some(shape) = builtin(kind, function) else {
        return Vec::new();
    };
    validate_shape(value, shape, kind, "", Purpose::Locate).unwrap_or_default()
}

fn validate_shape<'v>(
    value: &'v Value,
    shape: Kind,
    kind: &str,
    path: &str,
    purpose: Purpose,
) -> io::Result<Vec<Child<'v>>> {
    let mut pending = vec![Work::Check(Task {
        value,
        kind: shape,
        path: Location {
            diagnostic: path.to_owned(),
            pointer: String::new(),
        },
        owner: kind.to_owned(),
    })];
    let mut alternatives: Vec<Alternative<'v>> = Vec::new();
    let mut children = Vec::new();
    while let Some(work) = pending.pop() {
        let Work::Check(Task {
            value,
            kind: rule,
            mut path,
            owner,
        }) = work
        else {
            alternatives.pop();
            continue;
        };
        // Normalization locates typed child edges; it does not decode scalar
        // codecs or text twice. Alternative containers may all be inspected:
        // only their matching object/list shapes can expose child locations.
        if purpose == Purpose::Locate
            && matches!(
                rule,
                Kind::Bool
                    | Kind::Number
                    | Kind::Any
                    | Kind::Transient
                    | Kind::NbtPath
                    | Kind::NonAirItemId
                    | Kind::PlayerName
                    | Kind::TypedEntityData
                    | Kind::AttributeNumber(_)
                    | Kind::DifferenceLimit(..)
                    | Kind::WithinEndpoints(..)
                    | Kind::StringLimit(..)
                    | Kind::AtMostOne(..)
                    | Kind::NotBoolEnum(..)
                    | Kind::ChatString
                    | Kind::VariableName
                    | Kind::MacroTemplate
                    | Kind::HttpScheme
                    | Kind::Uuid
                    | Kind::Text
                    | Kind::SlotRange
                    | Kind::IntRange(..)
                    | Kind::String(_)
                    | Kind::Id
                    | Kind::Enum(_)
                    | Kind::FloatRange(..)
                    | Kind::Ordered(..)
                    | Kind::OrderedFloat(..)
                    | Kind::OrderedInt(..)
                    | Kind::PositiveWeights
                    | Kind::Tag
                    | Kind::RegistryEnum(_)
                    | Kind::HexColor(_)
                    | Kind::NotBoth(..)
                    | Kind::TextColor
                    | Kind::Snbt
                    | Kind::PartialWeightedList(_)
            )
        {
            continue;
        }
        let mut reason = None;
        let mut push = |kind, value, path| {
            pending.push(Work::Check(Task {
                value,
                kind,
                path,
                owner: owner.clone(),
            }))
        };
        match rule {
            Kind::Any => {}
            Kind::Text => {
                // Shared TextChecks semantic validation plugs in here (#2112).
                // Keep one entry point for every nested text field; no copied parser.
                push(Kind::Named("text_fields"), value, path.clone());
            }
            Kind::SlotRange => {
                if !value.as_str().is_some_and(slot_range) {
                    reason = Some("expected slot range".into());
                }
            }
            Kind::Entry => push(Kind::Named("loot_entry"), value, path.clone()),
            Kind::Table => {
                if let Some(id) = value.as_str() {
                    if identifier(id) {
                        children.push(Child {
                            value,
                            kind: ChildKind::Table,
                            path: path.diagnostic.clone(),
                            pointer: path.pointer.clone(),
                        });
                    } else {
                        reason = Some("expected loot table reference".into());
                    }
                } else {
                    push(Kind::Named("loot_table_direct"), value, path.clone());
                }
            }
            Kind::ChatString => {
                if !value.as_str().is_some_and(|s| {
                    s.encode_utf16()
                        .all(|unit| unit >= 32 && unit != 127 && unit != 167)
                }) {
                    reason = Some("expected chat-safe string".into());
                }
            }
            Kind::VariableName => {
                if !value.as_str().is_some_and(variable_name) {
                    reason = Some("expected Java letter/digit/underscore variable name".into());
                }
            }
            Kind::MacroTemplate => {
                if !value.as_str().is_some_and(macro_template) {
                    reason =
                        Some("expected command template containing a complete variable".into());
                }
            }
            Kind::HttpScheme => {
                if !value
                    .as_str()
                    .and_then(|s| s.split_once(':'))
                    .is_some_and(|(scheme, _)| {
                        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
                    })
                {
                    reason = Some("expected URI with HTTP or HTTPS scheme".into());
                }
            }
            Kind::Uuid => {
                if !value
                    .as_str()
                    .is_some_and(|s| sunrise_core::text::parse_java_uuid(s).is_some())
                {
                    reason = Some("expected UUID string".into());
                }
            }
            Kind::DifferenceLimit(min, max, limit) => {
                if let (Some(min), Some(max), Some(limit)) = (
                    value[min].as_number(),
                    value[max].as_number(),
                    value[limit].as_number(),
                ) {
                    let span = (int_value(max) as i32).wrapping_sub(int_value(min) as i32);
                    if int_value(limit) > i64::from(span) {
                        reason = Some("plateau exceeds integer interval".into());
                    }
                }
            }
            Kind::WithinEndpoints(initial, start, end) => {
                if let (Some(initial), Some(start), Some(end)) = (
                    value[initial].as_number(),
                    value[start].as_number(),
                    value[end].as_number(),
                ) {
                    let (initial, start, end) = (
                        double_value(initial) as f32,
                        double_value(start) as f32,
                        double_value(end) as f32,
                    );
                    if initial < start.min(end) || initial > start.max(end) {
                        reason = Some("initial value is outside endpoints".into());
                    }
                }
            }
            Kind::StringLimit(field, limit, default) => {
                if let Some(text) = value[field].as_str() {
                    let limit = value[limit].as_number().map_or(default, int_value);
                    if text.encode_utf16().count() as i128 > i128::from(limit) {
                        reason = Some(format!("{field} exceeds its length limit"));
                    }
                }
            }
            Kind::AtMostOne(list, flag) => {
                if value[list].as_array().is_some_and(|values| {
                    values
                        .iter()
                        .filter(|value| value[flag] == true)
                        .take(2)
                        .count()
                        > 1
                }) {
                    reason = Some("at most one initial option is allowed".into());
                }
            }
            Kind::NotBoolEnum(flag, default, field, forbidden) => {
                if value[flag].as_bool().unwrap_or(default)
                    && value[field].as_str() == Some(forbidden)
                {
                    reason = Some(format!("{flag} conflicts with {field}={forbidden}"));
                }
            }
            Kind::PartialWeightedList(element) => {
                if let Some(values) = value.as_array() {
                    let mut sum = 0_i32;
                    for (index, entry) in values.iter().enumerate() {
                        // MobSpawnSettings promotes list partials. Its element grammar
                        // contains only legacy count providers, not this list rule.
                        if validate_shape(
                            entry,
                            *element,
                            &owner,
                            &format!("{path}[{index}]"),
                            Purpose::Validate,
                        )
                        .is_ok()
                        {
                            let weight = entry["weight"].as_number().map_or(0, int_value) as i32;
                            if let Some(total) = sum.checked_add(weight) {
                                sum = total;
                            } else {
                                reason = Some("distribution weight exceeds 2147483647".into());
                                break;
                            }
                        }
                    }
                } else {
                    reason = Some("expected weighted list".into());
                }
            }
            Kind::Transient => reason = Some("component is not persistent".into()),
            Kind::NbtPath => {
                if !value
                    .as_str()
                    .is_some_and(|s| crate::nbt_path::NbtPath::parse(s).is_ok())
                {
                    reason = Some("expected valid NBT path string".into());
                }
            }
            Kind::Bool => {
                if !value.is_boolean() {
                    reason = Some("expected boolean".into());
                }
            }
            Kind::Number => {
                if !value.is_number() {
                    reason = Some("expected number".into());
                }
            }
            Kind::NonAirItemId => {
                if !value.as_str().is_some_and(|s| {
                    sunrise_core::identifier::Identifier::parse(s)
                        .is_ok_and(|id| id.namespace() != "minecraft" || id.path() != "air")
                }) {
                    reason = Some("expected non-air item identifier".into());
                }
            }
            Kind::PlayerName => {
                if !value
                    .as_str()
                    .is_some_and(|s| s.len() <= 16 && s.bytes().all(|b| (33..127).contains(&b)))
                {
                    reason = Some(
                        "expected player name of at most 16 printable ASCII characters".into(),
                    );
                }
            }
            Kind::TypedEntityData => {
                let valid = if let Some(object) = value.as_object() {
                    object
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(identifier)
                } else if let Some(text) = value.as_str() {
                    crate::custom_data::decode(sunrise_core::nbt::Tag::String(text.into())).is_ok_and(|tag| {
                        matches!(tag.get("id"), Some(sunrise_core::nbt::Tag::String(id)) if identifier(&id.to_string_lossy()))
                    })
                } else {
                    false
                };
                if !valid {
                    reason = Some("expected compound with entity type identifier id".into());
                }
            }
            Kind::AttributeNumber(float) => {
                let valid = value
                    .as_str()
                    .and_then(|id| sunrise_core::identifier::Identifier::parse(id).ok())
                    .and_then(|id| {
                        (id.namespace() == "minecraft")
                            .then(|| crate::environment_attributes::attribute(id.path()))
                            .flatten()
                    })
                    .is_some_and(|attribute| {
                        float
                            && matches!(
                                attribute.kind,
                                crate::environment_attributes::Kind::Float
                                    | crate::environment_attributes::Kind::AngleDegrees
                            )
                    });
                // The owning registry currently contains no INTEGER attributes.
                if !valid {
                    reason =
                        Some("environment attribute has no requested numeric conversion".into());
                }
            }
            Kind::EnvironmentCheck => {
                let shape = value
                    .get("attribute")
                    .and_then(Value::as_str)
                    .and_then(environment::value);
                match (shape, value.get("value").filter(|value| !value.is_null())) {
                    (Some(shape), Some(value)) => push(shape, value, path.field("value")),
                    (None, _) => reason = Some("unknown or missing environment attribute".into()),
                    (_, None) => reason = Some("missing required field value".into()),
                }
            }
            Kind::All(rules) => {
                for rule in rules {
                    push(*rule, value, path.clone());
                }
            }
            Kind::Tag => {
                if !value
                    .as_str()
                    .and_then(|s| s.strip_prefix('#'))
                    .is_some_and(identifier)
                {
                    reason = Some("expected registry tag reference".into());
                }
            }
            Kind::RegistryEnum(names) => {
                if !value.as_str().is_some_and(|value| {
                    sunrise_core::identifier::Identifier::parse(value)
                        .is_ok_and(|id| id.namespace() == "minecraft" && names.contains(&id.path()))
                }) {
                    reason = Some("unknown registry value".into());
                }
            }
            Kind::RegistryMap(rows) => {
                if let Some(object) = value.as_object() {
                    for (id, value) in object {
                        if let Some(row) = matching_row(rows, id, true) {
                            push(row.kind, value, path.field(id));
                        } else {
                            reason = Some(format!("unknown predicate type {id}"));
                            break;
                        }
                    }
                } else {
                    reason = Some("expected predicate map".into());
                }
            }
            Kind::FloatRange(min, max) => {
                if !value.as_number().is_some_and(|number| {
                    let number = double_value(number) as f32;
                    number.total_cmp(&(min as f32)).is_ge()
                        && number.total_cmp(&(max as f32)).is_le()
                }) {
                    reason = Some(format!("expected float in {min}..={max}"));
                }
            }
            Kind::Ordered(min, max, exclusive) | Kind::OrderedFloat(min, max, exclusive) => {
                if let (Some(min), Some(max)) = (value[min].as_number(), value[max].as_number()) {
                    let convert = |number| {
                        let number = double_value(number);
                        if matches!(rule, Kind::OrderedFloat(..)) {
                            f64::from(number as f32)
                        } else {
                            number
                        }
                    };
                    let (min, max) = (convert(min), convert(max));
                    if min > max || (exclusive && min == max) {
                        reason = Some("bounds are out of order".into());
                    }
                }
            }
            Kind::OrderedInt(min, max) => {
                if let (Some(min), Some(max)) = (value[min].as_number(), value[max].as_number())
                    && int_value(min) > int_value(max)
                {
                    reason = Some("bounds are out of order".into());
                }
            }
            Kind::HexColor(size) => {
                if !value
                    .as_str()
                    .and_then(|s| s.strip_prefix('#'))
                    .is_some_and(|s| s.len() == size && s.bytes().all(|c| c.is_ascii_hexdigit()))
                {
                    reason = Some(format!("expected #{size}-digit hexadecimal color"));
                }
            }
            Kind::PositiveWeights => {
                let sum = value.as_array().and_then(|values| {
                    values.iter().try_fold(0_i32, |sum, value| {
                        let weight = int_value(value["weight"].as_number()?) as i32;
                        (weight >= 0).then(|| sum.checked_add(weight)).flatten()
                    })
                });
                if !sum.is_some_and(|sum| sum > 0) {
                    reason = Some("distribution weight total must be in 1..=2147483647".into());
                }
            }
            Kind::IntRange(min, max) => {
                // Codec.INT reads Number.intValue; fractional inputs are not forbidden.
                let number = value.as_number().map(int_value);
                if !number.is_some_and(|n| n >= min && n <= max) {
                    reason = Some(format!("expected integer value in {min}..={max}"));
                }
            }
            Kind::String(max) => {
                if !value
                    .as_str()
                    .is_some_and(|s| s.encode_utf16().count() <= max)
                {
                    reason = Some(format!("expected string of at most {max} UTF-16 units"));
                }
            }
            Kind::Id => {
                if !value.as_str().is_some_and(identifier) {
                    reason = Some("expected resource identifier".into());
                }
            }
            Kind::Enum(values) => {
                if !value.as_str().is_some_and(|s| values.contains(&s)) {
                    reason = Some(format!("expected one of {}", values.join(", ")));
                }
            }
            Kind::Object(fields) => {
                if let Some(object) = value.as_object() {
                    for field in fields {
                        let child_path = path.field(field.name);
                        match object.get(field.name).filter(|v| !v.is_null()) {
                            Some(value) => push(field.kind, value, child_path),
                            None if field.required => {
                                path = child_path;
                                reason = Some(format!("missing required field {}", field.name));
                                break;
                            }
                            None => {}
                        }
                    }
                } else {
                    reason = Some("expected object".into());
                }
            }
            Kind::List(element, min, max) => {
                if let Some(list) = value
                    .as_array()
                    .filter(|list| list.len() >= min && list.len() <= max)
                {
                    for (index, value) in list.iter().enumerate() {
                        push(*element, value, path.index(index));
                    }
                } else {
                    reason = Some(format!("expected list with {min}..={max} elements"));
                }
            }
            Kind::Map(element)
            | Kind::IdMap(element)
            | Kind::SlotMap(element)
            | Kind::MapMax(element, _)
            | Kind::EnumMap(_, element) => {
                if let Some(object) = value.as_object() {
                    if let Kind::MapMax(_, maximum) = rule
                        && object.len() > maximum
                    {
                        reason = Some(format!("expected map with at most {maximum} keys"));
                    }
                    for (key, value) in object {
                        if matches!(rule, Kind::IdMap(_)) && !identifier(key) {
                            reason = Some(format!("invalid identifier key {key}"));
                            break;
                        }
                        if matches!(rule, Kind::SlotMap(_)) && !slot_range(key) {
                            reason = Some(format!("unknown slot range {key}"));
                            break;
                        }
                        if let Kind::EnumMap(names, _) = rule
                            && !names.contains(&key.as_str())
                        {
                            reason = Some(format!("unknown map key {key}"));
                            break;
                        }
                        push(*element, value, path.field(key));
                    }
                } else {
                    reason = Some("expected map".into());
                }
            }
            Kind::ComponentMap(_) | Kind::ComponentPredicates => {
                let patch = matches!(rule, Kind::ComponentMap(true));
                if let Some(object) = value.as_object() {
                    for (key, value) in object {
                        let (removed, id) = if patch {
                            key.strip_prefix('!')
                                .map_or((false, key.as_str()), |id| (true, id))
                        } else {
                            (false, key.as_str())
                        };
                        let Ok(parsed) = sunrise_core::identifier::Identifier::parse(id) else {
                            reason = Some(format!("invalid component identifier {key}"));
                            break;
                        };
                        let minecraft = parsed.namespace() == "minecraft";
                        let known = minecraft
                            .then(|| {
                                components::VALUES
                                    .iter()
                                    .find(|row| row.name == parsed.path())
                                    .map(|row| row.kind)
                            })
                            .flatten();
                        let shape = if removed {
                            known
                                .filter(|kind| !matches!(kind, Kind::Transient))
                                .map(|_| Kind::Object(&[]))
                        } else if matches!(rule, Kind::ComponentPredicates) {
                            minecraft
                                .then(|| {
                                    component_predicates::TYPES
                                        .iter()
                                        .find(|row| row.name == parsed.path())
                                        .map(|row| row.kind)
                                })
                                .flatten()
                                .or_else(|| known.map(|_| Kind::Object(&[])))
                        } else {
                            known
                        };
                        if let Some(shape) = shape {
                            push(shape, value, path.field(key));
                        } else {
                            reason = Some(format!("unknown or nonpersistent component {id}"));
                            break;
                        }
                    }
                } else {
                    reason = Some("expected component map".into());
                }
            }
            Kind::Named(name) => {
                if let Some(rule) = named(name) {
                    push(rule, value, path.clone());
                } else {
                    reason = Some(format!("missing internal codec shape {name}"));
                }
            }
            Kind::Switch(field, qualified, rows) => {
                let tag = value.get(field).and_then(Value::as_str);
                if let Some(row) = tag.and_then(|tag| matching_row(rows, tag, qualified)) {
                    let name = if qualified {
                        format!("minecraft:{}", row.name)
                    } else {
                        row.name.to_owned()
                    };
                    pending.push(Work::Check(Task {
                        value,
                        kind: row.kind,
                        path: path.clone(),
                        owner: format!("{owner} -> {name}"),
                    }));
                } else {
                    reason = Some(format!(
                        "unknown or missing {field}: {}",
                        tag.unwrap_or("<missing>")
                    ));
                }
            }
            Kind::Either(variants) => {
                if purpose == Purpose::Locate {
                    for kind in variants {
                        push(*kind, value, path.clone());
                    }
                    continue;
                }
                let mut remaining = Variants::Kinds(variants.iter());
                if let Some(first) = remaining.next() {
                    alternatives.push(Alternative {
                        value,
                        remaining,
                        path: path.clone(),
                        pending: pending.len(),
                        children: children.len(),
                        owner: owner.clone(),
                        failure: None,
                    });
                    pending.push(Work::EndAlternative);
                    pending.push(Work::Check(Task {
                        value,
                        kind: first,
                        path: path.clone(),
                        owner: owner.clone(),
                    }));
                } else {
                    reason = Some("no accepted codec alternative".into());
                }
            }
            Kind::FuzzySwitch(field, rows) => {
                if purpose == Purpose::Locate && value.get(field).is_none_or(Value::is_null) {
                    for row in rows {
                        push(row.kind, value, path.clone());
                    }
                    continue;
                }
                if value.get(field).is_some_and(|v| !v.is_null()) {
                    push(Kind::Switch(field, false, rows), value, path.clone());
                } else {
                    let mut remaining = Variants::Rows(rows.iter());
                    if let Some(first) = remaining.next() {
                        alternatives.push(Alternative {
                            value,
                            remaining,
                            path: path.clone(),
                            pending: pending.len(),
                            children: children.len(),
                            owner: owner.clone(),
                            failure: None,
                        });
                        pending.push(Work::EndAlternative);
                        pending.push(Work::Check(Task {
                            value,
                            kind: first,
                            path: path.clone(),
                            owner: owner.clone(),
                        }));
                    } else {
                        reason = Some("no matching codec".into());
                    }
                }
            }
            Kind::NotBoth(left, right) => {
                if value[left] == true && value[right] == true {
                    reason = Some(format!("{left} and {right} cannot both be true"));
                }
            }
            Kind::TextColor => {
                let valid = value.as_str().is_some_and(|s| {
                    if let Some(hex) = s.strip_prefix('#') {
                        let negative = hex.starts_with('-');
                        let digits = hex.strip_prefix(['+', '-']).unwrap_or(hex);
                        !digits.is_empty()
                            && digits
                                .encode_utf16()
                                .try_fold(0_i32, |value, unit| {
                                    value.checked_mul(16)?.checked_add(
                                        sunrise_core::java_character::digit(unit, 16)? as i32,
                                    )
                                })
                                .is_some_and(|n| {
                                    (0..=0xffffff).contains(&n) && (!negative || n == 0)
                                })
                    } else {
                        [
                            "black",
                            "dark_blue",
                            "dark_green",
                            "dark_aqua",
                            "dark_red",
                            "dark_purple",
                            "gold",
                            "gray",
                            "dark_gray",
                            "blue",
                            "green",
                            "aqua",
                            "red",
                            "light_purple",
                            "yellow",
                            "white",
                        ]
                        .contains(&s)
                    }
                });
                if !valid {
                    reason = Some("invalid text color".into());
                }
            }
            Kind::Snbt => {
                let valid = value.is_object()
                    || value.as_str().is_some_and(|text| {
                        crate::custom_data::validate(&sunrise_core::nbt::Tag::String(text.into()))
                            .is_ok()
                    });
                if !valid {
                    reason = Some("expected compound or compound SNBT string".into());
                }
            }
            Kind::Function | Kind::Condition | Kind::HolderSet(_) => {
                let valid = value.is_object()
                    || value.is_string()
                    || (!matches!(rule, Kind::Condition) && value.is_array());
                if !valid {
                    reason =
                        Some("expected typed object, reference or permitted holder list".into());
                } else if purpose == Purpose::Validate
                    && matches!(rule, Kind::HolderSet(_))
                    && value
                        .as_str()
                        .and_then(|id| id.strip_prefix('#'))
                        .is_some_and(|id| !identifier(id))
                {
                    reason = Some("invalid holder-set tag identifier".into());
                } else {
                    children.push(Child {
                        value,
                        path: path.diagnostic.clone(),
                        pointer: path.pointer.clone(),
                        kind: match rule {
                            Kind::Condition => ChildKind::Condition,
                            Kind::HolderSet(function) => ChildKind::Set(function),
                            _ => ChildKind::Function,
                        },
                    });
                }
            }
        }
        if let Some(reason) = reason {
            if purpose == Purpose::Locate {
                continue;
            }
            let mut failure = (path, owner, reason);
            let mut retry = false;
            while let Some(alternative) = alternatives.last_mut() {
                // Preserve a nested field refusal over a later, generic mismatch
                // from another branch (for example an inline table versus a tag).
                if alternative.failure.as_ref().is_none_or(|previous| {
                    failure.0.pointer.matches('/').count() > previous.0.pointer.matches('/').count()
                }) {
                    alternative.failure = Some(failure.clone());
                }
                pending.truncate(alternative.pending);
                children.truncate(alternative.children);
                if let Some(first) = alternative.remaining.next() {
                    pending.push(Work::EndAlternative);
                    pending.push(Work::Check(Task {
                        value: alternative.value,
                        kind: first,
                        path: alternative.path.clone(),
                        owner: alternative.owner.clone(),
                    }));
                    retry = true;
                    break;
                }
                failure = alternative.failure.take().unwrap_or(failure);
                alternatives.pop();
            }
            if !retry {
                return Err(invalid(&failure.0, &failure.1, failure.2));
            }
        }
    }
    Ok(children)
}
