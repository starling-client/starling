//! Load-time codec binding and direct rule dispatch, shared by both loot evaluators.
use crate::loot::{LootContext, LootStack, LootStackEditAccess, LootTags};
use serde_json::Value;
#[cfg(test)]
#[path = "loot_types/argument_tests.rs"]
mod argument_tests;
#[path = "loot_types/normalize.rs"]
mod normalize;
#[path = "loot_types/parameters.rs"]
mod parameters;
#[path = "loot_types/shapes/mod.rs"]
mod shapes;
/// Whether `kind` (`minecraft:<name>`) is a vanilla `LootItemCondition` type.
pub fn is_vanilla_condition(kind: &str) -> bool {
    shapes::contains(kind, false)
}
pub(crate) use shapes::validate_environment_value;
#[cfg(test)]
#[path = "loot_types/tests.rs"]
mod tests;
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::Arc,
};
/// Object-safe borrowing of the actual engine random; no cloned or seeded stream.
pub trait RollRandom {
    fn next_int(&mut self) -> i32;
    fn next_int_bound(&mut self, bound: i32) -> i32;
    fn next_long(&mut self) -> i64;
    fn next_float(&mut self) -> f32;
    fn next_double(&mut self) -> f64;
    fn next_boolean(&mut self) -> bool;
    fn next_gaussian(&mut self) -> f64;
}
impl<R: sunrise_core::random::RandomSource> RollRandom for R {
    fn next_int(&mut self) -> i32 {
        sunrise_core::random::RandomSource::next_int(self)
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        sunrise_core::random::RandomSource::next_int_bound(self, bound).unwrap_or(0)
    }
    fn next_long(&mut self) -> i64 {
        sunrise_core::random::RandomSource::next_long(self)
    }
    fn next_float(&mut self) -> f32 {
        sunrise_core::random::RandomSource::next_float(self)
    }
    fn next_double(&mut self) -> f64 {
        sunrise_core::random::RandomSource::next_double(self)
    }
    fn next_boolean(&mut self) -> bool {
        sunrise_core::random::RandomSource::next_boolean(self)
    }
    fn next_gaussian(&mut self) -> f64 {
        sunrise_core::random::RandomSource::next_gaussian(self)
    }
}

static CAPTURE: std::sync::OnceLock<fn() -> Types> = std::sync::OnceLock::new();
/// Install the executable's registry adapter before any loot data loads.
pub fn install(capture: fn() -> Types) -> Result<(), fn() -> Types> {
    CAPTURE.set(capture)
}
impl Types {
    pub fn capture() -> Self {
        CAPTURE
            .get()
            .map_or_else(Self::default, |capture| capture())
    }
}

/// A bound, decoded loot function rule supplied by the executable's codec registry.
pub trait BoundFunction: Send + Sync {
    fn apply(
        &self,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
        stack: &mut dyn LootStackEditAccess,
    ) -> Result<(), String>;
}

/// A bound, decoded loot condition rule supplied by the executable's codec registry.
pub trait BoundCondition: Send + Sync {
    fn test(
        &self,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
    ) -> Result<bool, String>;
}

/// Decoded loot function rule and its declared context parameters.
pub struct DecodedFunction {
    pub rule: Arc<dyn BoundFunction>,
    pub referenced_params: Vec<String>,
}

/// Decoded loot condition rule and its declared context parameters.
pub struct DecodedCondition {
    pub rule: Arc<dyn BoundCondition>,
    pub referenced_params: Vec<String>,
}

/// Frozen static codec registry captured by one owning data generation.
/// The implementation is supplied by the executable; world never depends on mod API types.
pub trait Registry: Send + Sync {
    fn contains(&self, kind: &str, function: bool) -> bool;
    fn conditional(&self, kind: &str) -> bool;
    fn decode_function(&self, kind: &str, data: &Value) -> Result<DecodedFunction, String>;
    fn decode_condition(&self, kind: &str, data: &Value) -> Result<DecodedCondition, String>;
}

/// Immutable codec registry captured before data loads.
/// Generation-specific decoded rules live separately in [`Bindings`].
#[derive(Clone, Default)]
pub struct Types {
    registry: Option<Arc<dyn Registry>>,
}
impl Types {
    pub fn new(registry: Arc<dyn Registry>) -> Self {
        Self {
            registry: Some(registry),
        }
    }
    pub fn contains(&self, kind: &str, function: bool) -> bool {
        self.registry
            .as_ref()
            .is_some_and(|registry| registry.contains(kind, function))
    }
    pub fn conditional(&self, kind: &str) -> bool {
        self.registry
            .as_ref()
            .is_none_or(|registry| registry.conditional(kind))
    }
    /// Decode one mod-registered condition node, for an evaluator that owns its own tree walk
    /// (an advancement's `player` predicate). `None` when `kind` is not registered by a mod;
    /// a decode the registry refuses is an error naming `path`.
    pub fn bind_registered_condition(
        &self,
        kind: &str,
        data: &Value,
        path: &str,
    ) -> io::Result<Option<BoundEntry<dyn BoundCondition>>> {
        let Some(registry) = self
            .registry
            .as_ref()
            .filter(|registry| registry.contains(kind, false))
        else {
            return Ok(None);
        };
        let decoded = registry
            .decode_condition(kind, data)
            .map_err(|reason| invalid(path, kind, reason))?;
        Ok(Some(BoundEntry {
            kind: kind.into(),
            location: Arc::from(path),
            referenced_params: decoded.referenced_params.into(),
            rule: decoded.rule,
        }))
    }

    /// Decode an inline `LootItemCondition.CODEC` field embedded in another record
    /// against the active predicate registry (`RegistryFileCodec.decode`).
    pub fn validate_condition_field(
        &self,
        value: &Value,
        predicates: &HashMap<String, Value>,
        path: &str,
    ) -> io::Result<()> {
        Walker::new(
            self,
            Tables::Map(HashMap::new()),
            predicates,
            &HashMap::new(),
        )
        .entry(value, false, path)
    }

    /// Validate and decode raw tables, predicates and modifiers without retaining bindings.
    pub fn validate<'a>(
        &self,
        tables: impl Iterator<Item = (&'a str, &'a Value)>,
        predicates: &HashMap<String, Value>,
        modifiers: &HashMap<String, Value>,
    ) -> io::Result<()> {
        let tables: HashMap<_, _> = tables.collect();
        Walker::new(self, Tables::Map(tables), predicates, modifiers).generation()
    }

    /// Decode and bind every custom node in one final immutable generation.
    /// The returned [`Bindings`] retains `tables`, `predicates`, `modifiers` and
    /// `enchantment_effects` alongside the address-indexed rule table so no indexed
    /// `Value` address can ever outlive its owning document.
    pub fn bind_generation(
        &self,
        tables: Arc<super::LootTables>,
        predicates: Arc<HashMap<String, Value>>,
        modifiers: Arc<HashMap<String, Value>>,
        enchantment_effects: Vec<(String, Arc<Value>)>,
        normalization: Normalization,
    ) -> io::Result<Bindings> {
        let rules = {
            let mut walker = Walker::new(self, Tables::Loaded(&tables), &predicates, &modifiers);
            walker.generation()?;
            let enchanted_damage =
                parameters::allowed(&Value::String("minecraft:enchanted_damage".into()));
            walker.allowed = enchanted_damage;
            for (name, effects) in &enchantment_effects {
                for (i, effect) in effects["minecraft:equipment_drops"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    if let Some(req) = effect.get("requirements").filter(|value| !value.is_null()) {
                        walker.entry(
                            req,
                            false,
                            &format!(
                                "enchantment {name}.effects.minecraft:equipment_drops[{i}].requirements"
                            ),
                        )?;
                    }
                }
            }
            walker.rules
        };
        Ok(Bindings {
            types: self.clone(),
            inner: Arc::new(BindingsInner {
                tables,
                predicates,
                modifiers,
                _enchantment_effects: enchantment_effects
                    .into_iter()
                    .map(|(_, effects)| effects)
                    .collect(),
                normalization,
                rules,
            }),
        })
    }
}

pub struct BoundEntry<T: ?Sized> {
    kind: Box<str>,
    location: Arc<str>,
    referenced_params: Arc<[String]>,
    rule: Arc<T>,
}

impl BoundEntry<dyn BoundFunction> {
    pub fn apply(
        &self,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
        stack: &mut dyn LootStackEditAccess,
    ) {
        if let Err(reason) = self.rule.apply(ctx, random, tags, stack) {
            log::warn!(
                "loot function type {} ({}): {reason}",
                self.kind,
                self.location
            );
        }
    }
}
impl BoundEntry<dyn BoundCondition> {
    pub fn test(
        &self,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
    ) -> bool {
        match self.rule.test(ctx, random, tags) {
            Ok(value) => value,
            Err(reason) => {
                log::warn!(
                    "loot condition type {} ({}): {reason}",
                    self.kind,
                    self.location
                );
                false
            }
        }
    }
}

/// Side table of decoded rules keyed by the address of their exact final immutable `Value` node.
/// Addresses are opaque identities, never dereferenced, and never outlive their owning documents.
#[derive(Default)]
pub struct RuleMap {
    functions: HashMap<usize, BoundEntry<dyn BoundFunction>>,
    conditions: HashMap<usize, BoundEntry<dyn BoundCondition>>,
}
impl RuleMap {
    fn referenced_params(&self, function: bool, key: usize) -> Option<Arc<[String]>> {
        if function {
            self.functions
                .get(&key)
                .map(|entry| Arc::clone(&entry.referenced_params))
        } else {
            self.conditions
                .get(&key)
                .map(|entry| Arc::clone(&entry.referenced_params))
        }
    }
}

/// Immutable generation bundle retaining the final normalized documents together with their
/// decoded loot rules. Shared from [`super::LootData`] to `Items::set_loot_data`.
#[derive(Clone)]
pub struct Bindings {
    types: Types,
    inner: Arc<BindingsInner>,
}
struct BindingsInner {
    rules: RuleMap,
    tables: Arc<super::LootTables>,
    predicates: Arc<HashMap<String, Value>>,
    modifiers: Arc<HashMap<String, Value>>,
    _enchantment_effects: Vec<Arc<Value>>,
    normalization: Normalization,
}

/// The merged holder sets and tags used when binding inline data to a loot generation.
#[derive(Default)]
pub struct Normalization {
    holder_sets: Arc<super::holder_sets::Binding>,
    tags: Arc<super::loot_tags::BoundTags>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self {
            types: Types::default(),
            inner: Arc::new(BindingsInner {
                tables: Arc::default(),
                predicates: Arc::default(),
                modifiers: Arc::default(),
                _enchantment_effects: Vec::new(),
                normalization: Normalization::default(),
                rules: RuleMap::default(),
            }),
        }
    }
}
impl Bindings {
    pub fn conditional(&self, kind: &str) -> bool {
        self.types.conditional(kind)
    }

    pub fn function<'a>(
        &'a self,
        local: Option<&'a RuleMap>,
        kind: &str,
        data: &Value,
    ) -> Option<&'a BoundEntry<dyn BoundFunction>> {
        let key = std::ptr::from_ref(data) as usize;
        let entry = local
            .and_then(|map| map.functions.get(&key))
            .or_else(|| self.inner.rules.functions.get(&key));
        if entry.is_none() && self.types.contains(kind, true) {
            log::error!(
                "loot function type {kind}: missing prepared binding (preparation/integration error)"
            );
        }
        entry
    }

    pub fn condition<'a>(
        &'a self,
        local: Option<&'a RuleMap>,
        kind: &str,
        data: &Value,
    ) -> Option<&'a BoundEntry<dyn BoundCondition>> {
        let key = std::ptr::from_ref(data) as usize;
        let entry = local
            .and_then(|map| map.conditions.get(&key))
            .or_else(|| self.inner.rules.conditions.get(&key));
        if entry.is_none() && self.types.contains(kind, false) {
            log::error!(
                "loot condition type {kind}: missing prepared binding (preparation/integration error)"
            );
        }
        entry
    }
    /// Normalize and bind an inline loot table at the argument boundary before drawing,
    /// reusing named generation bindings without re-decoding them.
    pub fn prepare_inline_table(&self, table: &Value) -> io::Result<PreparedInlineTable> {
        let mut boxed = Box::new(table.clone());
        let normalization = &self.inner.normalization;
        normalization.holder_sets.walk(
            "inline loot_table",
            &mut boxed,
            &normalization.tags,
            &self.types,
            None,
        )?;
        normalize::normalize_table(&mut boxed, &normalization.tags, &self.types);
        let rules = {
            let mut walker = Walker::new(
                &self.types,
                Tables::Loaded(&self.inner.tables),
                &self.inner.predicates,
                &self.inner.modifiers,
            );
            walker.inherited = Some(&self.inner.rules);
            // Programmatic roots keep their evaluator's stock-kind handling; bind
            // custom nodes at the same schema-owned edges as resource files.
            walker.strict = false;
            walker.table(&boxed, "inline loot_table $")?;
            walker.rules
        };
        Ok(PreparedInlineTable {
            value: boxed,
            rules,
        })
    }
    /// Normalize and bind a standalone condition (such as a global loot modifier's `condition`),
    /// reusing named predicate bindings from this generation without re-decoding them.
    pub fn prepare_condition(&self, condition: Value, path: &str) -> io::Result<PreparedCondition> {
        let mut boxed = Box::new(condition);
        let normalization = &self.inner.normalization;
        normalization.holder_sets.walk(
            path,
            &mut boxed,
            &normalization.tags,
            &self.types,
            Some(false),
        )?;
        normalize::normalize_condition(&mut boxed, &normalization.tags, &self.types);
        let rules = {
            let mut walker = Walker::new(
                &self.types,
                Tables::Loaded(&self.inner.tables),
                &self.inner.predicates,
                &self.inner.modifiers,
            );
            walker.inherited = Some(&self.inner.rules);
            walker.entry(&boxed, false, path)?;
            walker.rules
        };
        Ok(PreparedCondition {
            value: boxed,
            rules,
        })
    }
}

/// A normalized inline loot table paired with its locally bound custom rules.
/// Keeps the normalized `Value` on the heap so its node addresses stay stable across moves.
pub struct PreparedInlineTable {
    rules: RuleMap,
    value: Box<Value>,
}
impl PreparedInlineTable {
    pub fn value(&self) -> &Value {
        &self.value
    }
    pub fn rules(&self) -> &RuleMap {
        &self.rules
    }
}

/// A normalized condition node (for example a global loot modifier's `condition`) paired
/// with its locally bound custom rules. Named predicate references resolve through the
/// owning generation's [`Bindings`].
pub struct PreparedCondition {
    rules: RuleMap,
    value: Box<Value>,
}
impl PreparedCondition {
    pub fn value(&self) -> &Value {
        &self.value
    }
    pub fn rules(&self) -> &RuleMap {
        &self.rules
    }
}

impl super::LootData {
    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }
    pub fn modifiers(&self) -> &Arc<HashMap<String, Value>> {
        &self.modifiers
    }
}

fn resource(kind: &str, id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("{kind} {id} [data/{namespace}/{kind}/{path}.json] $")
}
fn invalid(path: &str, kind: &str, reason: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{path} (type {kind}): {reason}"),
    )
}

type AllowedParameters = Option<&'static [&'static str]>;

enum Tables<'a> {
    Map(HashMap<&'a str, &'a Value>),
    Loaded(&'a super::LootTables),
}
impl<'a> Tables<'a> {
    fn get(&self, id: &str) -> Option<&'a Value> {
        match self {
            Self::Map(map) => map.get(id).copied(),
            Self::Loaded(tables) => tables.get(id),
        }
    }
    fn ids(&self) -> Vec<&'a str> {
        match self {
            Self::Map(map) => map.keys().copied().collect(),
            Self::Loaded(tables) => tables.ids().collect(),
        }
    }
}

struct Walker<'a> {
    types: &'a Types,
    tables: Tables<'a>,
    visiting_tables: HashSet<String>,
    predicates: &'a HashMap<String, Value>,
    modifiers: &'a HashMap<String, Value>,
    visiting: HashSet<(bool, String)>,
    validated_predicates: HashMap<AllowedParameters, HashSet<String>>,
    inherited: Option<&'a RuleMap>,
    strict: bool,
    rules: RuleMap,
    allowed: AllowedParameters,
}

#[derive(Clone, Copy)]
enum TablePart {
    Table,
    Pool,
    Entry,
}

enum Visit<'v> {
    Field(&'v Value, bool, String),
    Entry(&'v Value, bool, String),
    Set(&'v Value, bool, String),
    Builtin(&'v Value, bool, String, String),
    NestedList(String),
    RestoreOrigin(String),
    Leave((bool, String)),
    TablePart(&'v Value, TablePart, String),
    TableChildren(&'v Value, TablePart, String),
    TableReference(&'v str, String),
    LeaveTable(String),
}

impl<'a> Walker<'a> {
    fn new(
        types: &'a Types,
        tables: Tables<'a>,
        predicates: &'a HashMap<String, Value>,
        modifiers: &'a HashMap<String, Value>,
    ) -> Self {
        Self {
            types,
            tables,
            visiting_tables: HashSet::new(),
            predicates,
            modifiers,
            visiting: HashSet::new(),
            validated_predicates: HashMap::new(),
            inherited: None,
            strict: true,
            rules: RuleMap::default(),
            allowed: None,
        }
    }

    fn generation(&mut self) -> io::Result<()> {
        let mut table_ids = self.tables.ids();
        table_ids.sort_unstable();
        for id in table_ids {
            if let Some(data) = self.tables.get(id) {
                self.visiting_tables.insert(id.to_owned());
                self.table(data, &resource("loot_table", id))?;
                self.visiting_tables.remove(id);
            }
        }
        let (predicates, modifiers) = (self.predicates, self.modifiers);
        let mut predicate_ids: Vec<_> = predicates.keys().collect();
        predicate_ids.sort_unstable();
        for id in predicate_ids {
            if !self.predicate_validated(id) {
                self.entry(&predicates[id], false, &resource("predicate", id))?;
                self.validated_predicates
                    .entry(None)
                    .or_default()
                    .insert(id.clone());
            }
        }
        let mut modifier_ids: Vec<_> = modifiers.keys().collect();
        modifier_ids.sort_unstable();
        for id in modifier_ids {
            self.entry(&modifiers[id], true, &resource("item_modifier", id))?;
        }
        Ok(())
    }

    fn predicate_validated(&self, id: &str) -> bool {
        self.validated_predicates
            .get(&self.allowed)
            .is_some_and(|ids| ids.contains(id))
    }

    fn table(&mut self, table: &Value, path: &str) -> io::Result<()> {
        let previous = self.allowed;
        self.allowed = parameters::allowed(&table["type"]);
        let result = self.walk(Visit::TablePart(table, TablePart::Table, path.to_owned()));
        self.allowed = previous;
        result
    }

    fn entry(&mut self, data: &Value, function: bool, path: &str) -> io::Result<()> {
        self.walk(Visit::Field(data, function, path.to_owned()))
    }

    /// `Validatable.validateHolder`: verify every reachable holder. Keep table and
    /// typed edges on the same work stack so alternating between them cannot recurse.
    fn walk<'v>(&mut self, first: Visit<'v>) -> io::Result<()>
    where
        'a: 'v,
    {
        let (predicates, modifiers) = (self.predicates, self.modifiers);
        let mut pending = vec![first];
        let mut origin = String::new();
        while let Some(visit) = pending.pop() {
            match visit {
                Visit::Field(data, function, path) => {
                    pending.push(Visit::RestoreOrigin(std::mem::replace(
                        &mut origin,
                        path.clone(),
                    )));
                    pending.push(Visit::Entry(data, function, path));
                }
                Visit::RestoreOrigin(previous) => origin = previous,
                Visit::Entry(data, function, path) => {
                    if let Some(id) = data.as_str() {
                        let id = sunrise_core::identifier::Identifier::parse(id)
                            .map_err(|reason| invalid(&path, id, reason))?
                            .to_string();
                        if !function && self.predicate_validated(&id) {
                            continue;
                        }
                        let target = if function {
                            modifiers.get(&id)
                        } else {
                            predicates.get(&id)
                        };
                        let Some(target) = target else {
                            if self.strict {
                                return Err(invalid(&path, &id, "unknown reference"));
                            }
                            continue;
                        };
                        let key = (function, id);
                        if !self.visiting.insert(key.clone()) {
                            return Err(invalid(&path, &key.1, "reference cycle"));
                        }
                        let path = if function {
                            format!("{path} -> {}", key.1)
                        } else {
                            // Keep the origin and the defining file, not a path string
                            // grown at every edge of a potentially very deep graph.
                            format!("{origin} -> {}", resource("predicate", &key.1))
                        };
                        pending.push(Visit::Leave(key));
                        pending.push(Visit::Entry(target, function, path));
                    } else if data.is_array() && (function || !self.strict) {
                        pending.push(Visit::Set(data, function, path));
                    } else if let Some((kind, builtin)) =
                        self.validate_type(data, function, &path)?
                    {
                        let conditional = function && (builtin || self.types.conditional(&kind));
                        if builtin {
                            pending.push(Visit::Builtin(data, function, kind, path.clone()));
                        }
                        if conditional {
                            // `LootItemConditionalFunction`'s codec reads `condition` alone: an
                            // old `conditions` list is an unknown field, ignored (#5378).
                            if let Some(condition) = data.get("condition").filter(|v| !v.is_null())
                            {
                                pending.push(Visit::Entry(
                                    condition,
                                    false,
                                    format!("{path}.condition"),
                                ));
                            }
                        }
                    }
                }
                Visit::Builtin(data, function, kind, path) => {
                    let children = if self.strict {
                        shapes::validate(data, function, &kind, &path)?
                    } else {
                        shapes::children(data, function, &kind)
                    };
                    for child in children.into_iter().rev() {
                        let path = if self.strict {
                            child.path
                        } else {
                            format!("{path}{}", child.path)
                        };
                        pending.push(match child.kind {
                            shapes::ChildKind::Function => Visit::Entry(child.value, true, path),
                            shapes::ChildKind::Condition => Visit::Entry(child.value, false, path),
                            shapes::ChildKind::Set(function) => {
                                Visit::Set(child.value, function, path)
                            }
                            shapes::ChildKind::Table => match child.value.as_str() {
                                Some(id) => Visit::TableReference(id, path),
                                None => continue,
                            },
                        });
                    }
                }
                Visit::Leave(key) => {
                    self.visiting.remove(&key);
                    if !key.0 {
                        self.validated_predicates
                            .entry(self.allowed)
                            .or_default()
                            .insert(key.1);
                    }
                }
                Visit::Set(data, function, path) => match data.as_array() {
                    Some(list) => {
                        for (i, value) in list.iter().enumerate().rev() {
                            let path = format!("{path}[{i}]");
                            pending.push(if value.is_array() && self.strict {
                                Visit::NestedList(path)
                            } else {
                                Visit::Entry(value, function, path)
                            });
                        }
                    }
                    None => pending.push(Visit::Entry(data, function, path)),
                },
                Visit::NestedList(path) => {
                    return Err(invalid(&path, "holder_set", "nested bare list"));
                }
                Visit::TablePart(data, part, path) => {
                    if matches!(part, TablePart::Table) && !data.is_object() {
                        return Err(invalid(&path, "loot_table", "expected object"));
                    }
                    pending.push(Visit::TableChildren(data, part, path.clone()));
                    // JsonOps map lookup treats JSON null as absent for optionalFieldOf. A pool's or
                    // entry's old `conditions` list is no codec field: ignored, not checked (#5378).
                    if let Some(value) = data.get("modifier").filter(|v| !v.is_null()) {
                        pending.push(Visit::Field(value, true, format!("{path}.modifier")));
                    }
                    if let Some(value) = data.get("condition").filter(|v| !v.is_null()) {
                        pending.push(Visit::Field(value, false, format!("{path}.condition")));
                    }
                }
                Visit::TableChildren(data, part, path) => {
                    let (key, kind, child_part) = match part {
                        TablePart::Table => ("pools", "loot_table", TablePart::Pool),
                        TablePart::Pool => ("entries", "pool", TablePart::Entry),
                        TablePart::Entry => {
                            if data["type"] == "minecraft:loot_table" {
                                let value = &data["value"];
                                if value.is_object() {
                                    pending.push(Visit::TablePart(
                                        value,
                                        TablePart::Table,
                                        format!("{path}.value"),
                                    ));
                                } else if let Some(id) = value.as_str() {
                                    pending
                                        .push(Visit::TableReference(id, format!("{path}.value")));
                                }
                            }
                            ("children", "entry", TablePart::Entry)
                        }
                    };
                    if let Some(children) = data
                        .get(key)
                        .filter(|v| matches!(part, TablePart::Pool) || !v.is_null())
                    {
                        let children = children.as_array().ok_or_else(|| {
                            invalid(&path, kind, format_args!("{key}: expected list"))
                        })?;
                        for (i, child) in children.iter().enumerate().rev() {
                            pending.push(Visit::TablePart(
                                child,
                                child_part,
                                format!("{path}.{key}[{i}]"),
                            ));
                        }
                    }
                }
                Visit::TableReference(id, path) => {
                    let id = sunrise_core::identifier::Identifier::parse(id)
                        .map_err(|reason| invalid(&path, "loot_table", reason))?
                        .to_string();
                    if let Some(table) = self.tables.get(&id)
                        && self.visiting_tables.insert(id.clone())
                    {
                        let path = format!("{path} -> loot_table {id}");
                        pending.push(Visit::LeaveTable(id));
                        // A referenced or inline table inherits the root's context set.
                        pending.push(Visit::TablePart(table, TablePart::Table, path));
                    }
                }
                Visit::LeaveTable(id) => {
                    self.visiting_tables.remove(&id);
                }
            }
        }
        Ok(())
    }

    fn validate_type(
        &mut self,
        data: &Value,
        function: bool,
        path: &str,
    ) -> io::Result<Option<(String, bool)>> {
        let Some(kind) = data.get("type").and_then(Value::as_str) else {
            return if self.strict {
                Err(invalid(path, "<missing>", "expected typed loot object"))
            } else {
                Ok(None)
            };
        };
        let qualified = sunrise_core::identifier::Identifier::parse(kind)
            .map_err(|reason| invalid(path, kind, reason))?
            .to_string();
        let kind = qualified.as_str();
        let builtin = builtin(kind, function);
        let registry = self
            .types
            .registry
            .as_ref()
            .filter(|r| r.contains(kind, function));
        if !builtin && let Some(registry) = registry {
            let key = std::ptr::from_ref(data) as usize;
            let params = if let Some(params) = self
                .inherited
                .and_then(|map| map.referenced_params(function, key))
                .or_else(|| self.rules.referenced_params(function, key))
            {
                params
            } else if function {
                let decoded = registry
                    .decode_function(kind, data)
                    .map_err(|reason| invalid(path, kind, reason))?;
                let params: Arc<[String]> = decoded.referenced_params.into();
                self.rules.functions.insert(
                    key,
                    BoundEntry {
                        kind: kind.into(),
                        location: Arc::from(path),
                        referenced_params: Arc::clone(&params),
                        rule: decoded.rule,
                    },
                );
                params
            } else {
                let decoded = registry
                    .decode_condition(kind, data)
                    .map_err(|reason| invalid(path, kind, reason))?;
                let params: Arc<[String]> = decoded.referenced_params.into();
                self.rules.conditions.insert(
                    key,
                    BoundEntry {
                        kind: kind.into(),
                        location: Arc::from(path),
                        referenced_params: Arc::clone(&params),
                        rule: decoded.rule,
                    },
                );
                params
            };
            if let Some(allowed) = self.allowed {
                for parameter in params.iter() {
                    let name = parameter.strip_prefix("minecraft:").unwrap_or(parameter);
                    if !allowed.contains(&name) {
                        log::warn!(
                            "{path} (type {kind}): context parameter {parameter} is not allowed by this table's context set"
                        );
                    }
                }
            }
        } else if !builtin && self.strict {
            return Err(invalid(path, kind, "unknown loot type"));
        }
        Ok(Some((qualified, builtin)))
    }
}
/// Whether `kind` is a built-in loot function (`function == true`) or built-in loot
/// condition (`function == false`).
pub fn is_builtin(kind: &str, function: bool) -> bool {
    builtin(kind, function)
}

fn builtin(kind: &str, function: bool) -> bool {
    shapes::contains(kind, function)
        || (!function
            && matches!(
                kind,
                "neoforge:loot_table_id" | "neoforge:can_item_perform_ability"
            ))
}

impl<R: sunrise_core::random::RandomSource> super::Context<'_, R> {
    pub(super) fn registered_condition(
        &mut self,
        bindings: Option<&RuleMap>,
        kind: &str,
        data: &Value,
    ) -> bool {
        let Some(bound) = self.data.bindings.condition(bindings, kind, data) else {
            return false;
        };
        let facts = super::query_context::roll_facts(
            &self.query,
            self.place.as_deref(),
            self.entity,
            self.biome,
            self.luck,
        );
        bound.test(&facts, &mut self.random, self.data)
    }
    pub(super) fn registered_function(&mut self, kind: &str, data: &Value, stack: &mut LootStack) {
        let Some(bound) = self.data.bindings.function(self.local_bindings, kind, data) else {
            return;
        };
        let facts = super::query_context::roll_facts(
            &self.query,
            self.place.as_deref(),
            self.entity,
            self.biome,
            self.luck,
        );
        bound.apply(&facts, &mut self.random, self.data, stack);
    }
}

/// Select winning bytes before parsing: an invalid lower-priority file cannot
/// defeat a valid override, and an invalid winner never falls back to a loser.
pub(super) fn load_files(
    root: &std::path::Path,
    overlay: &super::PackOverlay<'_>,
    kind: &str,
    decode: impl Fn(&[u8]) -> serde_json::Result<Value>,
) -> io::Result<HashMap<String, Value>> {
    let mut files = HashMap::new();
    for namespace in crate::json_folder::namespaces(&root.join("data")) {
        let directory = root.join("data").join(&namespace).join(kind);
        let folder = match crate::json_folder::read(&directory) {
            Ok(folder) => folder,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for file in &folder.files {
            if crate::json_folder::valid_resource_path(&namespace, &file.name) {
                files.insert(
                    format!("{namespace}:{}", file.name),
                    file.bytes().map(<[u8]>::to_vec),
                );
            }
        }
    }
    for (id, layers) in overlay.kind(kind) {
        files.insert(id.to_owned(), Ok(super::PackOverlay::top(layers).to_vec()));
    }
    let mut selected = HashMap::new();
    for (id, bytes) in files {
        let path = format!("{kind} {id} $");
        let bytes = bytes.map_err(|reason| invalid(&path, "<unreadable>", reason))?;
        let mut value = decode(&bytes).map_err(|reason| invalid(&path, "<undecodable>", reason))?;
        let admitted = if kind == "loot_table" {
            overlay.conditions.admit_loot_table(&mut value, &id)
        } else {
            overlay.conditions.allows_id(&value, kind, &id)
        };
        if admitted {
            selected.insert(id, value);
        }
    }
    Ok(selected)
}
