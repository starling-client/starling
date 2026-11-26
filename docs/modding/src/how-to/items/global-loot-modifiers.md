# Global loot modifiers

Global loot modifiers are registered serializer types with data-enabled instances, not per-roll
listeners. A type with no instance file never runs.

Register a `GlobalLootModifierSerializer` in `Hooks::register`:

```rust,ignore
fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
    registrar.register_global_loot_modifier_serializer("bonus", BonusSerializer);
}
```

A bare name uses the declaring mod's namespace while an explicit valid `namespace:path` keeps
its namespace. Duplicate ids, invalid ids and the built-in `neoforge:add_table` serializer refuse
registration.
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
a default priority or an absent condition. The runtime captures them once during decoding,
not during rolls. An inline condition uses the registered condition codec to bind its rule at the
same boundary. A named reference uses the bound predicate already in the owning loot generation.
An invalid condition refuses only that modifier file. The prepared instance and its code store
stay alive while a roll retains their generation, even after reload.

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

`GlobalLootModifier::apply` takes a borrowed `LootModifierContext` and a mutable `LootList<'_>`
borrowing the live generated list in place, returning `Result<(), LootModifierError>`. Like loot
functions and conditions, it is a data-only callback: it receives no `Host` or world-write access.
Its capabilities are borrowed parameters, the borrowed roll random, in-place list editing and raw
sub-rolls. Keep implementations `Send + Sync` and retain no borrowed context or list beyond the
callback.

`LootList` exposes `len`, `is_empty`, `get` (`Option<LootStackView<'_>>`), `get_mut`
(`Option<LootStackEditor<'_>>`), `iter`, `push`, `insert`, `replace`, `remove`, `pop`, `clear`,
`truncate`, `swap`, `reverse`, `reorder` and `retain`. The list keeps signed counts and arbitrary
component patches. A modifier may edit entries in place, add, remove, replace or reorder entries,
including empty and over-maximum stacks. Invalid pushed, inserted or replaced stacks return
`LootStackEditError` (which converts into `LootModifierError` with `?`) without modifying the
list; out-of-range indices return `Ok(false)` or `false`. No second splitter or count clamp runs
between modifiers or after the pass.

If application returns `Err`, the failure is reported and later modifiers continue with the list as
it stood when the error was returned (earlier valid edits and random draws are not rolled back),
and the modifier remains eligible on future rolls. A native panic disables the registering
serializer's callback owner through the standard retained-callback policy across its prepared
instances and reloads while other modifiers continue to run. Only when the Items caller
materializes its unsigned engine stack does a negative count become zero, retaining that list
position; subsequent modifiers still observe the original signed count.

`context.params()` lends the current parameters, including `queried_table_id()` for the
set-once root id. Optional parameters stay absent unless supplied. Reading them does not draw
randomness. `context.random()` borrows the actual selected roll source through `RandomSource`, the
existing shared draw capability; do not create a separate seeded generator.

Two raw sub-roll methods use the same parameters, selected random, visited-table guards and data
generation, running neither the modifier pass nor another stack splitter:

- `context.append_raw(&table_id, generated)` rolls the table raw and moves its unmodified engine
  stacks directly onto `generated` without copying them through API snapshots. The built-in
  `neoforge:add_table` modifier uses this same path.
- `context.roll_raw(&table_id, &mut |stack| ...)` rolls the table raw and lends each generated
  stack as a `LootStackView<'_>` to the visitor; call `stack.to_owned()?` only when keeping a copy.

For both methods, a missing table returns an error naming it and a visited table terminates with no
output.

```rust,ignore
use std::sync::Arc;
use mod_api::{
    DataLoadContext, GlobalLootModifier, GlobalLootModifierSerializer, Identifier, ItemKey,
    LootList, LootModifierContext, LootModifierError, LootModifierSettings, LootStack,
};

struct BonusSerializer {
    bonus_item: ItemKey,
}

struct BonusModifier {
    settings: LootModifierSettings,
    bonus_item: ItemKey,
    extra_table: Identifier,
}

impl GlobalLootModifierSerializer for BonusSerializer {
    fn decode(
        &self,
        data: &serde_json::Value,
        _load: &dyn DataLoadContext,
    ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
        let settings = LootModifierSettings::decode(data)?;
        let extra_table = data
            .get("extra_table")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LootModifierError::new("extra_table", "expected identifier string"))
            .and_then(|raw| {
                Identifier::parse(raw).map_err(|err| LootModifierError::new("extra_table", err))
            })?;
        Ok(Arc::new(BonusModifier {
            settings,
            bonus_item: self.bonus_item,
            extra_table,
        }))
    }
}

impl GlobalLootModifier for BonusModifier {
    fn priority(&self) -> Result<i32, LootModifierError> {
        Ok(self.settings.priority)
    }

    fn condition(&self) -> Result<Option<serde_json::Value>, LootModifierError> {
        Ok(self.settings.condition.clone())
    }

    fn apply(
        &self,
        context: &mut dyn LootModifierContext,
        generated: &mut LootList<'_>,
    ) -> Result<(), LootModifierError> {
        // Edit existing entries in place without cloning the whole list.
        if let Some(mut first) = generated.get_mut(0) {
            let count = first.view().count().saturating_add(1);
            first.set_count(count);
        }
        // Push an owned `LootStack` only when introducing a brand-new entry.
        generated.push(LootStack::new(self.bonus_item, 1))?;
        // Or append raw sub-rolls directly onto `generated` with zero snapshot copies.
        context.append_raw(&self.extra_table, generated)?;
        // Use `roll_raw` when inspecting sub-roll stacks before deciding what to keep.
        let mut kept = Vec::new();
        let mut capture_err = None;
        context.roll_raw(&self.extra_table, &mut |stack| {
            if capture_err.is_none() && stack.count() > 0 {
                match stack.to_owned() {
                    Ok(owned) => kept.push(owned),
                    Err(err) => capture_err = Some(err),
                }
            }
        })?;
        if let Some(err) = capture_err {
            return Err(err.into());
        }
        for stack in kept {
            generated.push(stack)?;
        }
        Ok(())
    }
}
```

The server's root pass is after table functions and stack splitting, before caller consumption and
container shuffle. Nested tables and raw sub-rolls do not run it. `loot_drops` and
`container_loot_filled` are separate later moments, not aliases for global loot modifiers.

## What parameters are available per caller

Each caller supplies vanilla's parameters for its roll, and only those; the rest stay missing.
`entity()` reads `THIS_ENTITY`, `tool()` reads `TOOL`, and `origin()` reads `ORIGIN`.

| Roll | `entity()` | `tool()` | `origin()` | `luck()` |
| --- | --- | --- | --- | --- |
| Piglin barter | the piglin | none | none | 0 |
| Advancement reward | the rewarded player | none | the player's position | 0 |
| Fishing | the fishing hook; its owner is the `attacker` entity target of conditions | the rod | the hook's position | the hook's and the owner's luck |
| Brushing | the brusher | the brush | the block's centre | the brusher's luck |
| Vault reward | the player | the inserted key | the vault's centre | the player's luck |
| Vault display item | none | none | the vault's centre | 0 |
| A mod's roll call | the entity it named, if any | the tool it gave | the origin it gave | 0 |

The fishing owner is not a damage source: `damage()` stays missing, and only a condition's `attacker` target reaches the owner. `level()` is present on all of them. A player's entity carries their worn and held items.
