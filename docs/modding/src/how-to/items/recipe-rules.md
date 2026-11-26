# Reading recipe rules

Use `Host::recipes_of_type` to read loaded recipe definitions and
`LevelRef::recipe_for` to compute the output and remainders for actual inputs.

| Recipe kind | Definition |
|---|---|
| Shaped, shapeless and cooking | Resolved ingredients and fixed results |
| Stonecutting | `RecipeRule::Stonecutting`, with its resolved input and complete fixed output |
| Smithing and special crafting | A typed `RecipeRule` with resolved ingredient roles; the result depends on the inputs |
| Mod-defined type | Original recipe data in `Recipe::json`; match it in your mod |

For smithing and special crafting, do not treat a `RecipeTemplate` as a fixed
stack. It retains the count and component patch used to assemble the result.
In `RecipeRule::SmithingTransform`, absent template or addition ingredients
require empty slots. Trim patterns are registry references or inline definitions.

`recipes_of_type` returns recipes after pack overrides, conditions and filters,
in file order. Read them again after a data reload. On the server, you read the
server's table. A client reads vanilla and content-mod recipes unless the server
sends the recipe type through `datapack_sync`. A loaded recipe the API cannot
represent is left out of this listing and logged at debug level with its id and
reason; the rest remain in order. This affects only the API listing boundary,
not recipe loading.

One malformed recipe of a known type refuses the **whole recipe load**, with an
error naming its file and the reason. A failed reload keeps the previous active
recipes and does not publish a partial table; a dedicated server stops startup
with that error. A file whose load conditions are false is skipped quietly before
its recipe type is looked up. An unregistered condition type anywhere in the
conditions list is a load error, regardless of its namespace or nesting.
Unregistered recipe types retain their existing behavior pending owner registration.

`LevelRef::recipe_for` returns `Ok(None)` when no recipe matches. Mod-defined
recipe types return `RecipeLookupError::NotMatchable`; use their JSON to build
your own matching logic. See [Cache reload-stable recipe definitions](reload-stable-recipes.md)
for a custom-type cache.

## Finding a recipe's recipe book ids

`transfer_recipe` and `place_recipe` take a recipe book id, which `Recipe`
does not carry. `RecipesClientHost::recipe_book_ids`
(`recipe_id`) returns the ids of a recipe's entries in the book, in the book's
order.

A server numbers its recipe book entries itself and names none of them, so the
ids are given only where the server sent the entry with that number and this
client's own numbering of the recipe agrees with it (the entry has the same
result, category and ingredients). Two different recipes with identical
displays cannot be told apart this way. The answer is empty for a recipe the server
sent no entry for (the player's book does not hold it, or it was removed), for
one the server numbers differently, for one the book does not show (campfire,
stonecutting, smithing, a mod's own type), for an unknown id, and outside a
world.

The answer changes when the server adds, replaces or removes recipe book
entries and when recipes reload or arrive, and no hook announces a recipe book
change: query it when you use it, rather than keeping an answer from
`recipes_received`. A viewer's "+" needs no book id: `transfer_recipe_id`
moves any recipe the server knows by its id, and `transfer_recipe_check` (the
same id) says beforehand what it would do, for a recipe the player's book holds
or not. Call it while the answer is wanted, for example while hovering the
"+". On a remote server the client asks again when the cached answer is one
second old, even if the menu has not changed.

## Recipe filters

You can remove recipes or replace inputs and outputs from a content mod or data
pack without listing recipe IDs. Create `data/<ns>/recipe_filter/<name>.json`
with a `{"filters": [...]}` object. Each filter requires all its match keys to
hold and exactly one action. A filter with no match keys matches every recipe:

| Key | Meaning |
|---|---|
| `id` | the recipe id; `*` matches any run of characters (`minecraft:*_planks`) |
| `type` | the recipe's `type`, with the same `*` (`mymod:*`) |
| `input` | an item id or `#tag` among the recipe's inputs |
| `output` | an item id among its outputs (anything under `result`, `results`, `output` or `outputs`) |
| `remove` | `true`: the recipe is not loaded |
| `replace_input` | `{"from": ..., "to": ...}`: every input that is `from` becomes `to` |
| `replace_output` | the same, for the outputs |

```json
{"filters": [
  {"id": "minecraft:*_planks", "remove": true},
  {"type": "mymod:*", "replace_input": {"from": "minecraft:iron_ingot", "to": "#c:ingots/iron"}}
]}
```

### Filter application rules

- Filters run after pack overrides are merged and recipe load conditions pass.
  They operate on JSON, including recipes of mod-defined types.
- Files apply in recipe-filter ID order; filters within a file apply in array
  order. Later filters see earlier replacements. Removal stops processing that recipe.
- A higher-priority pack's file replaces a file with the same ID.
- Input and output matching walks JSON strings, excluding `type`, `group`,
  `category` and `neoforge:conditions`. Strings under `result`, `results`,
  `output` or `outputs` count as outputs; other strings count as inputs.
  A `{"tag": "c:ingots/iron"}` value matches `#c:ingots/iron`.
- Replacement changes matching strings, not JSON object keys or structure.
  When replacing a `tag` value, the stored string omits the leading `#`.
- Crafting, station and brewing loaders, the recipe book and recipe data sent to
  players use the filtered recipes.

An unknown filter key, invalid value type, or anything other than exactly one
action skips that filter with a warning. `remove: false` does not count as an
action. A replacement requires string `from` and `to` values. Invalid JSON skips
the file with a warning.

## Recipe priorities

When two recipes match the same input, the first one wins. A content mod or data
pack picks the winner with `data/neoforge/recipe_priorities.json`, the resource
used by NeoForge's `RecipePriorityManager`. The same filename under any other
namespace has no effect.

```json
{"replace": false, "entries": {"example:specific": 5, "example:broad": -1}}
```

| Key | Meaning |
|---|---|
| `entries` | recipe ID to priority, a signed 32-bit integer; an ID without a namespace is `minecraft:`. A layer cannot give the same ID twice using different spellings. |
| `replace` | `true` clears every priority an earlier layer set, even when `entries` is absent or empty |
| `neoforge:conditions` | an array of [load conditions](conditional-data.md); a valid false condition skips the entire layer |

A recipe with no entry has priority 0.

### Priority layers

Files apply lowest layer first: the game's own data, then content mods in
dependency order (a mod's jars in declared order, below its own directory or
`.smod` files), then the world's enabled data packs in pack order. Every layer of
this one path counts, including lower copies. Content mods need not claim the
`neoforge` namespace to provide it. A later entry overrides an earlier one.
Well-formed IDs that name no loaded recipe are kept quietly.

An unreadable file is logged with its pack and resource path and skipped. A
readable file with malformed JSON, invalid load conditions, or invalid priority
data refuses the load and names its pack and resource path. Nothing from that
file applies. A layer with valid false conditions is skipped before its priority
data is decoded.

### Priority order

The recipes are sorted once per server data load, by descending priority and then
by vanilla's registration order (path, then namespace). Crafting, every cooking
type, smithing, the stonecutter list and the recipes of mod-defined types follow
that order; nothing sorts or looks a priority up while recipes match. With no
file, the order is vanilla's.

A client joined to a server lists its stonecutter recipes in the order the
server sent them, and its buttons number those entries, whatever packs the client
has itself.
