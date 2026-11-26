//! Callbacks into the executable for data-loading native trait mods.
use crate::{loot::LootTags, native_load_context::LoadContext};
use serde_json::{Map, Value};
use std::sync::OnceLock;

/// Data-loading hooks, installed before any data loads. Earlier loads see no mods.
pub struct NativeLoadHooks {
    pub item_ability_default: fn(&str, &str, &dyn LootTags) -> bool,
    pub loot_ability: fn(&dyn crate::loot::LootAbilityTool, &str, &dyn LootTags, &mut bool),
    pub loot_table_load: fn(&LoadContext, &str, &mut Value),
    pub modify_default_components: fn(&LoadContext, &str, &mut Map<String, Value>),
}

static HOOKS: OnceLock<NativeLoadHooks> = OnceLock::new();

/// Install the process-wide callbacks once; a second installation keeps the first.
pub fn install(hooks: NativeLoadHooks) -> Result<(), NativeLoadHooks> {
    HOOKS.set(hooks)
}

pub(crate) fn loot_table_load(context: &LoadContext, id: &str, table: &mut Value) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.loot_table_load)(context, id, table);
    }
}

pub(crate) fn modify_default_components(
    context: &LoadContext,
    item: &str,
    components: &mut Map<String, Value>,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.modify_default_components)(context, item, components);
    }
}

pub(crate) fn loot_ability(
    tool: &dyn crate::loot::LootAbilityTool,
    ability: &str,
    tags: &dyn LootTags,
    holds: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.loot_ability)(tool, ability, tags, holds);
    }
}

/// Item-layer defaults supplied by the executable, without a dependency from world to items.
pub fn item_ability_default(item: &str, ability: &str, tags: &dyn LootTags) -> bool {
    HOOKS
        .get()
        .is_some_and(|hooks| (hooks.item_ability_default)(item, ability, tags))
}

#[cfg(test)]
#[path = "native_load_hook_fixtures.rs"]
pub(crate) mod testing;
