//! NeoForge's load conditions (`neoforge:conditions`): a data file that has
//! the key loads only when every condition in it holds. A false condition
//! silently omits the file; a decode or handler error goes to the owning loader.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Enabled package IDs captured by the engine for one owning data load.
/// Readers share this immutable view; they never recover IDs from namespaces.
/// Resource selection happens before evaluating ordinary JSON files. Stacked
/// data maps evaluate each layer before merge or replace; tags ignore conditions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoadConditions {
    mods: Vec<String>,
    registered: Option<RegisteredLookup>,
    context: Context,
    diagnostics: Diagnostics,
}

impl LoadConditions {
    /// A vanilla data load has the built-in package only.
    pub const fn vanilla() -> Self {
        Self {
            mods: Vec::new(),
            registered: None,
            context: Context {
                features: None,
                feature_fingerprint: None,
                registries: None,
                tags: None,
            },
            diagnostics: Diagnostics::new(),
        }
    }

    /// Capture IDs visible to the owning load's side. The engine supplies its
    /// enabled-mod inventory's server view for logical-server data.
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut mods: Vec<_> = ids
            .into_iter()
            .filter(|id| *id != "minecraft")
            .map(str::to_owned)
            .collect();
        mods.sort();
        mods.dedup();
        Self {
            mods,
            registered: None,
            context: Context::default(),
            diagnostics: Diagnostics::default(),
        }
    }

    /// Use the recipe source's inventory while retaining this candidate's
    /// registered handlers, evaluation cache and diagnostics.
    pub fn with_mod_inventory<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut view = self.clone();
        view.mods = Self::new(ids).mods;
        if view.mods != self.mods {
            view.diagnostics = match &self.registered {
                Some(registered) => registered
                    .0
                    .inventories
                    .lock()
                    .unwrap()
                    .entry(view.mods.clone())
                    .or_default()
                    .clone(),
                // Without handlers there are no cached answers; keep registry
                // diagnostics shared across the candidate.
                None => self.diagnostics.clone(),
            };
        }
        view
    }

    pub fn mod_loaded(&self, id: &str) -> bool {
        id == "minecraft"
            || self
                .mods
                .binary_search_by(|loaded| loaded.as_str().cmp(id))
                .is_ok()
    }

    /// `IContext.enabledFeatures`: the world's enabled feature flag ids, which
    /// `neoforge:feature_flags_enabled` reads. Empty is vanilla's default set.
    pub fn with_features<'a>(mut self, flags: impl IntoIterator<Item = &'a str>) -> Self {
        let mut flags: BTreeSet<String> = flags.into_iter().map(qualified).collect();
        if flags.is_empty() {
            flags.insert("minecraft:vanilla".to_owned());
        }
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        flags.hash(&mut hasher);
        self.context.feature_fingerprint = Some(hasher.finish());
        self.context.features = Some(Arc::new(flags));
        self
    }

    /// `IContext.registries`: every registry's entry ids (built-in and the
    /// packs' datapack registries), which `neoforge:registered` reads.
    pub fn with_registries<'a, I>(
        mut self,
        registries: impl IntoIterator<Item = (&'a str, I)>,
    ) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.context.registries = Some(Arc::new(Names::new(registries)));
        self
    }

    /// `IContext.getTag`: the tags this load has bound, as each registry's
    /// tags holding at least one entry. Readers that run before the tags load
    /// never get this view, so `neoforge:tag_empty` there omits the file
    /// instead of answering from an older load's tags.
    pub fn with_tags<'a, I>(&self, non_empty: impl IntoIterator<Item = (&'a str, I)>) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut view = self.clone();
        view.context.tags = Some(Arc::new(Names::new(non_empty)));
        view
    }

    /// `ConditionalOps.decodeListWithElementConditions` on one list: drop an
    /// element whose own `neoforge:conditions` fail and strip the key from one
    /// whose conditions hold. Only the lists NeoForge decodes that way carry
    /// element conditions (a loot table's pools, data-map values, overlays and
    /// atlas sources, each read by its own reader); anywhere else a nested
    /// `neoforge:conditions` key is ordinary data and is never evaluated.
    fn filter_list(&self, elements: &mut Vec<Value>, path: &dyn Fn() -> String) -> bool {
        let mut valid = true;
        elements.retain_mut(|element| match self.registry_verdict(element, path) {
            Some(true) => {
                if let Some(fields) = element.as_object_mut() {
                    fields.remove("neoforge:conditions");
                }
                true
            }
            Some(false) => false,
            None => {
                valid = false;
                false
            }
        });
        valid
    }

    /// `allows_id` for a loot table, then its `pools` filtered by their own
    /// conditions: NeoForge decodes a table's pool list with element conditions
    /// (a false pool is dropped, a passing one loses the key). A pool's
    /// `entries` and `functions` and an entry's `children` are plain lists.
    pub(crate) fn admit_loot_table(&self, file: &mut Value, id: &str) -> bool {
        if !self.allows_id(file, "loot_table", id) {
            return false;
        }
        let path = || {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("data/{namespace}/loot_table/{path}.json")
        };
        file.get_mut("pools")
            .and_then(Value::as_array_mut)
            .is_none_or(|pools| self.filter_list(pools, &path))
    }

    /// Attach the engine's immutable registration table to this candidate load.
    pub fn with_lookup<S, F>(mut self, handlers: impl IntoIterator<Item = (S, F)>) -> Self
    where
        S: Into<String>,
        F: Fn(&Value) -> Result<bool, String> + Send + Sync + 'static,
    {
        let handlers: Handlers = handlers
            .into_iter()
            .map(|(id, handler)| (id.into(), Box::new(handler) as Box<ConditionHandler>))
            .collect();
        if handlers.is_empty() {
            return self;
        }
        if self.diagnostics.0.is_none() {
            self.diagnostics = Diagnostics::default();
        }
        self.registered = Some(RegisteredLookup(Arc::new(LookupState {
            handlers,
            inventories: std::sync::Mutex::new(BTreeMap::from([(
                self.mods.clone(),
                self.diagnostics.clone(),
            )])),
        })));
        self
    }

    /// Whether this candidate has any registered condition handlers.
    pub fn has_registered(&self) -> bool {
        self.registered.is_some()
    }

    pub fn allows(&self, file: &Value) -> bool {
        self.allows_file(file, "data file")
    }

    /// Evaluate a resource, constructing its path only for registered handlers or diagnostics.
    pub fn allows_id(&self, file: &Value, kind: &str, id: &str) -> bool {
        self.allows_context(file, || {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("data/{namespace}/{kind}/{path}.json")
        })
    }

    /// Evaluate a resource whose namespace and path are stored separately.
    pub fn allows_parts(&self, file: &Value, kind: &str, namespace: &str, path: &str) -> bool {
        self.allows_context(file, || format!("data/{namespace}/{kind}/{path}.json"))
    }

    /// Evaluate a registry entry. Its owning load checks accumulated errors
    /// before it publishes the registry.
    pub fn allows_file(&self, file: &Value, path: &str) -> bool {
        self.allows_context(file, || path.to_owned())
    }

    /// Return a file's error without adding it to registry diagnostics. The
    /// caller decides whether to skip the file or refuse its own candidate.
    pub fn allows_file_isolated(&self, file: &Value, path: &str) -> Result<bool, String> {
        self.verdict(file, &|| path.to_owned())
            .map_err(|error| error.message().to_owned())
    }

    fn allows_context(&self, file: &Value, path: impl Fn() -> String) -> bool {
        self.registry_verdict(file, &path).unwrap_or(false)
    }

    /// A false condition omits the entry; a decode or handler error also
    /// invalidates the enclosing registry file (for example a loot table).
    fn registry_verdict(&self, file: &Value, path: &dyn Fn() -> String) -> Option<bool> {
        match self.verdict(file, path) {
            Ok(answer) => Some(answer),
            Err(error) => {
                let diagnostic = format!("{}: {}", path(), error.message());
                // Some early readers do not have all of the built-in contexts.
                // Preserve their existing omission rather than reading stale tags.
                let invalid = matches!(error, ConditionError::Invalid(_));
                if let Some(state) = self.diagnostics.0.as_ref().filter(|_| invalid) {
                    state.lock().unwrap().errors.insert(diagnostic);
                } else {
                    log::error!("{diagnostic}; file left out");
                }
                if invalid { None } else { Some(false) }
            }
        }
    }

    fn verdict(&self, file: &Value, path: &dyn Fn() -> String) -> Verdict {
        let Some(payload) = file.get("neoforge:conditions") else {
            return Ok(true);
        };
        let loaded = |id: &str| self.mod_loaded(id);
        let context = &self.context;
        let Some(registered) = &self.registered else {
            return evaluate(payload, &loaded, None, context);
        };
        let path = path();
        let path = if path.starts_with("data/") {
            path.as_str()
        } else {
            path.rfind("/data/")
                .map_or(path.as_str(), |i| &path[i + 1..])
        };
        let state = self
            .diagnostics
            .0
            .as_ref()
            .expect("owning candidate diagnostics");
        // Context-free answers are shared across candidate views. Cache the
        // result, not its loader's recovery policy.
        let encoded = payload.to_string();
        let reads_context = [
            "neoforge:registered",
            "neoforge:tag_empty",
            "neoforge:feature_flags_enabled",
        ]
        .iter()
        .any(|name| encoded.contains(name));
        let cell = state
            .lock()
            .unwrap()
            .cache
            .entry((
                path.to_owned(),
                encoded,
                if reads_context { context.key() } else { 0 },
            ))
            .or_default()
            .clone();
        cell.get_or_init(|| evaluate(payload, &loaded, Some(&registered.0.handlers), context))
            .clone()
    }

    /// Refuse a failed candidate before publishing any of its tables.
    pub fn check_errors(&self) -> Result<(), String> {
        if let Some(state) = self.diagnostics.0.as_ref() {
            let state = state.lock().unwrap();
            if !state.errors.is_empty() {
                return Err(state.errors.iter().cloned().collect::<Vec<_>>().join("; "));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum ConditionError {
    Invalid(String),
    MissingContext(&'static str),
}
impl ConditionError {
    fn message(&self) -> &str {
        match self {
            Self::Invalid(message) => message,
            Self::MissingContext(message) => message,
        }
    }
}

type Verdict = Result<bool, ConditionError>;
type CachedVerdict = Arc<std::sync::OnceLock<Verdict>>;

#[derive(Debug, Default)]
struct CandidateDiagnostics {
    cache: BTreeMap<(String, String, u64), CachedVerdict>,
    errors: BTreeSet<String>,
}
// Diagnostics do not contribute to a resource generation's cache identity.
#[derive(Clone, Debug)]
struct Diagnostics(Option<std::sync::Arc<std::sync::Mutex<CandidateDiagnostics>>>);
impl Diagnostics {
    const fn new() -> Self {
        Self(None)
    }
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self(Some(Default::default()))
    }
}
impl PartialEq for Diagnostics {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Diagnostics {}
impl std::hash::Hash for Diagnostics {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

/// `ICondition.IContext`: what `feature_flags_enabled`, `registered` and
/// `tag_empty` read. A part the owning load didn't provide leaves its
/// condition unsupported: the file is omitted with one error, never answered stale.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Context {
    features: Option<Arc<BTreeSet<String>>>,
    feature_fingerprint: Option<u64>,
    registries: Option<Arc<Names>>,
    tags: Option<Arc<Names>>,
}
impl Context {
    /// The cache key of answers given under this context.
    fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        (&self.feature_fingerprint, &self.registries, &self.tags).hash(&mut hasher);
        hasher.finish()
    }
}

/// Registry id -> entry ids, compared and hashed by a fingerprint taken once,
/// so cache identity never rehashes every entry.
#[derive(Debug)]
struct Names {
    by_registry: BTreeMap<String, BTreeSet<String>>,
    fingerprint: u64,
}
impl Names {
    fn new<'a, I>(registries: impl IntoIterator<Item = (&'a str, I)>) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        use std::hash::{Hash, Hasher};
        let mut by_registry: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (registry, ids) in registries {
            by_registry
                .entry(qualified(registry))
                .or_default()
                .extend(ids.into_iter().map(qualified));
        }
        let mut hasher = std::hash::DefaultHasher::new();
        by_registry.hash(&mut hasher);
        Self {
            by_registry,
            fingerprint: hasher.finish(),
        }
    }
    fn contains(&self, registry: &str, id: &str) -> bool {
        self.by_registry
            .get(registry)
            .is_some_and(|ids| ids.contains(id))
    }
}
impl PartialEq for Names {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}
impl Eq for Names {}
impl std::hash::Hash for Names {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

/// An identifier with vanilla's default namespace (`Identifier.parse`).
fn qualified(id: &str) -> String {
    let id = id.strip_prefix('#').unwrap_or(id);
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

// Pass the raw node without copying; the registration adapter removes `type`
// before passing the remaining fields to the mod.
type ConditionHandler = dyn Fn(&Value) -> Result<bool, String> + Send + Sync;
type Handlers = BTreeMap<String, Box<ConditionHandler>>;
struct LookupState {
    inventories: std::sync::Mutex<BTreeMap<Vec<String>, Diagnostics>>,
    handlers: Handlers,
}
#[derive(Clone)]
struct RegisteredLookup(std::sync::Arc<LookupState>);
impl std::fmt::Debug for RegisteredLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredLookup")
    }
}
impl PartialEq for RegisteredLookup {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RegisteredLookup {}
impl std::hash::Hash for RegisteredLookup {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&std::sync::Arc::as_ptr(&self.0), state);
    }
}

#[cfg(test)]
fn conditions_hold(v: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
    evaluate(&v["neoforge:conditions"], loaded, None, &Context::default()).unwrap_or(false)
}

/// Does the built-in evaluator already implement this condition ID?
pub fn is_builtin_condition(id: &sunrise_core::identifier::Identifier) -> bool {
    BuiltinCondition::from_parts(id.namespace(), id.path()).is_some()
}

#[derive(Clone, Copy)]
enum BuiltinCondition {
    ModLoaded,
    Always,
    Never,
    FeatureFlags,
    Registered,
    TagEmpty,
    Not,
    And,
    Or,
}

impl BuiltinCondition {
    fn from_parts(namespace: &str, path: &str) -> Option<Self> {
        if namespace != "neoforge" {
            return None;
        }
        Some(match path {
            "mod_loaded" => Self::ModLoaded,
            "always" => Self::Always,
            "never" => Self::Never,
            "feature_flags_enabled" => Self::FeatureFlags,
            "registered" => Self::Registered,
            "tag_empty" => Self::TagEmpty,
            "not" => Self::Not,
            "and" => Self::And,
            "or" => Self::Or,
            _ => return None,
        })
    }
}

enum Condition<'a> {
    ModLoaded(&'a str),
    Literal(bool),
    FeatureFlags(Vec<String>),
    Registered { registry: String, value: String },
    TagEmpty { registry: String, tag: String },
    Not(Box<Self>),
    Combinator { and: bool, children: Vec<Self> },
    Custom(&'a ConditionHandler, &'a Value),
}

#[derive(Default)]
struct DecodeErrors<'a> {
    unknown: BTreeSet<&'a str>,
    invalid: BTreeSet<String>,
}

impl<'a> Condition<'a> {
    fn decode(
        node: &'a Value,
        handlers: Option<&'a Handlers>,
        errors: &mut DecodeErrors<'a>,
    ) -> Option<Self> {
        let Some(name) = node["type"].as_str() else {
            errors
                .invalid
                .insert("load condition: missing or invalid type".into());
            return None;
        };
        if let Some(kind) = name
            .split_once(':')
            .and_then(|(ns, path)| BuiltinCondition::from_parts(ns, path))
        {
            return match Self::decode_builtin(kind, node, handlers, errors) {
                Ok(condition) => condition,
                Err(field) => {
                    errors.invalid.insert(format!("{name}: invalid {field}"));
                    None
                }
            };
        }
        let handler = handlers.and_then(|handlers| match name.split_once(':') {
            Some((ns, _)) if !ns.is_empty() => handlers.get(name),
            _ => handlers.get(&format!(
                "minecraft:{}",
                name.strip_prefix(':').unwrap_or(name)
            )),
        });
        if let Some(handler) = handler {
            Some(Self::Custom(handler.as_ref(), node))
        } else {
            errors.unknown.insert(name);
            None
        }
    }

    fn decode_builtin(
        kind: BuiltinCondition,
        node: &'a Value,
        handlers: Option<&'a Handlers>,
        errors: &mut DecodeErrors<'a>,
    ) -> Result<Option<Self>, &'static str> {
        let identifier = |value: &Value, field| {
            value
                .as_str()
                .and_then(|id| sunrise_core::identifier::Identifier::parse(id).ok())
                .map(|id| id.to_string())
                .ok_or(field)
        };
        let registry = || {
            node.get("registry").map_or_else(
                || Ok("minecraft:item".into()),
                |value| identifier(value, "registry"),
            )
        };
        use BuiltinCondition::*;
        Ok(Some(match kind {
            Always => Self::Literal(true),
            Never => Self::Literal(false),
            ModLoaded => Self::ModLoaded(node["modid"].as_str().ok_or("modid")?),
            FeatureFlags => Self::FeatureFlags(
                node["flags"]
                    .as_array()
                    .filter(|flags| !flags.is_empty())
                    .ok_or("flags")?
                    .iter()
                    .map(|value| identifier(value, "flags"))
                    .collect::<Result<_, _>>()?,
            ),
            Registered => Self::Registered {
                registry: registry()?,
                value: identifier(&node["value"], "value")?,
            },
            TagEmpty => Self::TagEmpty {
                registry: registry()?,
                tag: identifier(&node["tag"], "tag")?,
            },
            Not => {
                let child = node.get("value").ok_or("value")?;
                return Ok(
                    Self::decode(child, handlers, errors).map(|child| Self::Not(Box::new(child)))
                );
            }
            kind @ (And | Or) => Self::Combinator {
                and: matches!(kind, And),
                children: node["values"]
                    .as_array()
                    .ok_or("values")?
                    .iter()
                    .filter_map(|child| Self::decode(child, handlers, errors))
                    .collect(),
            },
        }))
    }

    // Only pure tests with available inputs can be skipped after the result
    // is decided. Decode has already checked even the branches we skip.
    fn must_test(&self, context: &Context) -> bool {
        match self {
            Self::ModLoaded(_) | Self::Literal(_) => false,
            Self::FeatureFlags(_) => context.features.is_none(),
            Self::Registered { .. } => context.registries.is_none(),
            Self::TagEmpty { .. } => context.tags.is_none(),
            Self::Not(child) => child.must_test(context),
            Self::Combinator { children, .. } => {
                children.iter().any(|child| child.must_test(context))
            }
            Self::Custom(..) => true,
        }
    }

    fn test(
        &self,
        loaded: &dyn Fn(&str) -> bool,
        context: &Context,
        missing_context: &mut Option<&'static str>,
    ) -> Verdict {
        let mut unavailable = |reason| {
            missing_context.get_or_insert(reason);
            Ok(false)
        };
        Ok(match self {
            Self::ModLoaded(id) => loaded(id),
            Self::Literal(value) => *value,
            Self::FeatureFlags(flags) => {
                let Some(enabled) = &context.features else {
                    return unavailable(
                        "neoforge:feature_flags_enabled: no feature flags in this load",
                    );
                };
                flags.iter().all(|flag| enabled.contains(flag))
            }
            Self::Registered { registry, value } => {
                let Some(registries) = &context.registries else {
                    return unavailable("neoforge:registered: no registries in this load");
                };
                registries.contains(registry, value)
            }
            Self::TagEmpty { registry, tag } => {
                let Some(tags) = &context.tags else {
                    return unavailable("neoforge:tag_empty: tags not loaded yet");
                };
                !tags.contains(registry, tag)
            }
            Self::Not(child) => !child.test(loaded, context, missing_context)?,
            Self::Combinator { and, children } => {
                Self::combine(*and, children, loaded, context, missing_context)?
            }
            Self::Custom(handler, args) => handler(args).map_err(ConditionError::Invalid)?,
        })
    }

    fn combine(
        and: bool,
        children: &[Self],
        loaded: &dyn Fn(&str) -> bool,
        context: &Context,
        missing_context: &mut Option<&'static str>,
    ) -> Verdict {
        let mut answer = and;
        for child in children {
            if answer == and || child.must_test(context) {
                let value = child.test(loaded, context, missing_context)?;
                answer = if and {
                    answer && value
                } else {
                    answer || value
                };
            }
        }
        Ok(answer)
    }
}

fn evaluate(
    payload: &Value,
    loaded: &dyn Fn(&str) -> bool,
    handlers: Option<&Handlers>,
    context: &Context,
) -> Verdict {
    // Decode the entire tree before invoking any test, including branches that
    // cannot affect the boolean answer.
    let mut errors = DecodeErrors::default();
    let conditions: Vec<_> = payload
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| Condition::decode(node, handlers, &mut errors))
        .collect();
    if !errors.unknown.is_empty() {
        errors.invalid.insert(format!(
            "unknown load conditions [{}]",
            errors.unknown.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !errors.invalid.is_empty() {
        return Err(ConditionError::Invalid(
            errors.invalid.into_iter().collect::<Vec<_>>().join("; "),
        ));
    }
    let mut missing_context = None;
    let allowed = Condition::combine(true, &conditions, loaded, context, &mut missing_context)?;
    missing_context.map_or(Ok(allowed), |reason| {
        Err(ConditionError::MissingContext(reason))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_conditions_capture_package_ids_and_keep_their_generation() {
        let mut enabled = vec!["code_only".to_owned()];
        let old = LoadConditions::new(enabled.iter().map(String::as_str));
        enabled.clear();
        let next = LoadConditions::new(enabled.iter().map(String::as_str));
        let requires_code = json!({"neoforge:conditions":[{
            "type":"neoforge:mod_loaded", "modid":"code_only"
        }]});
        assert!(old.allows(&requires_code));
        assert!(!next.allows(&requires_code));
        assert!(old.mod_loaded("minecraft") && next.mod_loaded("minecraft"));
        assert!(!old.mod_loaded("resource_namespace"));
        assert!(old.allows(&json!({"values":[]})));
        assert!(!old.allows(&json!({"neoforge:conditions":[{"type":"future:condition"}]})));
    }

    #[test]
    fn load_condition_cache_identity_uses_membership_not_input_order() {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let key = |conditions: &LoadConditions| {
            let mut hasher = DefaultHasher::new();
            conditions.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(
            key(&LoadConditions::new(["b", "a", "a"])),
            key(&LoadConditions::new(["a", "b"]))
        );
        assert_ne!(
            key(&LoadConditions::new(["a"])),
            key(&LoadConditions::vanilla())
        );
        assert_eq!(
            LoadConditions::new(["minecraft"]),
            LoadConditions::vanilla()
        );
    }

    fn loaded(id: &str) -> bool {
        matches!(id, "minecraft" | "create")
    }

    #[test]
    fn a_file_without_conditions_loads() {
        assert!(conditions_hold(
            &json!({"type": "minecraft:crafting"}),
            &loaded
        ));
    }

    #[test]
    fn mod_loaded_follows_the_answer_and_minecraft_always_holds() {
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "create"}]}),
            &loaded
        ));
        assert!(!conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "ic2"}]}),
            &loaded
        ));
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "minecraft"}]}),
            &loaded
        ));
    }

    #[test]
    fn not_and_or_combine_and_all_must_hold() {
        let not = json!({"type": "neoforge:not", "value": {"type": "neoforge:mod_loaded", "modid": "ic2"}});
        let mut c = serde_json::Map::new();
        c.insert("neoforge:conditions".to_owned(), json!([not]));
        assert!(conditions_hold(&Value::Object(c.clone()), &loaded));
        let both = json!([
            {"type": "neoforge:mod_loaded", "modid": "create"},
            {"type": "neoforge:or", "values": [
                {"type": "neoforge:mod_loaded", "modid": "ic2"},
                {"type": "neoforge:always"}
            ]}
        ]);
        c.insert("neoforge:conditions".to_owned(), both);
        assert!(conditions_hold(&Value::Object(c), &loaded));
        // One failing condition in the list fails the file.
        let failing = json!([
            {"type": "neoforge:always"},
            {"type": "neoforge:mod_loaded", "modid": "ic2"}
        ]);
        let mut f = serde_json::Map::new();
        f.insert("neoforge:conditions".to_owned(), failing);
        assert!(!conditions_hold(&Value::Object(f), &loaded));
    }

    #[test]
    fn an_unknown_kind_nested_in_the_tree_leaves_the_file_out() {
        // `not` of `not` of an unknown: the unknown is two levels down.
        let nested_not = json!({
            "neoforge:conditions": [
                {"type": "neoforge:not", "value":
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}}
            ]
        });
        assert!(!conditions_hold(&nested_not, &loaded));
        // `or` with `always` beside a `not` of an unknown: one child is
        // known-true, but the other's subtree holds an unknown.
        let nested_or = json!({
            "neoforge:conditions": [
                {"type": "neoforge:or", "values": [
                    {"type": "neoforge:always"},
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}
                ]}
            ]
        });
        assert!(!conditions_hold(&nested_or, &loaded));
    }

    fn file(condition: Value) -> Value {
        json!({"neoforge:conditions": [condition]})
    }

    #[test]
    fn always_and_never_evaluate_without_a_registered_handler() {
        let conditions = LoadConditions::new([]);
        assert!(conditions.allows(&file(json!({"type": "neoforge:always"}))));
        assert!(!conditions.allows(&file(json!({"type": "neoforge:never"}))));
        assert!(conditions.allows(&file(
            json!({"type": "neoforge:not", "value": {"type": "neoforge:never"}})
        )));
        assert!(
            conditions.allows(&file(json!({"type": "neoforge:or", "values": [
                {"type": "neoforge:never"}, {"type": "neoforge:always"}
            ]})))
        );
    }

    #[test]
    fn registered_reads_the_loads_registries_and_defaults_to_items() {
        let stone = file(json!({"type": "neoforge:registered", "value": "minecraft:stone"}));
        let missing = file(
            json!({"type": "neoforge:registered", "registry": "minecraft:item", "value": "test:missing"}),
        );
        let biome = file(
            json!({"type": "neoforge:registered", "registry": "worldgen/biome", "value": "plains"}),
        );
        // No registries in this load: unsupported, so the file is left out.
        assert!(!LoadConditions::vanilla().allows(&stone));
        let conditions = LoadConditions::new([]).with_registries([
            ("minecraft:item", vec!["minecraft:stone"]),
            ("minecraft:worldgen/biome", vec!["minecraft:plains"]),
        ]);
        assert!(conditions.allows(&stone));
        assert!(!conditions.allows(&missing));
        assert!(conditions.allows(&biome));
    }

    #[test]
    fn feature_flags_enabled_needs_every_flag() {
        let both = file(json!({"type": "neoforge:feature_flags_enabled",
            "flags": ["minecraft:vanilla", "minecraft:trade_rebalance"]}));
        let vanilla = file(json!({"type": "neoforge:feature_flags_enabled", "flags": ["vanilla"]}));
        assert!(!LoadConditions::vanilla().allows(&vanilla));
        let default = LoadConditions::new([]).with_features([]);
        assert!(default.allows(&vanilla) && !default.allows(&both));
        let experiment = LoadConditions::new([])
            .with_features(["minecraft:vanilla", "minecraft:trade_rebalance"]);
        assert!(experiment.allows(&both));
    }

    #[test]
    fn tag_empty_reads_only_the_tags_this_load_bound() {
        let tin = file(json!({"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}));
        let before_tags = LoadConditions::new([]);
        assert!(!before_tags.allows(&tin), "no tag view yet: left out");
        let empty = before_tags.with_tags([("minecraft:item", vec!["minecraft:planks"])]);
        assert!(empty.allows(&tin));
        let with_tin = before_tags.with_tags([("minecraft:item", vec!["c:ingots/tin"])]);
        assert!(!with_tin.allows(&tin));
        assert_ne!(empty, with_tin, "the tag view is part of cache identity");
    }

    #[test]
    fn an_unsupplied_registry_is_empty_without_a_diagnostic() {
        let conditions = LoadConditions::new([])
            .with_registries([("item", ["stone"])])
            .with_tags([("item", ["planks"])]);
        for registry in ["minecraft:worldgen/biome", "test:missing"] {
            let registered = json!({"type": "neoforge:registered",
                "registry": registry, "value": "test:entry"});
            let empty = json!({"type": "neoforge:tag_empty",
                "registry": registry, "tag": "test:tag"});
            assert!(!conditions.allows_file(&file(registered.clone()), "data/test/recipe/a.json"));
            assert!(conditions.allows_file(&file(empty.clone()), "data/test/recipe/b.json"));
            assert!(conditions.allows(&file(json!({"type": "neoforge:not", "value": registered}))));
            assert!(!conditions.allows(&file(json!({"type": "neoforge:not", "value": empty}))));
        }
        assert_eq!(conditions.check_errors(), Ok(()));
    }

    #[test]
    fn a_retained_candidate_does_not_reuse_an_answer_across_tag_views() {
        let tin = file(json!({"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}));
        let candidate = LoadConditions::new([]).with_lookup([("test:flag", |_: &Value| Ok(true))]);
        let empty = candidate.with_tags([("minecraft:item", Vec::<&str>::new())]);
        let with_tin = candidate.with_tags([("minecraft:item", vec!["c:ingots/tin"])]);
        assert!(empty.allows_file(&tin, "data/a/recipe/b.json"));
        assert!(!with_tin.allows_file(&tin, "data/a/recipe/b.json"));
    }

    #[test]
    fn malformed_builtin_operands_are_errors_before_testing() {
        for malformed in [
            json!({"type": "neoforge:feature_flags_enabled"}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": null}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": "vanilla"}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": [false]}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": ["test:missing", 1]}),
            json!({"type": "neoforge:tag_empty"}),
            json!({"type": "neoforge:tag_empty", "tag": null}),
            json!({"type": "neoforge:tag_empty", "tag": []}),
            json!({"type": "neoforge:tag_empty", "tag": 1}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": []}),
            json!({"type": "neoforge:mod_loaded", "modid": false}),
            json!({"type": "neoforge:registered", "value": true}),
            json!({"type": "neoforge:registered", "registry": null, "value": "stone"}),
            json!({"type": "neoforge:tag_empty", "tag": "bad tag"}),
            json!({"type": "neoforge:not"}),
            json!({"type": "neoforge:and", "values": {}}),
            json!({"type": "neoforge:or", "values": [null]}),
        ] {
            let not = json!({"type": "neoforge:not", "value": malformed.clone()});
            let twice = json!({"type": "neoforge:not", "value": not.clone()});
            for condition in [malformed, not, twice] {
                let payload = json!([
                    {"type":"neoforge:mod_loaded","modid":"should_not_test"},
                    {"type":"neoforge:or","values":[{"type":"neoforge:always"},condition]}
                ]);
                assert!(
                    matches!(
                        evaluate(
                            &payload,
                            &|_| panic!("tested before decode"),
                            None,
                            &Context::default()
                        ),
                        Err(ConditionError::Invalid(_))
                    ),
                    "{payload}"
                );
            }
        }
    }

    #[test]
    fn decided_combinators_skip_pure_tests_but_not_missing_context() {
        for (op, deciding, answer) in [("and", "never", false), ("or", "always", true)] {
            let decisive = json!({ "type": format!("neoforge:{deciding}") });
            let pure = json!({"type":"neoforge:mod_loaded","modid":"create"});
            let payload = json!([{"type":format!("neoforge:{op}"),"values":[decisive,pure]}]);
            let loaded = |_: &str| panic!("tested an already decided pure branch");
            let context = Context::default();
            assert_eq!(evaluate(&payload, &loaded, None, &context).unwrap(), answer);
            let missing = json!([{"type":format!("neoforge:{op}"),"values":[decisive,
                {"type":"neoforge:not","value":{"type":"neoforge:tag_empty","tag":"test:x"}}]}]);
            assert!(matches!(
                evaluate(&missing, &loaded, None, &context),
                Err(ConditionError::MissingContext(
                    "neoforge:tag_empty: tags not loaded yet"
                ))
            ));
        }
    }

    #[test]
    fn unconditioned_elements_never_build_a_diagnostic_path() {
        let mut entries = vec![json!(1), json!({"name": "test:entry"})];
        let expected = entries.clone();
        LoadConditions::vanilla().filter_list(&mut entries, &|| {
            panic!("an unconditioned element has no diagnostic path")
        });
        assert_eq!(entries, expected);
    }

    #[test]
    fn unconditioned_loot_pools_are_kept_as_written() {
        let mut value = json!({"pools": [{"rolls": 1, "entries": [1, {"name": "test:entry"}]}]});
        let expected = value.clone();
        assert!(LoadConditions::vanilla().admit_loot_table(&mut value, "test:table"));
        assert_eq!(value, expected);
    }

    #[test]
    fn loot_pool_conditions_drop_failing_pools_and_strip_passing_keys() {
        let mut table = json!({"pools": [
            {"rolls": 1},
            {"rolls": 2, "neoforge:conditions": [
                {"type": "neoforge:mod_loaded", "modid": "create"}]},
            {"rolls": 3, "neoforge:conditions": [
                {"type": "neoforge:mod_loaded", "modid": "ic2"}]}
        ]});
        assert!(LoadConditions::new(["create"]).admit_loot_table(&mut table, "a:b"));
        assert_eq!(table["pools"], json!([{"rolls": 1}, {"rolls": 2}]));
    }

    #[test]
    fn an_unknown_condition_does_not_load_the_file() {
        let mut c = serde_json::Map::new();
        c.insert(
            "neoforge:conditions".to_owned(),
            json!([{"type": "neoforge:future_thing"}]),
        );
        assert!(!conditions_hold(&Value::Object(c), &loaded));
    }
}

#[cfg(test)]
#[path = "condition_registry_tests.rs"]
mod registry_tests;

#[cfg(test)]
#[path = "condition_log_tests.rs"]
mod log_tests;
