//! Moved here from the game so mods can name it.
use crate::item::ingredient_predicate::IngredientPredicate;
#[allow(unused_imports)]
use crate::*;
use serde_json::Value;
use std::collections::HashSet;
#[path = "recipe_property_validation.rs"]
mod recipe_property_validation;
pub(crate) use recipe_property_validation::validate_recipe_properties;
/// Items a mod adds to one of the recipe property sets: the items a furnace,
/// blast furnace, smoker, campfire, smithing table slot or brewing stand slot
/// accepts (`RecipeManager.RECIPE_PROPERTY_SETS`), or a mod-created menu input
/// set. New keys must belong to a namespace owned by the listener's manifest;
/// later listeners may extend sets created by earlier listeners.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct RecipePropertyAddition {
    /// The set's key, such as `minecraft:furnace_input` or `mymod:pot_input`.
    pub set: String,
    /// The item ids to add.
    pub items: Vec<String>,
}
impl RecipePropertyAddition {
    /// `items` added to the set named `set`.
    pub fn new(set: impl Into<String>, items: Vec<String>) -> Self {
        Self {
            set: set.into(),
            items,
        }
    }
}
// no wire: an ingredient can hold an in-process predicate bound to its defining mod.
/// A set of accepted items (expanded from an item, list or tag), and the
/// items in its `HolderSet` order (`Ingredient.display`'s `TagSlotDisplay`).
#[derive(Debug)]
// exhaustive: placement providers and the game construct complete ingredient values
pub struct Ingredient {
    /// Full candidate set; non-simple predicates test the concrete stack.
    pub items: HashSet<String>,
    /// Default-stack display/simple-match order; non-simple kinds may have none.
    pub order: Vec<String>,
    /// The predicate bound to this occurrence by the mod that defined it, not recipe metadata.
    /// Use [`Self::test`] for the complete match, including the candidate filter.
    pub custom: Option<IngredientPredicate>,
    /// NeoForge's shapes, with their children parsed: matching a concrete
    /// stack asks each child its own `test` (a mod's kind among them),
    /// as on NeoForge — the flat list above only names items.
    pub children: Option<(Shape, Vec<Self>)>,
    /// A concrete-stack built-in predicate.
    pub rule: Option<Box<IngredientRule>>,
    /// Explicit slot display, independent of membership.
    pub display: Option<Box<sunrise_core::nbt::Compound>>,
}
/// Built-in predicates evaluated against the concrete stack.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: the parser and engine share this closed set of built-in predicates
pub enum IngredientRule {
    /// Exact component patch, with explicit absence requirements.
    Components {
        patch: sunrise_core::nbt::Compound,
        removed: Vec<String>,
        strict: bool,
    },
}
/// How a shape's children combine at match time.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Shape {
    /// Any child matching the stack is enough.
    Compound,
    /// Every child must match it.
    Intersection,
    /// The first child must, and the second must not.
    Difference,
    /// Matching follows the base; presentation uses a separate slot display.
    CustomDisplay,
}
/// Why an ingredient cannot be retained as an exact, component-independent key set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum IngredientKeysError {
    /// A custom predicate may inspect components or context at match time.
    NotEnumerable,
    /// An ingredient name cannot be resolved in the completed item registry.
    UnknownItem,
}
impl crate::NoBackend for IngredientKeysError {
    fn no_backend() -> Self {
        Self::UnknownItem
    }
}
impl Ingredient {
    /// Resolve exact membership once per load/reload for local matching in mod memory.
    /// Plain item lists, expanded tags and standard shapes with no custom descendants
    /// are component-independent. A custom/component predicate anywhere in the tree
    /// returns `NotEnumerable`: the display item list is not an exact predicate (and
    /// for differences containing custom predicates is not even necessarily a superset).
    /// Such predicates remain mod-owned; this method never invokes them.
    /// One host call per ingredient at publication; never call this per tick or candidate.
    pub fn item_keys(&self, host: &dyn Host) -> Result<Arc<[crate::ItemKey]>, IngredientKeysError> {
        backend_of(host)
            .ok_or(IngredientKeysError::UnknownItem)?
            .recipe_ingredient_item_keys(self)
    }
    /// The items of a list, in order.
    pub fn of(items: Vec<String>) -> Self {
        Self {
            items: items.iter().cloned().collect(),
            order: items,
            custom: None,
            children: None,
            rule: None,
            display: None,
        }
    }
    /// `Ingredient.test`: whether `stack` fits the candidate list and custom
    /// predicate (`ICustomIngredient.test`). An empty custom list retains the
    /// predicate without item filtering, but cannot be placed by the recipe book.
    /// A bound predicate rejects when its defining mod is disabled or this occurrence's
    /// predicate has panicked; it never dispatches to other mods. An empty tag never
    /// matches; a host without a game refuses every stack.
    pub fn test(&self, host: &dyn Host, stack: &ItemStack) -> bool {
        backend_of(host).is_some_and(|backend| backend.recipe_ingredient_test(self, stack))
    }
}
impl Clone for Ingredient {
    fn clone(&self) -> Self {
        let clone_leaf = |node: &Self| Self {
            items: node.items.clone(),
            order: node.order.clone(),
            custom: node.custom.clone(),
            children: None,
            rule: node.rule.clone(),
            display: node.display.clone(),
        };
        let Some((root_shape, root_kids)) = &self.children else {
            return clone_leaf(self);
        };
        let mut stack: Vec<(&Self, Shape, std::slice::Iter<'_, Self>, Vec<Self>)> = vec![(
            self,
            root_shape.clone(),
            root_kids.iter(),
            Vec::with_capacity(root_kids.len()),
        )];
        loop {
            let next = stack.last_mut().expect("non-empty clone stack").2.next();
            if let Some(child) = next {
                if let Some((shape, kids)) = &child.children {
                    stack.push((
                        child,
                        shape.clone(),
                        kids.iter(),
                        Vec::with_capacity(kids.len()),
                    ));
                } else {
                    stack
                        .last_mut()
                        .expect("non-empty clone stack")
                        .3
                        .push(clone_leaf(child));
                }
            } else {
                let (node, shape, _, children) = stack.pop().expect("non-empty clone stack");
                let mut cloned = clone_leaf(node);
                cloned.children = Some((shape, children));
                if let Some(parent) = stack.last_mut() {
                    parent.3.push(cloned);
                } else {
                    return cloned;
                }
            }
        }
    }
}
impl PartialEq for Ingredient {
    fn eq(&self, other: &Self) -> bool {
        let mut pending = vec![(self, other)];
        while let Some((a, b)) = pending.pop() {
            if a.items != b.items
                || a.order != b.order
                || a.custom != b.custom
                || a.rule != b.rule
                || a.display != b.display
            {
                return false;
            }
            match (&a.children, &b.children) {
                (None, None) => {}
                (Some((sa, ka)), Some((sb, kb))) if sa == sb && ka.len() == kb.len() => {
                    pending.extend(ka.iter().zip(kb.iter()));
                }
                _ => return false,
            }
        }
        true
    }
}
impl Drop for Ingredient {
    fn drop(&mut self) {
        let Some((_, mut stack)) = self.children.take() else {
            return;
        };
        while let Some(mut node) = stack.pop() {
            if let Some((_, mut kids)) = node.children.take() {
                stack.append(&mut kids);
            }
        }
    }
}
// no wire: a recipe snapshot may include ingredients backed by native predicates.
/// A recipe in force (`RecipeManager.getAllRecipesFor`), whatever its type
/// (vanilla's or a mod's own): its typed parts, and its file for whatever a
/// mod's own type keeps there. The host parsing helpers accept JSON text.
/// Bound predicates live in the parsed ingredients alone. This snapshot is in-process;
/// send data-only values instead.
///
/// A recipe the game could not parse (an ingredient it cannot read, no
/// result) is not in force and is not listed.
///
/// `ingredients` and `results` are filled from the game's own parse of the
/// shaped, shapeless and cooking types: a shaped recipe's pattern cells row
/// by row (empty cells left out), a shapeless recipe's list, a cooking
/// recipe's `ingredient`, and the result as `results[0]`. Stonecutting,
/// smithing and special crafting recipes instead expose their typed [`RecipeRule`]
/// in `rule`, keeping these two lists empty. Dynamic outputs are rules/templates,
/// not fixed stacks; use [`LevelRef::recipe_for`] to match and assemble a crafting grid.
/// A mod's own type leaves these fields empty and keeps its `json`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Recipe {
    /// `<namespace>:<path>`, its place under `data/<namespace>/recipe/`.
    pub id: String,
    /// The recipe type, `minecraft:smelting` or `mymod:crusher`.
    pub kind: String,
    /// The recipe book group, empty for none.
    pub group: String,
    /// What it takes, as above.
    pub ingredients: Vec<Ingredient>,
    /// What it makes, as above.
    pub results: Vec<ItemStack>,
    /// Ticks a cooking recipe (`smelting`, `blasting`, `smoking`,
    /// `campfire_cooking`) takes, its file's `cookingtime`; `None` for any
    /// other type.
    pub cooking_time: Option<u32>,
    /// Experience a cooking recipe gives, `None` for any other type.
    pub experience: Option<f32>,
    /// Parsed stonecutting, smithing or special crafting rule. `None` for
    /// shaped, shapeless, cooking and mod-defined types. The ingredient roles,
    /// bounds and templates come from the game's canonical parse.
    pub rule: Option<RecipeRule>,
    /// The locally loaded recipe's file. For a server-synced recipe, this is
    /// what its serializer sent: resolved item lists for vanilla serializers,
    /// or the registered mod serializer's written JSON for its own kinds.
    pub json: Arc<Value>,
}
impl Recipe {
    /// The recipe `id` of type `kind` with its file `json`, the typed parts empty.
    pub fn new(id: impl Into<String>, kind: impl Into<String>, json: Arc<Value>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            group: String::new(),
            ingredients: Vec::new(),
            results: Vec::new(),
            cooking_time: None,
            experience: None,
            rule: None,
            json,
        }
    }
}

/// Typed input to the engine's matched recipe lookup. Slot order is significant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RecipeInput {
    /// Row-major slots matching the checked area. Either dimension may be zero
    /// with no slots; registered crafting serializers still receive the given dimensions.
    Crafting {
        width: u32,
        height: u32,
        slots: Vec<Option<ItemStack>>,
    },
    Single(ItemStack),
    Smithing {
        template: Option<ItemStack>,
        base: Option<ItemStack>,
        addition: Option<ItemStack>,
    },
}
/// A refusal distinct from a valid input with no matching recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RecipeLookupError {
    InvalidInput,
    InvalidItem,
    InvalidRecipe,
    UnavailableLevel,
    /// The engine has no matcher for this type. Cache its recipes once per load/reload
    /// through Host::recipes_of_type and match in mod memory, without per-tick crossings.
    NotMatchable,
}
impl crate::NoBackend for RecipeLookupError {
    fn no_backend() -> Self {
        Self::UnavailableLevel
    }
}
/// What the matched recipe assembles; lookup never consumes the input.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct RecipeMatch {
    pub recipe: crate::Identifier,
    pub output: ItemStack,
    /// Remaining items in input slot order after consuming one item per occupied slot.
    pub remainders: Vec<Option<ItemStack>>,
    pub experience: f32,
    /// Recipe-file cooking time; station-specific speed adjustments are not applied here.
    pub processing_time: u32,
}
impl RecipeMatch {
    /// Build the result of an engine match.
    pub fn new(
        recipe: crate::Identifier,
        output: ItemStack,
        remainders: Vec<Option<ItemStack>>,
        experience: f32,
        processing_time: u32,
    ) -> Self {
        Self {
            recipe,
            output,
            remainders,
            experience,
            processing_time,
        }
    }
}
