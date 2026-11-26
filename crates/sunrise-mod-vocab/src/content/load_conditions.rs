//! Load-phase condition handlers.
use crate::Identifier;
use crate::generated::HookId;
use crate::runtime::discard_registration;
use std::{collections::BTreeMap, sync::Arc};
type Entries = BTreeMap<Identifier, (&'static str, Arc<dyn LoadCondition>)>;

/// A condition's invalid arguments or a registration refusal.
// exhaustive: mods construct this error with a diagnostic message.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub struct ConditionError {
    /// Explanation included in the owning data file's reload diagnostic.
    pub message: String,
}

impl std::fmt::Display for ConditionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ConditionError {}

impl ConditionError {
    /// A refusal with `message` as its diagnostic.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<(Identifier, String)> for ConditionError {
    fn from((id, message): (Identifier, String)) -> Self {
        Self {
            message: format!("{id}: {message}"),
        }
    }
}

/// A handler that reads the condition's JSON from a borrowed raw value.
/// Arguments are the JSON object without `type`; validate their shape and
/// return `ConditionError` for invalid mod input, never panic while decoding it.
/// A loader first selects the winning file and decodes its entire condition list,
/// including children of `not`, `and` and `or`. Only built-in registry and tag reads
/// order the dependencies between registry files; custom arguments are just JSON.
/// An unknown ID, malformed condition list, returned error, or native panic refuses
/// registry-backed data before publication. A simple JSON listener or data map logs
/// the error and skips the offending file instead. A panic is logged and queues an
/// error toast naming the mod, and disables only that handler for the session, across
/// loads. Evaluations that reach it report an error without calling it again. A returned
/// `ConditionError` or an ordinary phase refusal is reported without disabling the
/// handler. Handlers must not mutate published data.
pub trait LoadCondition: Send + Sync + 'static {
    /// Read and validate the raw JSON arguments when testing reaches this condition.
    /// Return `Ok(false)` when the condition does not hold: the winner is omitted, not
    /// replaced by a lower pack. Return `Err` for invalid arguments. Testing stops as
    /// soon as the enclosing list, `and`, or `or` is decided.
    fn test(&self, args: &serde_json::Value) -> Result<bool, ConditionError>;
}

/// Registrar borrowed during `register_load_conditions`.
pub trait LoadConditions {
    /// Register a handler under any namespace, independently of the dispatching mod.
    /// Built-in IDs and duplicate names are refused; duplicate diagnostics name both mods.
    /// A completed listener's registration errors refuse preparation even if ignored.
    fn register_handler(
        &mut self,
        name: Identifier,
        condition: Arc<dyn LoadCondition>,
    ) -> Result<(), ConditionError>;
}
impl dyn LoadConditions + '_ {
    /// Store a concrete load condition handler.
    pub fn register(
        &mut self,
        name: Identifier,
        condition: impl LoadCondition,
    ) -> Result<(), ConditionError> {
        self.register_handler(name, Arc::new(condition))
    }
}

// no wire: process-local callback storage; handlers cross through generated dispatch.
/// Engine-owned declarations retaining the dispatch owner of each handler.
// exhaustive: private fields prevent mod construction or destructuring.
pub struct LoadConditionDeclarations {
    is_reserved: fn(&Identifier) -> bool,
    entries: Entries,
    errors: Vec<ConditionError>,
}
impl LoadConditionDeclarations {
    /// Start collecting declarations with the game evaluator's built-in ID check.
    // hidden: the engine creates this collector; mods receive only the registrar.
    #[doc(hidden)]
    pub fn new(is_reserved: fn(&Identifier) -> bool) -> Self {
        Self {
            is_reserved,
            entries: BTreeMap::new(),
            errors: Vec::new(),
        }
    }

    /// Publish the completed listener's handlers and its preparation errors together.
    // hidden: the engine stages one listener at a time; mods receive only the registrar trait.
    #[doc(hidden)]
    pub fn register_listener(&mut self, listen: &mut dyn FnMut(&mut dyn LoadConditions) -> bool) {
        let mut pending = Self::new(self.is_reserved);
        if listen(&mut ConditionEdits {
            published: Some(&self.entries),
            pending: &mut pending,
        }) {
            self.entries.extend(pending.entries);
            self.errors.append(&mut pending.errors);
        } else {
            let owner = crate::content::crash::current_native_mod();
            for (_, handler) in pending.entries.into_values() {
                discard_registration(REGISTRATION, owner, handler);
            }
        }
    }

    /// Finish registration, refusing even errors a completed listener ignored.
    // hidden: engine preparation consumes the declaration table.
    #[doc(hidden)]
    pub fn finish(self) -> Result<Entries, ConditionError> {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(error);
        }
        Ok(self.entries)
    }
}
const REGISTRATION: (HookId, &str) = (HookId::RegisterLoadConditions, "register_load_conditions");

impl LoadConditions for LoadConditionDeclarations {
    fn register_handler(
        &mut self,
        name: Identifier,
        condition: Arc<dyn LoadCondition>,
    ) -> Result<(), ConditionError> {
        ConditionEdits {
            published: None,
            pending: self,
        }
        .register_handler(name, condition)
    }
}

struct ConditionEdits<'a> {
    published: Option<&'a Entries>,
    pending: &'a mut LoadConditionDeclarations,
}

impl LoadConditions for ConditionEdits<'_> {
    fn register_handler(
        &mut self,
        name: Identifier,
        condition: Arc<dyn LoadCondition>,
    ) -> Result<(), ConditionError> {
        let Some(owner) = crate::content::crash::current_native_mod() else {
            return Err(ConditionError::new(format!(
                "cannot register condition {name} outside register_load_conditions dispatch"
            )));
        };
        if (self.pending.is_reserved)(&name) {
            let error = ConditionError::new(format!(
                "mod {owner} cannot register condition {name}: that ID is built in"
            ));
            self.pending.errors.push(error.clone());
            return Err(error);
        }
        if let Some((previous, _)) = self
            .pending
            .entries
            .get(&name)
            .or_else(|| self.published.and_then(|entries| entries.get(&name)))
        {
            let error = ConditionError {
                message: format!("condition {name} registered by both {previous} and {owner}"),
            };
            self.pending.errors.push(error.clone());
            return Err(error);
        }
        self.pending.entries.insert(name, (owner, condition));
        Ok(())
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    struct Always;
    impl LoadCondition for Always {
        fn test(&self, _: &serde_json::Value) -> Result<bool, ConditionError> {
            Ok(true)
        }
    }

    #[test]
    fn detached_registration_returns_a_named_error() {
        let mut declarations = LoadConditionDeclarations::new(|_| false);
        let error = declarations
            .register_handler(Identifier::parse("demo:flag").unwrap(), Arc::new(Always))
            .unwrap_err();
        assert_eq!(
            error.message,
            "cannot register condition demo:flag outside register_load_conditions dispatch"
        );
    }

    #[test]
    fn condition_error_supports_standard_error_propagation() {
        fn propagate() -> Result<(), Box<dyn std::error::Error>> {
            let value: Result<(), super::ConditionError> =
                Err(super::ConditionError::new("bad value"));
            value?;
            Ok(())
        }
        assert_eq!(propagate().unwrap_err().to_string(), "bad value");
    }
}
