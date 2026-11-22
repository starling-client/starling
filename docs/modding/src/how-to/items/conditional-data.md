# Conditional data

Add a `neoforge:conditions` array to control whether a server JSON file loads.
Supported readers are
recipes, loot tables, item modifiers, predicates, enchantments, jukebox songs,
instruments, damage types, advancements, and synchronized registry JSON.
Every condition must hold; a false condition omits the file. Mods can
[register conditions](load-conditions.md) for this same evaluator.
Register one handler per id using the read-only `RegistrationContext`. A handler
reads the condition's JSON from a borrowed raw value and validates its own
arguments. Invalid mod input must return `ConditionError`, never panic.

Ordinary JSON resources select the winning pack file first, then evaluate its
conditions before decoding the data. If the winner is omitted, a lower pack's
version does not return. Tag files and data-map files ignore a top-level
`neoforge:conditions` key. A data-map value with false conditions is omitted
before merging. In a loot table, only the pools
in the `pools` list carry their own `neoforge:conditions`: a false pool is
dropped and a passing pool loses the key before decoding. Anywhere else in a
recipe, predicate, item modifier, or a pool's entries and functions, a nested
key is ordinary data and is never evaluated, even when it names an unknown
condition. A damage
registry is frozen at world open; reloading reevaluates its tags, not its type
definitions. Generated item default components retain their prototype
lifetime and are not conditional data files.

`neoforge:mod_loaded` asks the owning resource generation's captured enabled
server-mod IDs, including active code-only mods, plus
`minecraft` always. Client-only, disabled and refused packages are absent;
resource namespaces are not mod IDs. Other supported forms are `neoforge:always`
(true), `neoforge:never` (false), `neoforge:not` (`value`), `neoforge:and` / `neoforge:or`
(`values`), `neoforge:feature_flags_enabled`, `neoforge:registered` and
`neoforge:tag_empty`. `feature_flags_enabled` takes `flags`, an array of feature-id
strings that must all be enabled. `registered` and `tag_empty` take a `registry` (default `minecraft:item`)
and a `value` or `tag`: `registered` reads the registries as this load holds them,
and `tag_empty` reads the block, item and fluid tags this load bound, so it is
answered for recipes and the readers after them, never for tag files.
Within a supplied view, a registry absent from it has no entries or tags: `registered` is false and `tag_empty` is true, with no diagnostic.

## Errors by loader

The loader resolves every type ID in a conditions list before testing any of
it, including all children of `not`, `and` and `or`. An unknown ID is a decode
error no matter its namespace, whether a mod of that name is installed, or
whether a sibling would decide the result.

| Loader | Unknown condition type or handler error |
| --- | --- |
| World and mod-declared data-pack registries that read conditions | Report the failing entries and refuse the world open. |
| Reloadable registries (loot tables and their pools, predicates, item modifiers, advancements, recipes) | Report the failing entries and keep the previous data generation. At world open, refuse the open. |
| Simple JSON reload listeners, including global loot modifiers | Log the condition ID and file path and skip only that file. |
| Data maps | Log the condition ID, file path, map type and registry. Skip the entire file, so its values, removals and `replace` flag have no effect. |

A file without the conditions array loads normally. Successful reloads publish
changed enabled IDs together with the data generation; retained readers keep
their old snapshot.
