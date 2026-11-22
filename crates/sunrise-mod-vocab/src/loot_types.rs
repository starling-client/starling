//! Static, unsynced loot function and condition map-codec registries.
use crate::{Identifier, LootContext, LootStack, RandomSource, Tags};
#[path = "loot_types/owned.rs"]
pub(crate) mod owned;

/// An inline loot entry: `type` and the codec's fields share one JSON object.
/// Wire uses the existing JSON tree encoding (length-prefixed UTF-8 JSON).
#[derive(Clone, Debug, sunrise_mod_api_macros::Wire)]
// exhaustive: its field is private; callers construct it through the object-checking constructor.
pub struct LootEntryData {
    value: serde_json::Value,
}
impl LootEntryData {
    /// Check the object shape once at the data-loading boundary.
    pub fn new(value: serde_json::Value) -> Result<Self, String> {
        if !value.is_object() {
            return Err("expected a loot entry object".into());
        }
        Ok(Self { value })
    }
    /// The immutable object, including unknown fields left to this codec.
    pub fn value(&self) -> &serde_json::Value {
        &self.value
    }
}

/// Facts, tags and the actual random of one active loot evaluation.
/// This borrow cannot be retained by a codec. Snapshot reads consume no random.
pub trait LootTypeContext {
    /// Optional parameters, level facts, luck and the set-once queried table id.
    fn snapshot(&self) -> LootContext;
    /// Authoritative merged tags of this evaluation.
    fn tags(&self) -> &dyn Tags;
    /// Every draw advances the evaluator's selected random, in call order.
    fn random(&mut self) -> &mut dyn RandomSource;
}

/// A map codec in `Registries.LOOT_FUNCTION_TYPE`, registered before data loads.
/// Values are immutable and shared between rolls; do not retain random state.
pub trait ModLootFunction: Send + Sync + 'static {
    /// Decode-equivalent field validation, run once per entry at load, not roll time.
    /// A failure refuses the entire candidate data generation.
    fn validate(&self, data: &LootEntryData) -> Result<(), String>;
    /// Parameters this codec reads. Missing parameters warn at load and stay absent.
    fn referenced_context_params(&self) -> Vec<Identifier> {
        Vec::new()
    }
    /// Whether the engine evaluates the optional `condition` before this body.
    fn conditional(&self) -> bool {
        true
    }
    /// Transform a stack. Signed, empty and over-max counts remain intact until splitting.
    /// A refused resource call is reported by type, never silently substituted.
    fn apply(
        &self,
        data: &LootEntryData,
        stack: LootStack,
        context: &mut dyn LootTypeContext,
    ) -> Result<LootStack, String>;
}

/// A map codec in `Registries.LOOT_CONDITION_TYPE`, registered before data loads.
/// Terms run in order and short-circuit; a skipped term draws no random.
pub trait ModLootCondition: Send + Sync + 'static {
    /// Validate this entry once at load; failures refuse the candidate generation.
    fn validate(&self, data: &LootEntryData) -> Result<(), String>;
    /// Parameters this codec reads. Absent parameters must be handled at runtime.
    fn referenced_context_params(&self) -> Vec<Identifier> {
        Vec::new()
    }
    /// Test this entry using only its context's random source.
    fn test(&self, data: &LootEntryData, context: &mut dyn LootTypeContext)
    -> Result<bool, String>;
}
