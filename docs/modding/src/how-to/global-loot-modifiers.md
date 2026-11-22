# Global loot modifiers

Global loot modifiers are registered serializer types with data-enabled instances, not per-roll
listeners. A type with no instance file never runs.

Register a `GlobalLootModifierSerializer` in `Hooks::register`:

```rust,ignore
fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
    registrar.register_global_loot_modifier_serializer("bonus", BonusSerializer);
}
```

The resulting type id belongs to the mod's manifest namespace. Duplicate ids, invalid ids and ids
in another mod's namespace refuse registration. `minecraft:`, `sunrise:` and `neoforge:` are reserved.
The static serializer registry is `neoforge:global_loot_modifier_serializers`; it is not synced to clients.

## Preparing an instance

`GlobalLootModifierSerializer::decode` takes the instance's JSON and a borrowed `DataLoadContext`,
and returns `Result<Arc<dyn GlobalLootModifier>, LootModifierError>`. Validate custom fields there,
then retain typed immutable values. The load context exposes the owning registry/settings snapshot,
not a mutable game host. Report invalid fields with `LootModifierError::new(field_path, reason)`.

`LootModifierSettings::decode` parses the optional common fields:

- `priority`: a signed 32-bit integer, default `1000`; higher priorities run first, ties have no
  contractual order;
- `condition`: an inline loot condition or predicate id, absent for unconditional application.

A serializer can retain those settings and implement `GlobalLootModifier::priority` and `condition`
from them, returning `Ok(value)`. An error from either metadata method refuses the modifier instead of silently using
a default priority or an absent condition. The runtime captures them once during
decoding, not during rolls. The prepared
instance and its code store stay alive while a roll retains their generation, even after reload.

Instances are selected by files under `data/<namespace>/loot_modifiers/<path>.json`, for example:

```json
{
  "type": "example:bonus",
  "priority": 1100,
  "condition": {
    "type": "neoforge:loot_table_id",
    "loot_table_id": "minecraft:chests/simple_dungeon"
  }
}
```

## Loading and reloading

The server selects the highest enabled pack's file for each instance id before reading or decoding
it. A winning file with false `neoforge:conditions` removes the instance; it never falls back to a
lower file. Malformed JSON, unknown types, refused metadata and decoder errors log the instance id,
pack, file and reason, then skip only that file. Load-condition errors are isolated to that file.

Every server data reload prepares a complete immutable generation with its loot tables, predicates
and item evaluator. Only a successful preparation publishes it. A failed world preparation applies
no reload listeners and leaves the previous generation active. Retained references keep their old
instances alive; they do not switch silently to the new generation.

The built-in `neoforge:add_table` type requires `table`, a loot table id, and accepts the same common
`priority` and `condition` fields. It rolls that table raw with the current parameters, random and
visited guards, then appends its output unchanged:

```json
{
  "type": "neoforge:add_table",
  "table": "example:bonus",
  "condition": { "type": "neoforge:loot_table_id", "loot_table_id": "minecraft:blocks/stone" }
}
```

`GlobalLootModifierManager::key()` names the retained `neoforge:loot_modifiers` server reload listener.
From a server hook, read it with
`host.reload_resources().listener(&GlobalLootModifierManager::key())`. The manager's `modifiers()`
list is in application order, `get(&id)` finds an instance and `id_of(&instance)` finds its file id
by resource identity. It retains the exact generation that successful reload published. The anchor
follows the function library and precedes unrelated mod listeners by default; explicit dependencies
can order listeners against it.

## Applying an instance

`GlobalLootModifier::apply` takes a borrowed `LootModifierContext` and the preceding list as
`Vec<LootStack>`. It returns the replacement list or a typed error. Like loot functions and
conditions, it is a data-only callback: it receives no `Host` or world-write access. Its capabilities
are the facts snapshot, the borrowed roll random and raw sub-rolls. Keep implementations
`Send + Sync` and retain no borrowed context beyond the callback.

The list keeps signed counts and arbitrary component patches. A modifier may add, remove, replace
or reorder entries, including empty and over-maximum stacks. No second splitter or count clamp runs
between modifiers or after the pass. If application fails, later modifiers receive the preceding
list; draws already made from the roll random are not undone. A native panic disables the failing
prepared instance through the standard retained-callback policy. Only when the Items caller
materializes its unsigned engine stack does a negative count become zero, retaining that list
position; subsequent modifiers still observe the original signed count.

`context.snapshot()` reads the current facts and the set-once queried root table id. Missing facts
stay missing. Its legacy `seed` field is zero: obtaining a snapshot does not draw a new seed.
`context.random()` borrows the actual selected roll source through `RandomSource`, the existing
shared draw capability; do not create a separate seeded generator. `context.roll_raw(&table_id)`
rolls into an appendable list using the same parameters, selected random, visited-table guards and
data generation. It runs neither the modifier pass nor another stack splitter. A missing table
returns an error naming it; a visited table terminates with an empty list.

The server's root pass is after table functions and stack splitting, before caller consumption and
container shuffle. Nested tables and raw sub-rolls do not run it. `loot_drops` and
`container_loot_filled` are separate later moments, not aliases for global loot modifiers.

## What a snapshot holds per caller

Each caller supplies vanilla's parameters for its roll, and only those; the rest stay missing.
`entity` is `THIS_ENTITY`, `tool` is `TOOL`, and `place.origin` is `ORIGIN`.

| Roll | `entity` | `tool` | `place.origin` | `luck` |
| --- | --- | --- | --- | --- |
| Piglin barter | the piglin | none | none | 0 |
| Advancement reward | the rewarded player | none | the player's position | 0 |
| Fishing | the fishing hook; its owner is the `attacker` entity target of conditions | the rod | the hook's position | the hook's and the owner's luck |
| Brushing | the brusher | the brush | the block's centre | the brusher's luck |
| Vault reward | the player | the inserted key | the vault's centre | the player's luck |
| Vault display item | none | none | the vault's centre | 0 |
| A mod's roll call | the entity it named, if any | the tool it gave | the origin it gave | 0 |

The fishing owner is not a damage source: `damage` stays missing, and only a condition's `attacker` target reaches the owner. `place.level` is present on all of them. A player's entity carries their worn and held items.
