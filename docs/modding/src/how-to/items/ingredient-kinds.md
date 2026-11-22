# Custom ingredient kinds

Implement `Hooks::ingredient(context, event) -> Option<IngredientDefinition>` to resolve recipe ingredient types outside the `minecraft:` namespace. Return `None` when you do not handle `event.kind`. For your kind, parse `event.data` once and return `Some(IngredientDefinition::new(items, predicate))`. The first answer in dispatch order wins; the loader removes duplicate item IDs in first-seen order.

This list supplies recipe-book placement candidates. The [bound predicate](ingredient-matching.md) checks a concrete stack. A nonempty candidate list filters matching by item on every matching path, including shaped, shapeless and non-simple shapeless matching, before the stack test runs. The list must therefore contain every item the test should accept; the test can only narrow it. An empty custom list retains the predicate without this filter, so the stack test decides which items match manually. Recipe displays and synced candidate data do not carry the predicate; the server rechecks the actual stacks during placement.

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

A predicate runs as the mod that answered. Disabling the owner after load makes the predicate reject stacks without invoking that mod or asking another mod for a match. A panic is logged with the mod's name and queues an error toast on a local client. Only the panicking predicate then answers `false` for the session, including after `/reload` of the same definition. Other occurrences still run, including those of the same kind. Returning `false` normally is just a non-match and does not disable anything. Clients without the predicate receive only display and placement data; the server's check is authoritative.

## Ingredient data

Read `event.data` as an API `Compound`, using typed `Tag` variants such as `Int`, `Long`, `List` and `Compound`, not JSON accessors. A compound's traversal order depends on its construction and mutation history; equality compares entries regardless of order.

A `Compound`'s keys and `Tag::String` values are `JavaString`s: UTF-16 text that can keep Java's unpaired surrogates, so they are not Rust `String`s. Build one from a Rust string with `JavaString::from("key")` (a `&str` converts), for example to look up `data.get(&JavaString::from("count"))`.

The loader converts recipe metadata once using vanilla JsonOps-to-NbtOps rules. JSON `true` and `false` become `Byte` 1 and 0, so the hook cannot tell a bool from the number 1 or 0. A number that is an exact signed 64-bit integer, including a decimal such as `1.0` or `1e3` and `-0.0`, becomes the smallest of `Byte`, `Short`, `Int` and `Long` that holds it (`1` is a `Byte`, `300` a `Short`, `100000` an `Int`). Any other number becomes a `Float` if it survives a round trip through `f32` (`0.5`), otherwise a `Double` (`0.1`). Because the width depends on the value, read numeric tags by value, accepting any numeric variant, rather than matching one tag type. Numeric JSON arrays remain lists. Nested null values refuse the ingredient and fail the recipe. Capture only the configuration you need in the predicate; the game does not reconvert or dispatch the metadata during matching.
