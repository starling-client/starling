//! Shared queried parameters and NeoForge conditions for both loot evaluators.
use super::LootStack;
use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
};
pub use sunrise_core::UNKNOWN_LOOT_TABLE;

/// Parameters shared unchanged by all nested tables. The first id set wins.
#[derive(Default)]
pub struct LootQuery {
    id: OnceCell<String>,
    /// The stack whose data is being queried. No existing engine caller sets it.
    pub queried_stack: Option<LootStack>,
    /// Tool retained by the world evaluator and its nested tables.
    pub tool: Option<LootStack>,
}
impl LootQuery {
    /// An unset query with stack parameters retained for every nested table.
    pub fn new(queried_stack: Option<LootStack>, tool: Option<LootStack>) -> Self {
        Self {
            id: OnceCell::new(),
            queried_stack,
            tool,
        }
    }
    /// Set once, either by the caller or after the root completes.
    pub fn set_id(&self, id: &str) {
        self.id.get_or_init(|| id.to_owned());
    }
    /// The stored id, if a caller or completed root supplied one.
    pub fn id(&self) -> Option<&str> {
        self.id.get().map(String::as_str)
    }
    /// Unset contexts report the unknown-table id until the root finishes.
    pub fn table_id(&self) -> &str {
        self.id().unwrap_or(UNKNOWN_LOOT_TABLE)
    }
    /// Set-once id completion, called by each evaluator's `roll_root` after
    /// all nested tables. Nested raw rolls never call this.
    pub fn finish(&self, id: Option<&str>) {
        self.set_id(id.unwrap_or(UNKNOWN_LOOT_TABLE));
    }
}
thread_local! { static QUERY: RefCell<Option<Rc<LootQuery>>> = const { RefCell::new(None) }; }
/// Supply parameters and inspect the root's completed queried id. Restores on unwind.
pub fn with_query<T>(query: Rc<LootQuery>, roll: impl FnOnce() -> T) -> T {
    struct Restore(Option<Rc<LootQuery>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            QUERY.with(|q| *q.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(QUERY.with(|q| q.replace(Some(query))));
    roll()
}
/// A fresh context unless a caller explicitly supplied query parameters.
pub fn current_query() -> Rc<LootQuery> {
    QUERY.with(|q| q.borrow().clone()).unwrap_or_default()
}
/// Borrowed tool stack for `neoforge:can_item_perform_ability` without eager conversion.
pub trait LootAbilityTool {
    /// Namespaced item identifier (for example `"minecraft:shears"`).
    fn item_id(&self) -> &str;
    /// Empty or air stacks never perform abilities and never dispatch listeners.
    fn is_empty(&self) -> bool;
    /// Present when the evaluator already holds a [`LootStack`].
    fn as_loot_stack(&self) -> Option<&LootStack> {
        None
    }
    /// Non-empty count and optional damage for a resident engine `ItemStack`.
    fn resident_scalars(&self) -> Option<(std::num::NonZeroU32, Option<u32>)> {
        None
    }
    /// Lazily encode extra components for a resident engine `ItemStack` only when projected.
    fn encode_components(&self) -> Option<sunrise_core::nbt::Tag> {
        None
    }
}

impl LootAbilityTool for LootStack {
    fn item_id(&self) -> &str {
        &self.item
    }

    fn is_empty(&self) -> bool {
        self.count <= 0 || self.item == "minecraft:air"
    }

    fn as_loot_stack(&self) -> Option<&LootStack> {
        Some(self)
    }
}

/// Recognise stock conditions before the mod-kind fallback, without drawing randomness.
pub fn condition(
    value: &serde_json::Value,
    query: &LootQuery,
    tool: Option<&dyn LootAbilityTool>,
    tags: &dyn super::LootTags,
    default: impl Fn(&str, &str) -> bool,
) -> Option<bool> {
    match value["type"].as_str()? {
        "neoforge:loot_table_id" => Some(
            value["loot_table_id"]
                .as_str()
                .is_some_and(|id| id == query.table_id()),
        ),
        "neoforge:can_item_perform_ability" => Some(
            tool.filter(|tool| !tool.is_empty())
                .zip(value["ability"].as_str())
                .is_some_and(|(tool, ability)| {
                    let mut holds = default(tool.item_id(), ability);
                    crate::native_load_hooks::loot_ability(tool, ability, tags, &mut holds);
                    holds
                }),
        ),
        _ => None,
    }
}
