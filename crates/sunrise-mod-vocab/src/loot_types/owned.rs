//! Attribute every codec call to its owning mod, including load-time metadata.
use super::*;
use std::sync::{Arc, atomic::AtomicBool};

pub(crate) fn function(
    owner: &'static str,
    index: usize,
    codec: Arc<dyn ModLootFunction>,
) -> Arc<dyn ModLootFunction> {
    let poisoned = AtomicBool::new(false);
    let metadata = crate::callback_owner::call(
        owner,
        index,
        &poisoned,
        "loot function type",
        "loot_function_metadata",
        || (codec.referenced_context_params(), codec.conditional()),
    );
    Arc::new(Function {
        owner,
        index,
        codec,
        poisoned,
        metadata,
    })
}
pub(crate) fn condition(
    owner: &'static str,
    index: usize,
    codec: Arc<dyn ModLootCondition>,
) -> Arc<dyn ModLootCondition> {
    let poisoned = AtomicBool::new(false);
    let metadata = crate::callback_owner::call(
        owner,
        index,
        &poisoned,
        "loot condition type",
        "loot_condition_metadata",
        || codec.referenced_context_params(),
    );
    Arc::new(Condition {
        owner,
        index,
        codec,
        poisoned,
        metadata,
    })
}
struct Function {
    owner: &'static str,
    index: usize,
    codec: Arc<dyn ModLootFunction>,
    poisoned: AtomicBool,
    metadata: Result<(Vec<Identifier>, bool), String>,
}
impl ModLootFunction for Function {
    fn validate(&self, data: &LootEntryData) -> Result<(), String> {
        self.metadata.as_ref().map_err(Clone::clone)?;
        crate::callback_owner::call(
            self.owner,
            self.index,
            &self.poisoned,
            "loot function type",
            "loot_function_validate",
            || self.codec.validate(data),
        )?
    }
    fn referenced_context_params(&self) -> Vec<Identifier> {
        self.metadata
            .as_ref()
            .map(|m| m.0.clone())
            .unwrap_or_default()
    }
    fn conditional(&self) -> bool {
        self.metadata.as_ref().map(|m| m.1).unwrap_or(true)
    }
    fn apply(
        &self,
        data: &LootEntryData,
        stack: LootStack,
        context: &mut dyn LootTypeContext,
    ) -> Result<LootStack, String> {
        crate::callback_owner::call(
            self.owner,
            self.index,
            &self.poisoned,
            "loot function type",
            "loot_function_apply",
            || self.codec.apply(data, stack, context),
        )?
    }
}
struct Condition {
    owner: &'static str,
    index: usize,
    codec: Arc<dyn ModLootCondition>,
    poisoned: AtomicBool,
    metadata: Result<Vec<Identifier>, String>,
}
impl ModLootCondition for Condition {
    fn validate(&self, data: &LootEntryData) -> Result<(), String> {
        self.metadata.as_ref().map_err(Clone::clone)?;
        crate::callback_owner::call(
            self.owner,
            self.index,
            &self.poisoned,
            "loot condition type",
            "loot_condition_validate",
            || self.codec.validate(data),
        )?
    }
    fn referenced_context_params(&self) -> Vec<Identifier> {
        self.metadata.clone().unwrap_or_default()
    }
    fn test(
        &self,
        data: &LootEntryData,
        context: &mut dyn LootTypeContext,
    ) -> Result<bool, String> {
        crate::callback_owner::call(
            self.owner,
            self.index,
            &self.poisoned,
            "loot condition type",
            "loot_condition_test",
            || self.codec.test(data, context),
        )?
    }
}
