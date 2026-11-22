# Recipe priorities

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
| `neoforge:conditions` | an array of [load conditions](load-conditions.md); a valid false condition skips the entire layer |

A recipe with no entry has priority 0.

## Layers

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

## Order

The recipes are sorted once per server data load, by descending priority and then
by vanilla's registration order (path, then namespace). Crafting, every cooking
type, smithing, the stonecutter list and the recipes of mod-defined types follow
that order; nothing sorts or looks a priority up while recipes match. With no
file, the order is vanilla's.

A client joined to a server lists its stonecutter recipes in the order the
server sent them, and its buttons number those entries, whatever packs the client
has itself.
