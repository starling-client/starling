//! Static, unsynced loot function and condition map-codec registries.
use crate::{Identifier, LootParamsView, LootStackEditor, RandomSource, Tags};
use std::sync::Arc;
#[path = "loot_types/owned.rs"]
pub(crate) mod owned;

/// Facts, tags and the actual random of one active loot evaluation.
/// This borrow cannot be retained by a codec or bound rule. Reading parameters consumes no random.
pub trait LootTypeContext {
    /// Optional parameters, level facts, luck and the set-once queried table id.
    fn params(&self) -> LootParamsView<'_>;
    /// Authoritative merged tags of this evaluation.
    fn tags(&self) -> &dyn Tags;
    /// Every draw advances the evaluator's selected random, in call order.
    fn random(&mut self) -> &mut dyn RandomSource;
}

/// A map codec in `Registries.LOOT_FUNCTION_TYPE`, registered before data loads.
/// Decodes each resource JSON object once at load time into a bound [`ModLootFunction`] rule.
pub trait ModLootFunctionCodec: Send + Sync + 'static {
    /// Decode one inline loot function object once at load time, not roll time.
    /// Unknown fields are retained for this codec. A failure refuses the candidate generation.
    fn decode(&self, data: &serde_json::Value) -> Result<Arc<dyn ModLootFunction>, String>;
    /// Whether the engine evaluates the optional `condition` before this function's body.
    /// Defaults to `true` (vanilla `LootItemConditionalFunction`); return `false` for a direct
    /// function codec that owns `condition` itself.
    fn conditional(&self) -> bool {
        true
    }
}

/// A decoded, immutable loot function rule bound to one resource entry.
/// Shared between rolls; do not retain random state.
pub trait ModLootFunction: Send + Sync + 'static {
    /// Parameters this decoded rule reads (`LootContextUser.getReferencedContextParams`).
    /// Missing parameters warn at load and stay absent at runtime.
    fn referenced_context_params(&self) -> Vec<Identifier> {
        Vec::new()
    }
    /// Edit the current candidate stack in place. Signed, empty and over-max counts remain
    /// intact until splitting. Earlier valid edits and random draws are retained if a later
    /// write or the callback fails; a refused resource call is reported by type, never
    /// silently substituted.
    fn apply(
        &self,
        stack: &mut LootStackEditor<'_>,
        context: &mut dyn LootTypeContext,
    ) -> Result<(), String>;
}

/// A map codec in `Registries.LOOT_CONDITION_TYPE`, registered before data loads.
/// Decodes each predicate JSON object once at load time into a bound [`ModLootCondition`] rule.
pub trait ModLootConditionCodec: Send + Sync + 'static {
    /// Decode one loot condition object once at load time; failures refuse the candidate
    /// generation and leave the previous loot data active.
    fn decode(&self, data: &serde_json::Value) -> Result<Arc<dyn ModLootCondition>, String>;
}

/// A decoded, immutable loot condition rule bound to one resource entry.
/// Terms run in order and short-circuit; a skipped term draws no random.
pub trait ModLootCondition: Send + Sync + 'static {
    /// Parameters this decoded rule reads (`LootContextUser.getReferencedContextParams`).
    /// Absent parameters must be handled at runtime.
    fn referenced_context_params(&self) -> Vec<Identifier> {
        Vec::new()
    }
    /// Test this decoded rule using only its context's random source.
    fn test(&self, context: &mut dyn LootTypeContext) -> Result<bool, String>;
}
