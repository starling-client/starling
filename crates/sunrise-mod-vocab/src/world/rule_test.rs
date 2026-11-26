//! Rule tests (`BuiltInRegistries.RULE_TEST`) and position rule tests
//! (`BuiltInRegistries.POS_RULE_TEST`). A trait mod registers a rule test type
//! with [`Registrar::register_rule_test_type`](crate::Registrar::register_rule_test_type)
//! and a position rule test type with
//! [`Registrar::register_pos_rule_test_type`](crate::Registrar::register_pos_rule_test_type).
//!
//! A rule processor (`minecraft:rule`) or ore/block-replacement target entry
//! whose `predicate_type` names a mod's registered ID decodes once when the
//! processor list or feature loads and runs on the worldgen worker threads
//! beside vanilla's built-in tests.
//!
//! As with structure processors and placement modifiers, there is no `Host`
//! here. Bind block IDs, block tags, state properties and nested rule tests at
//! decode time so evaluation does no per-block lookup or allocation. Values
//! are immutable and shared; a test keeps no state between calls.
use super::structure_type::StructureError;
use crate::{BlockKey, BlockMetadata, BlockState, RandomSource};
use std::any::Any;
use std::sync::Arc;
use sunrise_core::nbt::Tag;

/// A mod's rule test type: the codec of vanilla's `RuleTestType`
/// (`BuiltInRegistries.RULE_TEST`). The game keeps one instance per registered
/// type. It must be deterministic and stateless.
pub trait RuleTestType: Send + Sync + 'static {
    /// Decode one rule test. `config` is the rule test entry converted from
    /// the data pack's JSON to NBT, with `predicate_type` included. Bind block
    /// IDs, block tags, state properties and nested rule tests here.
    ///
    /// Returning `Err(StructureError)` leaves the processor list (and every
    /// template pool naming it) or feature out of the load with a diagnostic
    /// naming the file, the list, the type and the failing field.
    fn decode(
        &self,
        context: &dyn RuleTestDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn RuleTest>, StructureError>;
}

/// What [`RuleTestType::decode`] sees while a processor list or feature loads.
pub trait RuleTestDecodeContext {
    /// Immutable, load-owned block metadata.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The block's default state; resolve once at decode rather than per block.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// Every registered state of block `block` (canonical or unqualified ID).
    /// Returns `Err` when `block` is not a registered block ID.
    fn block_states(&self, block: &str) -> Result<Vec<BlockState>, StructureError>;
    /// Decode a vanilla block state (`"minecraft:stone"` or
    /// `{"id": "...", "properties": {...}}`), binding its block ID and state
    /// properties at load time. As vanilla's `BlockState.CODEC`, an unknown
    /// property or invalid value keeps the default; returns `Err` for a
    /// missing or unknown block ID.
    fn decode_block_state(&self, config: &Tag) -> Result<BlockState, StructureError>;
    /// Every block state in block tag `tag` (with or without a leading `#`).
    /// Returns `Err` when `tag` is not a known block tag.
    fn block_tag(&self, tag: &str) -> Result<Vec<BlockState>, StructureError>;
    /// Decode a nested rule test of any type, vanilla's or a mod's, through
    /// `RuleTest.CODEC`.
    fn rule_test(&self, config: &Tag) -> Result<RuleTestRef, StructureError>;
}

/// A decoded rule test of any type, vanilla's or a mod's: a handle the game
/// resolves with no lookup per block. Clone freely; clones share one test.
#[derive(Clone)]
#[non_exhaustive]
// no wire: a handle to a rule test the game holds on the worldgen workers.
pub struct RuleTestRef {
    engine: Arc<dyn Any + Send + Sync>,
    mod_rule: Option<Arc<dyn RuleTest>>,
}
impl std::fmt::Debug for RuleTestRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleTestRef")
            .field("mod_rule", &self.mod_rule)
            .finish_non_exhaustive()
    }
}
impl RuleTestRef {
    /// The rule test as a mod's test of type `T`, when it is one. Vanilla's
    /// tests and other mod types give `None`.
    pub fn downcast_ref<T: RuleTest>(&self) -> Option<&T> {
        let rule: &dyn Any = self.mod_rule.as_deref()?;
        rule.downcast_ref()
    }
}

/// Construct the handle the game answers nested rule test decodes with.
// hidden: only the game's adapter builds a rule test handle; mods receive one from decode.
#[doc(hidden)]
pub fn rule_test_ref(
    engine: Arc<dyn Any + Send + Sync>,
    mod_rule: Option<Arc<dyn RuleTest>>,
) -> RuleTestRef {
    RuleTestRef { engine, mod_rule }
}

/// The game's own rule test behind a handle.
// hidden: only the game's adapter reads its rule test back from a handle.
#[doc(hidden)]
pub fn rule_test_payload(rule: &RuleTestRef) -> &Arc<dyn Any + Send + Sync> {
    &rule.engine
}

/// A decoded rule test of a mod's type: vanilla's `RuleTest`. Values are
/// immutable and shared on worldgen worker threads; keep no state between
/// calls.
pub trait RuleTest: Any + Send + Sync + std::fmt::Debug {
    /// `RuleTest.test(state, pos, random)`: test `state` at world position
    /// `pos`. Draw from `random` only when needed (as `random_block_match`
    /// draws one float only for a state that matches). A panic fails the
    /// structure's generation, naming the mod and type, with nothing
    /// substituted.
    fn test(
        &self,
        context: &dyn RuleTestContext,
        state: BlockState,
        pos: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> bool;
}

/// Immutable block facts and nested-rule evaluation available to
/// [`RuleTest::test`]. Carries no world state and no `Host`.
pub trait RuleTestContext {
    /// Immutable, load-owned block metadata.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// `BlockState.isAir`.
    fn is_air(&self, state: BlockState) -> bool;
    /// Whether `state` carries a non-empty fluid (`!state.getFluidState().isEmpty()`).
    fn is_liquid(&self, state: BlockState) -> bool;
    /// `BlockState.canOcclude()`, or `None` when block properties are
    /// unavailable in this context.
    fn can_occlude(&self, state: BlockState) -> Option<bool>;
    /// Evaluate a decoded nested rule test on `state` and `pos`, sharing
    /// `random`.
    fn test_rule(
        &self,
        rule: &RuleTestRef,
        state: BlockState,
        pos: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> Result<bool, StructureError>;
}

/// A mod's position rule test type: the codec of vanilla's `PosRuleTestType`
/// (`BuiltInRegistries.POS_RULE_TEST`). The game keeps one instance per
/// registered type. It must be deterministic and stateless.
pub trait PosRuleTestType: Send + Sync + 'static {
    /// Decode one position rule test. `config` is the `position_predicate`
    /// entry converted from the data pack's JSON to NBT, with `predicate_type`
    /// included.
    ///
    /// Returning `Err(StructureError)` leaves the processor list (and every
    /// template pool naming it) out of the load with a diagnostic naming the
    /// file, the list, the type and the failing field.
    fn decode(
        &self,
        context: &dyn PosRuleTestDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn PosRuleTest>, StructureError>;
}

/// What [`PosRuleTestType::decode`] sees while a processor list loads.
pub trait PosRuleTestDecodeContext {
    /// Decode a nested position rule test of any type, vanilla's or a mod's,
    /// through `PosRuleTest.CODEC`.
    fn pos_rule_test(&self, config: &Tag) -> Result<PosRuleTestRef, StructureError>;
}

/// A decoded position rule test of any type, vanilla's or a mod's: a handle
/// the game resolves with no lookup per block. Clone freely; clones share one
/// test.
#[derive(Clone)]
#[non_exhaustive]
// no wire: a handle to a position rule test the game holds on the worldgen workers.
pub struct PosRuleTestRef {
    engine: Arc<dyn Any + Send + Sync>,
    mod_rule: Option<Arc<dyn PosRuleTest>>,
}
impl std::fmt::Debug for PosRuleTestRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PosRuleTestRef")
            .field("mod_rule", &self.mod_rule)
            .finish_non_exhaustive()
    }
}
impl PosRuleTestRef {
    /// The position rule test as a mod's test of type `T`, when it is one.
    /// Vanilla's tests and other mod types give `None`.
    pub fn downcast_ref<T: PosRuleTest>(&self) -> Option<&T> {
        let rule: &dyn Any = self.mod_rule.as_deref()?;
        rule.downcast_ref()
    }
}

/// Construct the handle the game answers nested position rule test decodes
/// with.
// hidden: only the game's adapter builds a position rule test handle; mods receive one from decode.
#[doc(hidden)]
pub fn pos_rule_test_ref(
    engine: Arc<dyn Any + Send + Sync>,
    mod_rule: Option<Arc<dyn PosRuleTest>>,
) -> PosRuleTestRef {
    PosRuleTestRef { engine, mod_rule }
}

/// The game's own position rule test behind a handle.
// hidden: only the game's adapter reads its position rule test back from a handle.
#[doc(hidden)]
pub fn pos_rule_test_payload(rule: &PosRuleTestRef) -> &Arc<dyn Any + Send + Sync> {
    &rule.engine
}

/// A decoded position rule test of a mod's type: vanilla's `PosRuleTest`.
/// Values are immutable and shared on worldgen worker threads; keep no state
/// between calls.
pub trait PosRuleTest: Any + Send + Sync + std::fmt::Debug {
    /// `PosRuleTest.test(inTemplatePos, worldPos, worldReference, random)`:
    /// test a block at template-relative `in_template_pos`, world position
    /// `world_pos`, and piece reference `world_reference`, drawing from
    /// `random` as needed. A panic fails the structure's generation, naming
    /// the mod and type, with nothing substituted.
    fn test(
        &self,
        context: &dyn PosRuleTestContext,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> bool;
}

/// Nested-test evaluation available to [`PosRuleTest::test`]. Carries no world
/// state and no `Host`.
pub trait PosRuleTestContext {
    /// Evaluate a decoded nested position rule test, sharing `random`.
    fn test_pos_rule(
        &self,
        rule: &PosRuleTestRef,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> Result<bool, StructureError>;
}

/// A registered API rule test type; the game builds its engine adapter at
/// startup.
pub(crate) struct RuleTestTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) mod_id: &'static str,
    pub(crate) ty: Arc<dyn RuleTestType>,
}

/// A registered API position rule test type; the game builds its engine
/// adapter at startup.
pub(crate) struct PosRuleTestTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) mod_id: &'static str,
    pub(crate) ty: Arc<dyn PosRuleTestType>,
}

/// Registered rule test types, for the hidden game integration boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn rule_test_type_declarations() -> Vec<(String, &'static str, Arc<dyn RuleTestType>)> {
    crate::parts::installed_rule_test_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.mod_id, info.ty.clone()))
        .collect()
}

/// Registered position rule test types, for the hidden game integration
/// boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn pos_rule_test_type_declarations() -> Vec<(String, &'static str, Arc<dyn PosRuleTestType>)> {
    crate::parts::installed_pos_rule_test_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.mod_id, info.ty.clone()))
        .collect()
}
