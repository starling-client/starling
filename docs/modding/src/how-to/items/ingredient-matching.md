# Match custom ingredients

Return an `IngredientDefinition::new(items, test)` from [`Hooks::ingredient`](ingredient-kinds.md). The definition belongs to this occurrence of the ingredient; capture its parsed configuration in the predicate rather than comparing the kind again while matching.

The predicate takes `(&dyn LoopContext, IngredientStack<'_>)` and returns `true` to accept a stack or `false` to reject it. Use `stack.item_id()` or `stack.item_key()` for identity, `stack.count()` for the count, and `stack.damage()` for optional damage (including `Some(0)`). `has_component` checks effective presence, `component_removed` checks explicit removal, and `stored_component` lends retained NBT without constructing an owned component patch. `stored_component` does not return inherited defaults or NBT for typed-only fields such as damage.

The predicate runs as its defining mod on the side testing the recipe. It can run many times while crafting and placing and cannot edit the game. Give the same answer for the same inputs. `Ingredient::test(host, stack)` uses the host's loop context and calls the bound predicate only after any nonempty candidate list accepts the item's ID. A disabled owner rejects without running the predicate; another mod is not asked. A panic disables only this occurrence's predicate, including after reloading the same definition. An empty custom list has no item filter for manual matching, but contributes no placement candidates; an empty ordinary tag never matches.

`Ingredient`, `Recipe`, `RecipeRule`, `FireworkShapeIngredient`, `PlacementInfo`, `RecipeBookCategory` and `RecipeBookDisplayEntry` remain available in-process but do not implement `Wire`: they can contain native predicates. For a typed payload, derive `Wire` on a separate struct containing the data you need, such as recipe IDs, item IDs or `RecipeBookDisplay` values. Keep the resolved ingredients on the matching side. For recipe-book synchronization the game sends display data and static placement requirements (projecting non-empty custom ingredients to their candidate item sets, and collapsing only empty candidate sets to `PlacementInfo::NOT_PLACEABLE`), never the predicates. The server rechecks live stacks on a placement request.

When a mod asks `datapack_sync` to send a vanilla recipe type in full, the
receiving client sees an item list for each ingredient, using the server's
resolved candidates. A bound predicate stays on the server. Do not use the
client's candidate check to make authoritative crafting or placement decisions.
