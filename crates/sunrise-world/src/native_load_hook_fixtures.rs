//! One installer for the world library test binary; observations belong to each test thread.
use super::*;
use crate::loot::{self, LootContext, LootStack};
struct QueryCodec;
impl loot::types::Registry for QueryCodec {
    fn contains(&self, kind: &str, function: bool) -> bool {
        !function && kind == "test:query"
    }
    fn validate(&self, _: &str, _: &serde_json::Value, _: bool) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    fn conditional(&self, _: &str) -> bool {
        true
    }
    fn apply(
        &self,
        _: &str,
        _: &serde_json::Value,
        _: &LootContext<'_>,
        _: &mut dyn loot::types::RollRandom,
        _: &dyn LootTags,
        stack: &LootStack,
    ) -> Result<LootStack, String> {
        Ok(stack.clone())
    }
    fn test(
        &self,
        _: &str,
        _: &serde_json::Value,
        ctx: &LootContext<'_>,
        _: &mut dyn loot::types::RollRandom,
        _: &dyn LootTags,
    ) -> Result<bool, String> {
        CONTEXT.with(|value| {
            *value.borrow_mut() = Some((ctx.table_id().to_owned(), ctx.queried_stack.clone()))
        });
        Ok(true)
    }
}
thread_local! {
    static CONTEXT: std::cell::RefCell<Option<(String, Option<LootStack>)>> = const { std::cell::RefCell::new(None) };
    static ABILITY: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}
pub(crate) fn take_context() -> Option<(String, Option<LootStack>)> {
    CONTEXT.with(|v| v.take())
}
pub(crate) fn take_ability() -> Option<(String, String)> {
    ABILITY.with(|v| v.take())
}
pub(crate) fn install() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        assert!(
            loot::types::install(|| loot::types::Types::new(std::sync::Arc::new(QueryCodec)))
                .is_ok()
        );
        assert!(
            super::install(NativeLoadHooks {
                item_ability_default: |item, ability, tags| match ability {
                    "shears_dig" => item == "minecraft:shears",
                    "sword_sweep" => tags.contains("item", "minecraft:swords", item),
                    _ => false,
                },
                loot_table_load: |_, _, _| {},
                modify_default_components: |_, item, components| match item {
                    "a:raised" => {
                        components.insert("minecraft:max_stack_size".into(), 500.into());
                    }
                    "a:zero" => {
                        components.insert("minecraft:max_stack_size".into(), 0.into());
                    }
                    "a:huge" => {
                        components.insert("minecraft:max_stack_size".into(), u64::MAX.into());
                    }
                    "a:string" => {
                        components.insert("minecraft:max_stack_size".into(), "500".into());
                    }
                    "a:removed" => {
                        components.remove("minecraft:max_stack_size");
                    }
                    _ => {}
                },
                loot_ability: |tool, ability, holds| {
                    ABILITY
                        .with(|v| *v.borrow_mut() = Some((tool.item.clone(), ability.to_owned())));
                    if tool.item == "test:grant" && ability == "shears_dig" {
                        *holds = true;
                    }
                    if tool.item == "test:deny" {
                        *holds = false;
                    }
                },
            })
            .is_ok()
        );
    });
}
