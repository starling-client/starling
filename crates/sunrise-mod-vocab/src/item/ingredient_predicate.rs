//! Retained ingredient predicates and their borrowed stack input.
use crate::{ItemKey, LoopContext, Tag, runtime::RetainedCallback};
use std::{fmt, sync::Arc};

type IngredientTest = dyn for<'a> Fn(&dyn LoopContext, IngredientStack<'a>) -> bool + Send + Sync;

// no wire: the definition carries native code and is consumed in-process at recipe load.
// exhaustive: the constructor owns the ordered candidates and predicate together.
/// Ordered item candidates and a predicate that tests concrete stacks for one custom ingredient.
pub struct IngredientDefinition {
    items: Vec<String>,
    test: Arc<IngredientTest>,
}

impl IngredientDefinition {
    /// Pair the candidate item IDs, in recipe-book order, with a stack test.
    /// The owning load validates the IDs. The test can capture parsed configuration and may
    /// run many times during crafting and placement; answer consistently for the same input.
    /// Its context and stack are read-only and borrowed for the call. A nonempty candidate
    /// list filters by item before the test; an empty list has no filter for manual matching
    /// but prevents recipe-book placement. A panic rejects the stack and disables only this
    /// occurrence's predicate for the rest of the session, including after reloading the
    /// same definition.
    pub fn new(
        items: Vec<String>,
        test: impl for<'a> Fn(&dyn LoopContext, IngredientStack<'a>) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            items,
            test: Arc::new(test),
        }
    }
}

// no wire: the bound predicate holds native code and a process-local dispatch slot.
// exhaustive: the recipe loader alone binds the predicate to its owner and mod slot.
/// Engine-owned predicate bound to the mod that defined this ingredient.
#[derive(Clone)]
pub struct IngredientPredicate {
    callback: RetainedCallback,
    test: Arc<IngredientTest>,
}

impl IngredientPredicate {
    /// Test this stack as the owning mod. A panic is reported with the owner's name, and only
    /// this occurrence's predicate then answers `false` for the session, even after reload.
    pub fn test(&self, context: &dyn LoopContext, stack: IngredientStack<'_>) -> bool {
        self.callback
            .call(|| (self.test)(context, stack))
            .unwrap_or(false)
    }
}

impl PartialEq for IngredientPredicate {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.test, &other.test)
    }
}

impl Eq for IngredientPredicate {}

impl fmt::Debug for IngredientPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IngredientPredicate")
            .field("callback", &self.callback)
            .finish_non_exhaustive()
    }
}

/// Split out the ordered candidate IDs and bind the test to its engine-resolved mod slot and definition key.
pub fn bind_ingredient_predicate(
    owner: &'static str,
    mod_index: usize,
    definition_key: &str,
    definition: IngredientDefinition,
) -> (Vec<String>, IngredientPredicate) {
    let IngredientDefinition { items, test } = definition;
    (
        items,
        IngredientPredicate {
            callback: RetainedCallback::new(
                owner,
                mod_index,
                "ingredient_predicate",
                definition_key,
            ),
            test,
        },
    )
}

// no wire: the stack view lends live, call-scoped values instead of serializing a component patch.
// exhaustive: the engine supplies call-scoped reads of the borrowed stack.
/// Read-only view of the concrete stack passed to an ingredient predicate.
/// Component queries read the stack directly, without building an owned snapshot.
pub struct IngredientStack<'a> {
    item_id: &'a str,
    item_key: Option<ItemKey>,
    count: u32,
    damage: Option<u32>,
    has_component: &'a dyn Fn(&str) -> bool,
    component_removed: &'a dyn Fn(&str) -> bool,
    stored_component: &'a dyn Fn(&str) -> Option<&'a Tag>,
}

impl<'a> IngredientStack<'a> {
    /// The fully qualified item ID.
    pub fn item_id(&self) -> &'a str {
        self.item_id
    }

    /// The process-local key, if the item has been resolved in the registry.
    pub fn item_key(&self) -> Option<ItemKey> {
        self.item_key
    }

    /// The number of items in this stack.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The effective damage component. `Some(0)` is present zero damage;
    /// `None` means the component is absent or was removed.
    pub fn damage(&self) -> Option<u32> {
        self.damage
    }

    /// Whether the stack carries this component, including an unremoved item default.
    /// Pass the fully qualified component name, without `!`.
    pub fn has_component(&self, id: &str) -> bool {
        (self.has_component)(id)
    }

    /// Whether the stack's patch explicitly removed this component.
    /// Pass the fully qualified component name, without `!`.
    pub fn component_removed(&self, id: &str) -> bool {
        (self.component_removed)(id)
    }

    /// Borrow a component kept as NBT: preserved data such as `minecraft:custom_data` and opaque
    /// mod components, raw-backed fields such as `minecraft:custom_name` and `minecraft:consumable`,
    /// or shared values for the raid captain's banner. Pass the fully qualified name without `!`.
    /// This does not read inherited defaults or encode typed-only fields such as damage, so it can
    /// return `None` even when [`Self::has_component`] is true.
    pub fn stored_component(&self, id: &str) -> Option<&'a Tag> {
        (self.stored_component)(id)
    }
}

/// Lend the stack's identity, scalars and component readers for one predicate call.
pub fn ingredient_stack<'a>(
    item_id: &'a str,
    item_key: Option<ItemKey>,
    count: u32,
    damage: Option<u32>,
    has_component: &'a dyn Fn(&str) -> bool,
    component_removed: &'a dyn Fn(&str) -> bool,
    stored_component: &'a dyn Fn(&str) -> Option<&'a Tag>,
) -> IngredientStack<'a> {
    IngredientStack {
        item_id,
        item_key,
        count,
        damage,
        has_component,
        component_removed,
        stored_component,
    }
}
