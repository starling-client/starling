# Custom ingredient kinds

Implement `Hooks::ingredient(context, event) -> Option<IngredientDefinition>` to resolve recipe ingredient types outside the `minecraft:` namespace. Return `None` when you do not handle `event.kind`. For your kind, parse `event.data` once and return `Some(IngredientDefinition::new(items, predicate))`. The first answer in dispatch order wins; the loader removes duplicate item IDs in first-seen order.

The item list supplies recipe-book placement candidates and filters which stacks the
[bound predicate](#testing-stacks) can accept.

For example, `example:used_shears` can require at least the damage named by the
recipe's `minimum_damage` field:

```rust,ignore
use mod_api::{DataLoadContext, Hooks, IngredientDefinition, IngredientEvent, JavaString, Tag};

struct MyMod;

#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn ingredient(
        &self,
        _: &dyn DataLoadContext,
        event: &mut IngredientEvent<'_>,
    ) -> Option<IngredientDefinition> {
        if event.kind != "example:used_shears" {
            return None;
        }
        let minimum = match event.data.get(&JavaString::from("minimum_damage"))? {
            Tag::Byte(n) => i64::from(*n),
            Tag::Short(n) => i64::from(*n),
            Tag::Int(n) => i64::from(*n),
            Tag::Long(n) => *n,
            _ => return None,
        };
        let minimum = u32::try_from(minimum).ok()?;
        let items = vec!["minecraft:shears".to_owned()];
        Some(IngredientDefinition::new(items, move |_, stack| {
            stack.damage().is_some_and(|damage| damage >= minimum)
        }))
    }
}
```

Missing or invalid metadata in this example leaves the occurrence unanswered.
The predicate itself reads just the borrowed stack and the parsed minimum.

## Load contract

- The hook receives the owning load's `DataLoadContext`.
- Each occurrence is dispatched once per load, including nested ingredients and ingredients in any recipe slot.
- Dispatch occurs before the load's tables parse recipes. Server tables and stations use the bound answer; the data sent to guests is derived from its candidates, not its code. The loader does not write answers into recipe JSON, and a file field such as `sunrise:resolved_items` cannot supply candidates. Only the winning definition supplies candidates.
- The client's recipe preload reports `ClientPreload` through `origin()`; the server's recipe load reports `ServerDataLoad`, including on a dedicated server.
- `/reload` dispatches the occurrences again. A client joining a remote server does not receive `ServerDataLoad`.
- Return a consistent answer for the same input. If no mod answers for the kind, a recipe file naming a custom kind fails whole at load with an error naming the file and the kind, and the recipe is absent everywhere. If a mod returns a definition with an empty list, the recipe and its match-time predicate are retained, with a warning naming the file and the kind that the recipe cannot be placed. It is excluded from recipe-book learning/displays, recipe property sets and stonecutter selections; the server also refuses recipe-book placement. The effective recipe map and manual matcher still retain it.
- An existing empty item tag likewise retains an unplaceable recipe, but never matches a stack. An undefined tag fails the recipe with an error. A direct empty list (`[]`) remains invalid and fails the recipe with an error.
- A non-string entry (a number, an object, or `null`) in an ingredient list fails the whole recipe file, logged at error level naming the file; nothing is dropped. A directly named `minecraft:air` (alone or in a list) fails the file the same way, while a tag that holds air is taken as the tag holds it.
- A recipe that names an item that does not exist, as a result or as an ingredient's id, fails the whole load the same way (a world refuses to open, `/reload` fails and keeps the old recipes), and so does a recipe whose result carries a component that is not persistent. Every such recipe is listed.

## Testing stacks

The predicate takes `(&dyn LoopContext, IngredientStack<'_>)` and returns `true` to accept a
stack. It runs as the mod that supplied the definition, on the side testing the recipe.
It can run many times without editing the game; return the same answer for the same inputs.

- Read identity with `item_id()` or `item_key()`, quantity with `count()`, and optional damage with
  `damage()` (which may return `Some(0)`).
- `has_component(id)` reads effective presence, `component_removed(id)` checks for an explicit
  removal, and `stored_component(id)` borrows retained NBT without building a patch. The latter
  does not expose inherited defaults or typed-only fields such as damage.

`Ingredient::test(host, stack)` uses the host's loop context. On every matching path, including shaped
and shapeless recipes, a nonempty candidate list filters by item ID before the predicate runs.
List every item your predicate might accept. An empty custom list has no filter for manual
matching, but supplies no placement candidates.

A disabled owner rejects stacks without running the predicate or asking another mod.
A panic logs the owner's name, queues a local error toast, and disables only that occurrence's
predicate for the session, including after reloading the same definition. Returning `false`
normally is just a non-match.

`Ingredient`, `Recipe`, `RecipeRule`, `FireworkShapeIngredient`, `PlacementInfo`,
`RecipeBookCategory` and `RecipeBookDisplayEntry` can contain native predicates and do not
implement `Wire`. Derive it on a separate payload carrying data such as recipe IDs, item IDs
or `RecipeBookDisplay` values.
The recipe book syncs display data and static candidates; empty candidate sets become
`PlacementInfo::NOT_PLACEABLE`. When `datapack_sync` sends a vanilla recipe type in full,
the client receives resolved candidate lists, not the predicates. The server rechecks live
stacks for crafting and placement.

## Ingredient data

Read `event.data` as an API `Compound`, using typed `Tag` variants such as `Int`, `Long`, `List` and `Compound`, not JSON accessors. A compound's traversal order depends on its construction and mutation history; equality compares entries regardless of order.

A `Compound`'s keys and `Tag::String` values are `JavaString`s: UTF-16 text that can keep Java's unpaired surrogates, so they are not Rust `String`s. Build one from a Rust string with `JavaString::from("key")` (a `&str` converts), for example to look up `data.get(&JavaString::from("count"))`.

The loader converts recipe metadata once using vanilla JsonOps-to-NbtOps rules. JSON `true` and `false` become `Byte` 1 and 0, so the hook cannot tell a bool from the number 1 or 0. A number that is an exact signed 64-bit integer, including a decimal such as `1.0` or `1e3` and `-0.0`, becomes the smallest of `Byte`, `Short`, `Int` and `Long` that holds it (`1` is a `Byte`, `300` a `Short`, `100000` an `Int`). Any other number becomes a `Float` if it survives a round trip through `f32` (`0.5`), otherwise a `Double` (`0.1`). Because the width depends on the value, read numeric tags by value, accepting any numeric variant, rather than matching one tag type. Numeric JSON arrays remain lists. Nested null values refuse the ingredient and fail the recipe. Capture only the configuration you need in the predicate; the game does not reconvert or dispatch the metadata during matching.

## Fluid ingredients

Use `FluidIngredient::from_json(json, context)` to parse fluid inputs for your recipe type. Pass the
`DataLoadContext` supplied to your data-load hook. For direct tag reads,
[`DataLoadContext::fluid_tags()`](../../concepts/load-phases.md#load-contexts) borrows the recipe
pass's frozen table without reading files. `None` means the load has no recipe overlay;
`Some(&[])` means the overlay is available but has no fluid tags. In a populated table, each
tag's members are in source order. Use the borrowed table inside the callback; copy it only if
it must outlive the callback.

| JSON input | Matches |
| --- | --- |
| `"minecraft:water"` | The named fluid. |
| `"#my_mod:coolants"` | Fluids in the load context's tag. |
| A list of ingredients | Any listed ingredient. |
| `neoforge:compound` | Any child. |
| `neoforge:intersection` | Every child. |
| `neoforge:difference` | The base, excluding the subtracted ingredient. |

Call `test(&FluidStack)` to match a tank's contents. Components are not checked. Use `fluids()` to list accepted fluid keys in source order, including tag order, for a recipe book.

Parsing returns `Result<FluidIngredient, FluidIngredientError>`. Invalid shapes, custom kinds and a list holding an entry that isn't a fluid name or `#tag` return `FluidIngredientError::Invalid`; unknown fluid names return `FluidIngredientError::UnknownFluid`. `Hooks::ingredient` binds predicates only for custom **item** ingredients. A custom kind in fluid JSON is refused, not passed to that hook.

### Required fluid amounts

`SizedFluidIngredient::from_json` parses an ingredient and a positive amount in millibuckets:

```json
{"ingredient": "minecraft:water", "amount": 500}
```

- Omit `amount` to require one bucket (1000 millibuckets).
- Invalid input, including zero or a negative amount, returns `None`.
- `test(&FluidStack)` requires both a matching fluid and at least the requested amount.
