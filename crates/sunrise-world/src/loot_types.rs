//! Load validation and direct type dispatch, shared by both loot evaluators.
use crate::loot::{LootContext, LootStack, LootTags};
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
impl super::LootData {
    /// Validate a private candidate before publishing it or replacing prior data.
    pub fn validate_types(&self) -> io::Result<()> {
        self.types.validate(
            self.tables.ids().map(|id| (id, &self.tables[id])),
            &self.predicates,
            &self.modifiers,
        )
    }
    pub fn types(&self) -> &Types {
        &self.types
    }
    pub fn modifiers(&self) -> &HashMap<String, Value> {
        &self.modifiers
    }
}

/// Frozen static codec registry captured by one owning data generation.
/// The implementation is supplied by the executable; world never depends on mod API types.
pub trait Registry: Send + Sync {
    fn contains(&self, kind: &str, function: bool) -> bool;
    fn validate(&self, kind: &str, data: &Value, function: bool) -> Result<Vec<String>, String>;
    fn conditional(&self, kind: &str) -> bool;
    fn apply(
        &self,
        kind: &str,
        data: &Value,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
        stack: &LootStack,
    ) -> Result<LootStack, String>;
    fn test(
        &self,
        kind: &str,
        data: &Value,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
    ) -> Result<bool, String>;
}

/// One data generation retains its codecs and their owning code stores.
#[derive(Clone, Default)]
pub struct Types {
    registry: Option<Arc<dyn Registry>>,
    warned: Arc<[std::sync::Mutex<HashSet<String>>; 2]>,
}
impl Types {
    pub fn new(registry: Arc<dyn Registry>) -> Self {
        Self {
            registry: Some(registry),
            ..Self::default()
        }
    }
    /// Report a callback refusal once per registry type in this retained generation.
    /// Cloned snapshots share the gate; preparing a new generation starts a new gate.
    pub fn warn_once(&self, kind: &str, function: bool, reason: &str) {
        let first = {
            let mut warned = self.warned[usize::from(function)]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if warned.contains(kind) {
                false
            } else {
                warned.insert(kind.to_owned())
            }
        };
        if first {
            let category = if function { "function" } else { "condition" };
            log::warn!("loot {category} type {kind}: {reason}");
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
    pub fn apply(
        &self,
        kind: &str,
        data: &Value,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
        stack: &LootStack,
    ) -> Result<LootStack, String> {
        self.registry
            .as_ref()
            .ok_or_else(|| format!("unknown loot function type {kind}"))?
            .apply(kind, data, ctx, random, tags, stack)
    }
    pub fn test(
        &self,
        kind: &str,
        data: &Value,
        ctx: &LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn LootTags,
    ) -> Result<bool, String> {
        self.registry
            .as_ref()
            .ok_or_else(|| format!("unknown loot condition type {kind}"))?
            .test(kind, data, ctx, random, tags)
    }
    pub fn validate<'a>(
        &self,
        tables: impl Iterator<Item = (&'a str, &'a Value)>,
        predicates: &HashMap<String, Value>,
        modifiers: &HashMap<String, Value>,
    ) -> io::Result<()> {
        let tables: HashMap<_, _> = tables.collect();
        let mut walker = Walker {
            types: self,
            tables: tables.clone(),
            visiting_tables: HashSet::new(),
            predicates,
            modifiers,
            visiting: HashSet::new(),
            validated: HashMap::new(),
            allowed: None,
        };
        for (id, data) in tables {
            walker.visiting_tables.insert(id.to_owned());
            walker.table(data, &resource("loot_table", id))?;
            walker.visiting_tables.remove(id);
        }
        walker.allowed = None;
        for (id, data) in predicates {
            walker.entry(data, false, &resource("predicate", id))?;
        }
        for (id, data) in modifiers {
            walker.entry(data, true, &resource("item_modifier", id))?;
        }
        Ok(())
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
struct Walker<'a> {
    types: &'a Types,
    tables: HashMap<&'a str, &'a Value>,
    visiting_tables: HashSet<String>,
    predicates: &'a HashMap<String, Value>,
    modifiers: &'a HashMap<String, Value>,
    visiting: HashSet<(bool, String)>,
    validated: HashMap<(bool, usize), Vec<String>>,
    allowed: Option<&'static [&'static str]>,
}
impl Walker<'_> {
    fn table(&mut self, table: &Value, path: &str) -> io::Result<()> {
        let previous = self.allowed;
        self.allowed = parameters::allowed(&table["type"]);
        let result = self.contents(table, path);
        self.allowed = previous;
        result
    }
    fn contents(&mut self, table: &Value, path: &str) -> io::Result<()> {
        if !table.is_object() {
            return Err(invalid(path, "loot_table", "expected object"));
        }
        self.fields(table, path)?;
        if let Some(pools) = table.get("pools").filter(|value| !value.is_null()) {
            let pools = pools
                .as_array()
                .ok_or_else(|| invalid(path, "loot_table", "pools: expected list"))?;
            for (i, pool) in pools.iter().enumerate() {
                let path = format!("{path}.pools[{i}]");
                self.fields(pool, &path)?;
                if let Some(entries) = pool.get("entries") {
                    for (i, entry) in entries
                        .as_array()
                        .ok_or_else(|| invalid(&path, "pool", "entries: expected list"))?
                        .iter()
                        .enumerate()
                    {
                        self.container(entry, &format!("{path}.entries[{i}]"))?;
                    }
                }
            }
        }
        Ok(())
    }
    fn container(&mut self, entry: &Value, path: &str) -> io::Result<()> {
        self.fields(entry, path)?;
        if let Some(children) = entry.get("children").filter(|value| !value.is_null()) {
            for (i, child) in children
                .as_array()
                .ok_or_else(|| invalid(path, "entry", "children: expected list"))?
                .iter()
                .enumerate()
            {
                self.container(child, &format!("{path}.children[{i}]"))?;
            }
        }
        if entry["type"] == "minecraft:loot_table" {
            if entry["value"].is_object() {
                self.contents(&entry["value"], &format!("{path}.value"))?;
            } else if let Some(id) = entry["value"].as_str() {
                self.reference_table(id, &format!("{path}.value"))?;
            }
        }
        Ok(())
    }
    fn reference_table(&mut self, id: &str, path: &str) -> io::Result<()> {
        let id = sunrise_core::identifier::Identifier::parse(id)
            .map_err(|reason| invalid(path, "loot_table", reason))?
            .to_string();
        if self.visiting_tables.insert(id.clone()) {
            let result = if let Some(table) = self.tables.get(id.as_str()).copied() {
                self.contents(table, &format!("{path} -> loot_table {id}"))
            } else {
                Ok(())
            };
            self.visiting_tables.remove(&id);
            result?;
        }
        Ok(())
    }
    fn fields(&mut self, data: &Value, path: &str) -> io::Result<()> {
        // JsonOps map lookup treats JSON null as absent for optionalFieldOf.
        if let Some(value) = data.get("condition").filter(|value| !value.is_null()) {
            self.entry(value, false, &format!("{path}.condition"))?;
        }
        if let Some(value) = data.get("modifier").filter(|value| !value.is_null()) {
            self.entry(value, true, &format!("{path}.modifier"))?;
        }
        Ok(())
    }
    fn entry(&mut self, data: &Value, function: bool, path: &str) -> io::Result<()> {
        if let Some(id) = data.as_str() {
            let id = sunrise_core::identifier::Identifier::parse(id)
                .map_err(|reason| invalid(path, id, reason))?
                .to_string();
            let key = (function, id.clone());
            if !self.visiting.insert(key.clone()) {
                return Err(invalid(path, &id, "reference cycle"));
            }
            let value = if function {
                self.modifiers.get(&id)
            } else {
                self.predicates.get(&id)
            }
            .ok_or_else(|| invalid(path, &id, "unknown reference"))?;
            self.entry(value, function, &format!("{path} -> {id}"))?;
            self.visiting.remove(&key);
            return Ok(());
        }
        if function && data.is_array() {
            return self.set(data, true, path);
        }
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(path, "<missing>", "expected typed loot object"))?;
        let qualified = sunrise_core::identifier::Identifier::parse(kind)
            .map_err(|reason| invalid(path, kind, reason))?
            .to_string();
        let kind = qualified.as_str();
        let builtin = builtin(kind, function);
        if !builtin {
            let registry = self
                .types
                .registry
                .as_ref()
                .filter(|r| r.contains(kind, function))
                .ok_or_else(|| invalid(path, kind, "unknown loot type"))?;
            let key = (function, std::ptr::from_ref(data) as usize);
            let params = match self.validated.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                    registry
                        .validate(kind, data, function)
                        .map_err(|reason| invalid(path, kind, reason))?,
                ),
            };
            if let Some(allowed) = self.allowed {
                for parameter in params {
                    let name = parameter.strip_prefix("minecraft:").unwrap_or(parameter);
                    if !allowed.contains(&name) {
                        log::warn!(
                            "{path} (type {kind}): context parameter {parameter} is not allowed by this table's context set"
                        );
                    }
                }
            }
        }
        if function
            && (builtin || self.types.conditional(kind))
            && let Some(condition) = data.get("condition").filter(|value| !value.is_null())
        {
            self.entry(condition, false, &format!("{path}.condition"))?;
        }
        if builtin {
            for child in shapes::validate(data, function, kind, path)? {
                match child.kind {
                    shapes::ChildKind::Function => self.entry(child.value, true, &child.path)?,
                    shapes::ChildKind::Condition => self.entry(child.value, false, &child.path)?,
                    shapes::ChildKind::Set(function) => {
                        self.set(child.value, function, &child.path)?
                    }
                    shapes::ChildKind::Table => {
                        if let Some(id) = child.value.as_str() {
                            self.reference_table(id, &child.path)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn set(&mut self, data: &Value, function: bool, path: &str) -> io::Result<()> {
        if let Some(list) = data.as_array() {
            for (i, value) in list.iter().enumerate() {
                if value.is_array() {
                    return Err(invalid(
                        &format!("{path}[{i}]"),
                        "holder_set",
                        "nested bare list",
                    ));
                }
                self.entry(value, function, &format!("{path}[{i}]"))?;
            }
            Ok(())
        } else {
            self.entry(data, function, path)
        }
    }
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
    pub(super) fn registered_condition(&mut self, kind: &str, data: &Value) -> bool {
        if !self.data.types.contains(kind, false) {
            return false;
        }
        let snapshot = self.roll_facts(0);
        match self
            .data
            .types
            .test(kind, data, &snapshot, &mut self.random, self.data)
        {
            Ok(value) => value,
            Err(reason) => {
                self.data.types.warn_once(kind, false, &reason);
                false
            }
        }
    }
    pub(super) fn registered_function(
        &mut self,
        kind: &str,
        data: &Value,
        stack: LootStack,
    ) -> LootStack {
        if !self.data.types.contains(kind, true) {
            return stack;
        }
        let snapshot = self.roll_facts(0);
        match self
            .data
            .types
            .apply(kind, data, &snapshot, &mut self.random, self.data, &stack)
        {
            Ok(value) => value,
            Err(reason) => {
                self.data.types.warn_once(kind, true, &reason);
                stack
            }
        }
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
            files.insert(
                format!("{namespace}:{}", file.name),
                file.bytes().map(<[u8]>::to_vec),
            );
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
