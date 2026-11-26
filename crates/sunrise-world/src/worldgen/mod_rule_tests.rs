//! Rule test (`BuiltInRegistries.RULE_TEST`) and position rule test
//! (`BuiltInRegistries.POS_RULE_TEST`) types a mod registers, handed over once
//! at startup and looked up while processor lists and block/ore features
//! decode, after vanilla's built-in types.
//!
//! Resolved types become [`RuleTest`](super::rule_test::RuleTest) and
//! [`PosRule`](super::structure::processor::PosRule) values directly, with no
//! per-block lookup or allocation. Both run on worldgen worker threads, so
//! values are immutable and shared and carry no `Host`.
use super::DecodeError;
use super::block_feature::FeatureBlockBindings;
use super::mod_random::ApiRandom;
use super::rule_test::RuleTest;
use super::structure::processor::PosRule;
use crate::registry::BlockStateId;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// Decode-time context passed to [`ModRuleTestType::decode`]: carries the
/// block/tag/fluid/property bindings and enforces depth and node budgets across
/// nested rule test decodes.
pub struct RuleTestDecodeScope<'a> {
    bindings: &'a FeatureBlockBindings<'a>,
    depth: usize,
    remaining: Cell<usize>,
}
impl<'a> RuleTestDecodeScope<'a> {
    pub(super) fn new(
        bindings: &'a FeatureBlockBindings<'a>,
        depth: usize,
        remaining: usize,
    ) -> Self {
        Self {
            bindings,
            depth,
            remaining: Cell::new(remaining),
        }
    }
    pub(super) fn remaining(&self) -> usize {
        self.remaining.get()
    }
    /// Load-time block, tag, fluid and property bindings.
    pub fn bindings(&self) -> &'a FeatureBlockBindings<'a> {
        self.bindings
    }
    /// Decode a nested rule test of any type, vanilla's or a mod's.
    pub fn decode_rule_test(&self, json: &str) -> Result<RuleTest, DecodeError> {
        let mut remaining = self.remaining.get();
        let rule = RuleTest::decode(json, self.bindings, self.depth + 1, &mut remaining);
        self.remaining.set(remaining);
        rule
    }
}

/// Decode-time context passed to [`ModPosRuleTestType::decode`]: enforces
/// depth and node budgets across nested position rule test decodes.
pub struct PosRuleTestDecodeScope {
    depth: usize,
    remaining: Cell<usize>,
}
impl PosRuleTestDecodeScope {
    pub(crate) fn new(depth: usize, remaining: usize) -> Self {
        Self {
            depth,
            remaining: Cell::new(remaining),
        }
    }
    pub(crate) fn remaining(&self) -> usize {
        self.remaining.get()
    }
    /// Decode a nested position rule test of any type, vanilla's or a mod's.
    pub fn decode_pos_rule_test(&self, value: &serde_json::Value) -> Result<PosRule, DecodeError> {
        let mut remaining = self.remaining.get();
        let rule = PosRule::decode_inner(Some(value), self.depth + 1, &mut remaining);
        self.remaining.set(remaining);
        rule
    }
}

/// A rule test type's engine side: the codec of vanilla's `RuleTestType`, as
/// the executable's adapter sees it.
pub trait ModRuleTestType: Send + Sync {
    /// Decode one rule test from its JSON object (`predicate_type` included).
    /// An error leaves the processor list (and pools naming it) or feature out
    /// of the load.
    fn decode(
        &self,
        config: &serde_json::Value,
        scope: &RuleTestDecodeScope<'_>,
    ) -> Result<Arc<dyn ModRuleTest>, String>;
}

/// A decoded rule test of a mod's type: vanilla's `RuleTest`. `Send + Sync`
/// because chunks generate concurrently on worker threads; values keep no
/// mutable state.
pub trait ModRuleTest: Send + Sync + std::fmt::Debug + std::any::Any {
    /// `RuleTest.test(state, pos, random)`. Returns `Err` if the mod test
    /// panics or faults (naming mod and type), never substituting a default.
    fn test(
        &self,
        context: &dyn ModRuleTestContext,
        state: BlockStateId,
        position: [i32; 3],
        random: &mut dyn mod_api::RandomSource,
    ) -> Result<bool, String>;
}

/// Nested-rule evaluation available to [`ModRuleTest::test`].
pub trait ModRuleTestContext {
    /// Evaluate a decoded nested rule test, sharing `random`.
    fn test_rule(
        &self,
        rule: &RuleTest,
        state: BlockStateId,
        position: [i32; 3],
        random: &mut dyn mod_api::RandomSource,
    ) -> Result<bool, String>;
}

/// A position rule test type's engine side: the codec of vanilla's
/// `PosRuleTestType`, as the executable's adapter sees it.
pub trait ModPosRuleTestType: Send + Sync {
    /// Decode one position rule test from its JSON object (`predicate_type`
    /// included). An error leaves the processor list (and pools naming it) out
    /// of the load.
    fn decode(
        &self,
        config: &serde_json::Value,
        scope: &PosRuleTestDecodeScope,
    ) -> Result<Arc<dyn ModPosRuleTest>, String>;
}

/// A decoded position rule test of a mod's type: vanilla's `PosRuleTest`.
/// `Send + Sync` because chunks generate concurrently on worker threads;
/// values keep no mutable state.
pub trait ModPosRuleTest: Send + Sync + std::fmt::Debug + std::any::Any {
    /// `PosRuleTest.test(inTemplatePos, worldPos, worldReference, random)`.
    /// Returns `Err` if the mod test panics or faults (naming mod and type),
    /// never substituting a default.
    fn test(
        &self,
        context: &dyn ModPosRuleTestContext,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn mod_api::RandomSource,
    ) -> Result<bool, String>;
}

/// Nested-test evaluation available to [`ModPosRuleTest::test`].
pub trait ModPosRuleTestContext {
    /// Evaluate a decoded nested position rule test, sharing `random`.
    fn test_pos_rule(
        &self,
        rule: &PosRule,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn mod_api::RandomSource,
    ) -> Result<bool, String>;
}

pub(crate) struct RuleTestRunContext;
impl ModRuleTestContext for RuleTestRunContext {
    fn test_rule(
        &self,
        rule: &RuleTest,
        state: BlockStateId,
        position: [i32; 3],
        random: &mut dyn mod_api::RandomSource,
    ) -> Result<bool, String> {
        let mut random = ApiRandom::Borrowed(random);
        rule.test(state, position, &mut random)
    }
}

pub(crate) struct PosRuleTestRunContext;
impl ModPosRuleTestContext for PosRuleTestRunContext {
    fn test_pos_rule(
        &self,
        rule: &PosRule,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn mod_api::RandomSource,
    ) -> Result<bool, String> {
        let mut random = ApiRandom::Borrowed(random);
        rule.test(in_template_pos, world_pos, world_reference, &mut random)
    }
}

/// The installed rule test types, `modid:matter_phase` → type. Set once before
/// any world opens and read while processor lists and block features decode.
pub type ModRuleTestTypes = BTreeMap<String, Arc<dyn ModRuleTestType>>;

/// The installed position rule test types, `modid:y_value` → type. Set once
/// before any world opens and read while processor lists decode.
pub type ModPosRuleTestTypes = BTreeMap<String, Arc<dyn ModPosRuleTestType>>;

static MOD_RULE_TYPES: OnceLock<ModRuleTestTypes> = OnceLock::new();
static MOD_POS_RULE_TYPES: OnceLock<ModPosRuleTestTypes> = OnceLock::new();

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// The rule test types a test installs, in place of the process-wide table.
    static TEST_RULE_TYPES: std::cell::RefCell<Option<ModRuleTestTypes>> =
        const { std::cell::RefCell::new(None) };
    /// The position rule test types a test installs, in place of the process-wide table.
    static TEST_POS_RULE_TYPES: std::cell::RefCell<Option<ModPosRuleTestTypes>> =
        const { std::cell::RefCell::new(None) };
}

/// Install the process-wide rule test table once; a second installation keeps
/// the first and hands its own types back.
pub fn install_mod_rule_test_types(types: ModRuleTestTypes) -> Result<(), ModRuleTestTypes> {
    MOD_RULE_TYPES.set(types)
}

/// Install the process-wide position rule test table once; a second
/// installation keeps the first and hands its own types back.
pub fn install_mod_pos_rule_test_types(
    types: ModPosRuleTestTypes,
) -> Result<(), ModPosRuleTestTypes> {
    MOD_POS_RULE_TYPES.set(types)
}

/// The installed rule test type `id`: none until the executable installs the
/// mods', and a test's own while one is installed.
pub fn mod_rule_test_type(id: &str) -> Option<Arc<dyn ModRuleTestType>> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(found) =
        TEST_RULE_TYPES.with(|cell| cell.borrow().as_ref().map(|types| types.get(id).cloned()))
    {
        return found;
    }
    MOD_RULE_TYPES.get()?.get(id).cloned()
}

/// The installed position rule test type `id`: none until the executable
/// installs the mods', and a test's own while one is installed.
pub fn mod_pos_rule_test_type(id: &str) -> Option<Arc<dyn ModPosRuleTestType>> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(found) =
        TEST_POS_RULE_TYPES.with(|cell| cell.borrow().as_ref().map(|types| types.get(id).cloned()))
    {
        return found;
    }
    MOD_POS_RULE_TYPES.get()?.get(id).cloned()
}

/// Runs `with` over a test's own rule test types, as decoding reads them.
/// Restores the previous scoped table even if `with` panics.
#[cfg(any(test, feature = "test-support"))]
pub fn with_test_rule_test_types<T>(types: &ModRuleTestTypes, with: impl FnOnce() -> T) -> T {
    struct Guard(Option<ModRuleTestTypes>);
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_RULE_TYPES.with(|cell| *cell.borrow_mut() = self.0.take());
        }
    }
    let prev = TEST_RULE_TYPES.with(|cell| cell.borrow_mut().replace(types.clone()));
    let _guard = Guard(prev);
    with()
}

/// Runs `with` over a test's own position rule test types, as decoding reads
/// them. Restores the previous scoped table even if `with` panics.
#[cfg(any(test, feature = "test-support"))]
pub fn with_test_pos_rule_test_types<T>(
    types: &ModPosRuleTestTypes,
    with: impl FnOnce() -> T,
) -> T {
    struct Guard(Option<ModPosRuleTestTypes>);
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_POS_RULE_TYPES.with(|cell| *cell.borrow_mut() = self.0.take());
        }
    }
    let prev = TEST_POS_RULE_TYPES.with(|cell| cell.borrow_mut().replace(types.clone()));
    let _guard = Guard(prev);
    with()
}
