//! Attribute every codec and bound-rule call to its registration's shared `CallbackOwner`.
use super::*;
use crate::callback_owner::{CallbackOwner, CallbackValue};
use std::sync::Arc;

pub(crate) fn function_codec(
    owner: CallbackOwner,
    codec: Arc<dyn ModLootFunctionCodec>,
) -> Arc<dyn ModLootFunctionCodec> {
    let codec = CallbackValue::new(
        owner,
        "loot function type",
        "loot_function_codec_drop",
        codec,
    );
    let conditional = codec
        .owner()
        .call("loot function type", "loot_function_metadata", || {
            codec.value().conditional()
        });
    Arc::new(FunctionCodec { codec, conditional })
}

pub(crate) fn condition_codec(
    owner: CallbackOwner,
    codec: Arc<dyn ModLootConditionCodec>,
) -> Arc<dyn ModLootConditionCodec> {
    Arc::new(ConditionCodec(CallbackValue::new(
        owner,
        "loot condition type",
        "loot_condition_codec_drop",
        codec,
    )))
}

struct FunctionCodec {
    codec: CallbackValue<Arc<dyn ModLootFunctionCodec>>,
    conditional: Result<bool, String>,
}
impl ModLootFunctionCodec for FunctionCodec {
    fn decode(&self, data: &serde_json::Value) -> Result<Arc<dyn ModLootFunction>, String> {
        self.conditional.as_ref().map_err(Clone::clone)?;
        let owner = self.codec.owner();
        let rule = owner
            .call("loot function type", "loot_function_decode", || {
                self.codec.value().decode(data)
            })?
            .map_err(|reason| format!("mod {}: {reason}", owner.id()))?;
        let rule = CallbackValue::new(
            owner.clone(),
            "loot function type",
            "loot_function_drop",
            rule,
        );
        let params = owner.call("loot function type", "loot_function_metadata", || {
            rule.value().referenced_context_params()
        })?;
        Ok(Arc::new(OwnedFunction { rule, params }))
    }
    fn conditional(&self) -> bool {
        self.conditional.as_ref().copied().unwrap_or(true)
    }
}

struct OwnedFunction {
    rule: CallbackValue<Arc<dyn ModLootFunction>>,
    params: Vec<Identifier>,
}
impl ModLootFunction for OwnedFunction {
    fn referenced_context_params(&self) -> Vec<Identifier> {
        self.params.clone()
    }
    fn apply(
        &self,
        stack: &mut LootStackEditor<'_>,
        context: &mut dyn LootTypeContext,
    ) -> Result<(), String> {
        let owner = self.rule.owner();
        owner
            .call("loot function type", "loot_function_apply", || {
                self.rule.value().apply(stack, context)
            })?
            .map_err(|reason| format!("mod {}: {reason}", owner.id()))
    }
}

struct ConditionCodec(CallbackValue<Arc<dyn ModLootConditionCodec>>);
impl ModLootConditionCodec for ConditionCodec {
    fn decode(&self, data: &serde_json::Value) -> Result<Arc<dyn ModLootCondition>, String> {
        let owner = self.0.owner();
        let rule = owner
            .call("loot condition type", "loot_condition_decode", || {
                self.0.value().decode(data)
            })?
            .map_err(|reason| format!("mod {}: {reason}", owner.id()))?;
        let rule = CallbackValue::new(
            owner.clone(),
            "loot condition type",
            "loot_condition_drop",
            rule,
        );
        let params = owner.call("loot condition type", "loot_condition_metadata", || {
            rule.value().referenced_context_params()
        })?;
        Ok(Arc::new(OwnedCondition { rule, params }))
    }
}

struct OwnedCondition {
    rule: CallbackValue<Arc<dyn ModLootCondition>>,
    params: Vec<Identifier>,
}
impl ModLootCondition for OwnedCondition {
    fn referenced_context_params(&self) -> Vec<Identifier> {
        self.params.clone()
    }
    fn test(&self, context: &mut dyn LootTypeContext) -> Result<bool, String> {
        let owner = self.rule.owner();
        owner
            .call("loot condition type", "loot_condition_test", || {
                self.rule.value().test(context)
            })?
            .map_err(|reason| format!("mod {}: {reason}", owner.id()))
    }
}
