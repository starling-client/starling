//! Functions registered for the in-world game test runner.
use crate::ServerHost;
use crate::generated::HookId;
use crate::runtime::discard_registration;
use std::{collections::BTreeMap, sync::Arc};

/// The outcome of a function invocation. Pending keeps the runner's timeout active.
#[derive(Clone, Debug, Default, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum GameTestResult {
    /// Complete this attempt successfully.
    Pass,
    /// Complete this attempt with an assertion message.
    Fail(String),
    /// Leave this attempt running until its timeout.
    #[default]
    Pending,
}

/// API-owned coordinates for one attempt, valid only during its callback.
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct GameTestContext {
    /// The server level containing this attempt.
    pub level: crate::LevelRef,
    /// Structure origin in the current server level.
    pub origin: [i32; 3],
    /// Structure rotation in clockwise quarter turns.
    pub rotation: u8,
}
impl GameTestContext {
    /// Construct the context at the engine boundary.
    pub fn new(level: crate::LevelRef, origin: [i32; 3], rotation: u8) -> Self {
        Self {
            level,
            origin,
            rotation,
        }
    }
    /// Convert structure-relative coordinates to world coordinates.
    pub fn absolute_pos(&self, [x, y, z]: [i32; 3]) -> [i32; 3] {
        let (x, z) = match self.rotation % 4 {
            1 => (-z, x),
            2 => (-x, -z),
            3 => (z, -x),
            _ => (x, z),
        };
        [self.origin[0] + x, self.origin[1] + y, self.origin[2] + z]
    }
}
/// A function run once at the start of each attempt, with the owning server level.
pub trait GameTestFunction: Send + Sync {
    /// Return an outcome; the runner owns retries, timeouts and completion.
    fn run(&self, host: &mut dyn ServerHost, context: &GameTestContext) -> GameTestResult;
}
/// Metadata for a function test added to the world's test-instance registry.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct GameTestDefinition {
    /// Structure template to place.
    pub structure: String,
    /// Registered function identifier.
    pub function: String,
    /// Environment definition selected by the runner.
    pub environment: String,
    /// Server dimension in which to run.
    pub dimension: String,
    /// Timeout in ticks.
    pub max_ticks: i32,
    /// Delay before invoking the function.
    pub setup_ticks: i32,
    /// Maximum attempts.
    pub max_attempts: i32,
    /// Number of successful attempts required.
    pub required_successes: i32,
    /// Whether failure counts as a required failure.
    pub required: bool,
}
impl GameTestDefinition {
    /// A required test with a 100-tick timeout, no setup delay and one attempt.
    pub fn new(structure: String, function: String) -> Self {
        Self {
            structure,
            function,
            environment: "minecraft:default".into(),
            dimension: "minecraft:overworld".into(),
            max_ticks: 100,
            setup_ticks: 0,
            max_attempts: 1,
            required_successes: 1,
            required: true,
        }
    }
}
/// Registration service supplied to the registration hook.
pub trait GameTests {
    /// Register a test instance; duplicate ids and invalid timing are refused.
    fn register_test(
        &mut self,
        id: &str,
        definition: GameTestDefinition,
    ) -> Result<(), &'static str>;
    /// Register a unique namespaced function. Vanilla identifiers are reserved.
    fn register(
        &mut self,
        id: &str,
        function: Arc<dyn GameTestFunction>,
    ) -> Result<(), &'static str>;
}
/// Load-owned function registry. Duplicate and invalid identifiers are refused.
// no wire: owns process-local test callbacks.
#[derive(Default, Clone)]
pub struct GameTestFunctions {
    definitions: BTreeMap<String, GameTestDefinition>,
    functions: BTreeMap<String, Arc<dyn GameTestFunction>>,
}

const REGISTRATION: (HookId, &str) = (HookId::RegisterGameTests, "register_game_tests");

struct GameTestEdits<'a> {
    published: Option<&'a GameTestFunctions>,
    pending: &'a mut GameTestFunctions,
}

impl GameTests for GameTestEdits<'_> {
    fn register_test(
        &mut self,
        id: &str,
        definition: GameTestDefinition,
    ) -> Result<(), &'static str> {
        if crate::Identifier::parse(id).is_err()
            || !id.contains(':')
            || id.starts_with("minecraft:")
        {
            return Err("invalid or reserved test id");
        }
        if crate::Identifier::parse(&definition.function).is_err()
            || !definition.function.contains(':')
        {
            return Err("expected namespaced function id");
        }
        if self.pending.definitions.contains_key(id)
            || self
                .published
                .is_some_and(|registry| registry.definitions.contains_key(id))
        {
            return Err("duplicate test id");
        }
        if definition.max_ticks < 1
            || definition.setup_ticks < 0
            || definition.max_attempts < 1
            || definition.required_successes < 1
        {
            return Err("invalid test timing");
        }
        self.pending.definitions.insert(id.into(), definition);
        Ok(())
    }

    /// Register a namespaced function. Vanilla identifiers are reserved.
    fn register(
        &mut self,
        id: &str,
        function: Arc<dyn GameTestFunction>,
    ) -> Result<(), &'static str> {
        let Some((namespace, path)) = id.split_once(':') else {
            return Err("expected namespaced function id");
        };
        if namespace == "minecraft"
            || namespace.is_empty()
            || path.is_empty()
            || !namespace
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
            || !path
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"/_.-".contains(&c))
        {
            return Err("invalid or reserved function id");
        }
        if self.pending.functions.contains_key(id)
            || self
                .published
                .is_some_and(|registry| registry.functions.contains_key(id))
        {
            return Err("duplicate function id");
        }
        self.pending.functions.insert(id.into(), function);
        Ok(())
    }
}

impl GameTests for GameTestFunctions {
    fn register_test(
        &mut self,
        id: &str,
        definition: GameTestDefinition,
    ) -> Result<(), &'static str> {
        self.edit().register_test(id, definition)
    }

    fn register(
        &mut self,
        id: &str,
        function: Arc<dyn GameTestFunction>,
    ) -> Result<(), &'static str> {
        self.edit().register(id, function)
    }
}

impl GameTestFunctions {
    fn edit(&mut self) -> GameTestEdits<'_> {
        GameTestEdits {
            published: None,
            pending: self,
        }
    }

    /// Publish the completed listener's test instances and functions together.
    #[doc(hidden)]
    pub fn register_listener(&mut self, listen: &mut dyn FnMut(&mut dyn GameTests) -> bool) {
        let mut pending = Self::default();
        if listen(&mut GameTestEdits {
            published: Some(self),
            pending: &mut pending,
        }) {
            self.definitions.extend(pending.definitions);
            self.functions.extend(pending.functions);
        } else {
            let owner = crate::content::crash::current_native_mod();
            for function in pending.functions.into_values() {
                discard_registration(REGISTRATION, owner, function);
            }
        }
    }

    /// Test definitions collected at world load, in identifier order.
    pub fn definitions(&self) -> impl Iterator<Item = (&str, &GameTestDefinition)> {
        self.definitions
            .iter()
            .map(|(id, definition)| (id.as_str(), definition))
    }

    /// Resolve once when a test definition loads. The callback remains owned by that definition.
    pub fn resolve(&self, id: &str) -> Option<Arc<dyn GameTestFunction>> {
        self.functions.get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Pass;
    impl GameTestFunction for Pass {
        fn run(&self, _: &mut dyn ServerHost, _: &GameTestContext) -> GameTestResult {
            GameTestResult::Pass
        }
    }
    #[test]
    fn a_panicking_game_test_registrar_leaves_both_keys_available_to_the_next_listener() {
        use crate::{Hooks, Host, ModEntry, RegisterGameTestsEvent, testing::FakeHost};
        use std::sync::atomic::{AtomicBool, Ordering};

        struct Registrant {
            callback: Arc<dyn GameTestFunction>,
            structure: &'static str,
            panic_after: Option<Arc<AtomicBool>>,
        }
        #[crate::__subscriptions]
        impl Hooks for Registrant {
            fn register_game_tests(&self, _: &dyn Host, event: &mut RegisterGameTestsEvent<'_>) {
                let definition =
                    GameTestDefinition::new(self.structure.into(), "example:shared".into());
                event
                    .functions
                    .register_test("example:shared", definition.clone())
                    .unwrap();
                event
                    .functions
                    .register("example:shared", self.callback.clone())
                    .unwrap();
                for duplicate in ["example:shared", "example:retained"] {
                    assert_eq!(
                        event.functions.register_test(duplicate, definition.clone()),
                        Err("duplicate test id"),
                    );
                    assert_eq!(
                        event.functions.register(duplicate, self.callback.clone()),
                        Err("duplicate function id"),
                    );
                }
                if let Some(reached) = &self.panic_after {
                    event
                        .functions
                        .register_test("example:leaked", definition)
                        .unwrap();
                    event
                        .functions
                        .register("example:leaked", self.callback.clone())
                        .unwrap();
                    reached.store(true, Ordering::Relaxed);
                    panic!("game test registrar fixture failed");
                }
            }
        }

        let reached = Arc::new(AtomicBool::new(false));
        let healthy: Arc<dyn GameTestFunction> = Arc::new(Pass);
        let retained: Arc<dyn GameTestFunction> = Arc::new(Pass);
        crate::testing::with_dispatch(
            vec![
                ModEntry::new(
                    "a",
                    Box::new(Registrant {
                        callback: Arc::new(Pass),
                        structure: "example:failed",
                        panic_after: Some(reached.clone()),
                    }),
                ),
                ModEntry::new(
                    "b",
                    Box::new(Registrant {
                        callback: healthy.clone(),
                        structure: "example:healthy",
                        panic_after: None,
                    }),
                ),
            ]
            .into(),
            || {
                let mut registry = GameTestFunctions::default();
                registry
                    .register("example:retained", retained.clone())
                    .unwrap();
                registry
                    .register_test(
                        "example:retained",
                        GameTestDefinition::new(
                            "minecraft:empty".into(),
                            "example:retained".into(),
                        ),
                    )
                    .unwrap();
                crate::dispatch_register_game_tests(&FakeHost::new(), |listen| {
                    registry.register_listener(listen)
                });
                assert!(reached.load(Ordering::Relaxed));
                assert_eq!(
                    registry
                        .definitions()
                        .map(|(id, definition)| (
                            id,
                            definition.structure.as_str(),
                            definition.function.as_str()
                        ))
                        .collect::<Vec<_>>(),
                    [
                        ("example:retained", "minecraft:empty", "example:retained"),
                        ("example:shared", "example:healthy", "example:shared"),
                    ],
                );
                assert!(Arc::ptr_eq(
                    &registry.resolve("example:retained").unwrap(),
                    &retained
                ));
                assert!(Arc::ptr_eq(
                    &registry.resolve("example:shared").unwrap(),
                    &healthy
                ));
                assert!(registry.resolve("example:leaked").is_none());
            },
        );
    }

    #[test]
    fn test_definitions_reject_duplicates_and_invalid_timing() {
        let mut registry = GameTestFunctions::default();
        let definition = GameTestDefinition::new("minecraft:empty".into(), "example:pass".into());
        registry
            .register_test("example:passing", definition.clone())
            .unwrap();
        assert!(
            registry
                .register_test("example:passing", definition.clone())
                .is_err()
        );
        let mut invalid = definition;
        invalid.max_ticks = 0;
        assert!(registry.register_test("example:invalid", invalid).is_err());
        let bare = GameTestDefinition::new("example:structure".into(), "pass".into());
        assert_eq!(
            registry.register_test("example:bare", bare),
            Err("expected namespaced function id")
        );
        assert_eq!(
            registry.definitions().map(|(id, _)| id).collect::<Vec<_>>(),
            ["example:passing"]
        );
    }
    #[test]
    fn resolved_functions_survive_registry_replacement_and_duplicates_preserve_original() {
        let mut registry = GameTestFunctions::default();
        let original: Arc<dyn GameTestFunction> = Arc::new(Pass);
        registry.register("example:pass", original.clone()).unwrap();
        assert!(registry.register("example:pass", Arc::new(Pass)).is_err());
        let resolved = registry.resolve("example:pass").unwrap();
        drop(registry);
        assert!(Arc::ptr_eq(&resolved, &original));
        for id in [
            "pass",
            "minecraft:always_pass",
            "Example:test",
            "example:",
            ":test",
            "example:a:b",
        ] {
            assert!(
                GameTestFunctions::default()
                    .register(id, Arc::new(Pass))
                    .is_err(),
                "{id}"
            );
        }
    }
}
