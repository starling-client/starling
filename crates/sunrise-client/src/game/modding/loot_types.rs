//! Direct, owner-scoped adapters for the two loot map-codec registries.
use super::api_values::{TryIntoApi, TryIntoEngine};
#[cfg(test)]
#[path = "loot_types/tests.rs"]
mod tests;
use mod_api::{LootEntryData, ModLootCondition, ModLootFunction};
use std::{collections::HashMap, sync::Arc};
use sunrise_world::loot::{
    self,
    types::{Registry, RollRandom, Types},
};

pub(crate) fn capture() -> Types {
    let (functions, conditions) = mod_api::__private::loot_type_declarations();
    Types::new(Arc::new(Codecs {
        functions: functions.into_iter().collect(),
        conditions: conditions.into_iter().collect(),
    }))
}
struct Codecs {
    functions: HashMap<String, Arc<dyn ModLootFunction>>,
    conditions: HashMap<String, Arc<dyn ModLootCondition>>,
}
impl Registry for Codecs {
    fn contains(&self, kind: &str, function: bool) -> bool {
        if function {
            self.functions.contains_key(kind)
        } else {
            self.conditions.contains_key(kind)
        }
    }
    fn validate(
        &self,
        kind: &str,
        data: &serde_json::Value,
        function: bool,
    ) -> Result<Vec<String>, String> {
        let data = LootEntryData::new(data.clone())?;
        if function {
            let codec = self
                .functions
                .get(kind)
                .ok_or_else(|| format!("unknown loot function type {kind}"))?;
            codec.validate(&data)?;
            Ok(codec
                .referenced_context_params()
                .iter()
                .map(ToString::to_string)
                .collect())
        } else {
            let codec = self
                .conditions
                .get(kind)
                .ok_or_else(|| format!("unknown loot condition type {kind}"))?;
            codec.validate(&data)?;
            Ok(codec
                .referenced_context_params()
                .iter()
                .map(ToString::to_string)
                .collect())
        }
    }
    fn conditional(&self, kind: &str) -> bool {
        self.functions
            .get(kind)
            .is_none_or(|codec| codec.conditional())
    }
    fn apply(
        &self,
        kind: &str,
        data: &serde_json::Value,
        ctx: &loot::LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn loot::LootTags,
        stack: &loot::LootStack,
    ) -> Result<loot::LootStack, String> {
        let codec = self
            .functions
            .get(kind)
            .ok_or_else(|| format!("unknown loot function type {kind}"))?;
        let snapshot = ctx
            .try_into_api()
            .map_err(|error| format!("context: {error:?}"))?;
        let stack = stack
            .try_into_api()
            .map_err(|error| format!("input stack: {error:?}"))?;
        let mut context = Context {
            snapshot,
            random,
            tags: crate::game::loot_tags::LootTags(tags),
        };
        codec
            .apply(&LootEntryData::new(data.clone())?, stack, &mut context)?
            .try_into_engine(&())
            .map_err(|error| format!("output stack: {error:?}"))
    }
    fn test(
        &self,
        kind: &str,
        data: &serde_json::Value,
        ctx: &loot::LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn loot::LootTags,
    ) -> Result<bool, String> {
        let codec = self
            .conditions
            .get(kind)
            .ok_or_else(|| format!("unknown loot condition type {kind}"))?;
        let snapshot = ctx
            .try_into_api()
            .map_err(|error| format!("context: {error:?}"))?;
        let mut context = Context {
            snapshot,
            random,
            tags: crate::game::loot_tags::LootTags(tags),
        };
        codec.test(&LootEntryData::new(data.clone())?, &mut context)
    }
}
struct Context<'a> {
    snapshot: mod_api::LootContext,
    random: &'a mut dyn RollRandom,
    tags: crate::game::loot_tags::LootTags<'a>,
}
impl mod_api::LootTypeContext for Context<'_> {
    fn snapshot(&self) -> mod_api::LootContext {
        self.snapshot.clone()
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        &self.tags
    }
    fn random(&mut self) -> &mut dyn mod_api::RandomSource {
        self
    }
}
impl mod_api::RandomSource for Context<'_> {
    fn next_int(&mut self) -> i32 {
        self.random.next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.random.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.random.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.random.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.random.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.random.next_boolean()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.random.next_gaussian()
    }
}
