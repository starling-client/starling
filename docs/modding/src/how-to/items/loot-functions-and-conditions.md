# Custom loot functions and conditions

Register a map codec once in the static, server-only `LOOT_FUNCTION_TYPE` or
`LOOT_CONDITION_TYPE` registry. These are the NeoForge 26.3 registries; their
entries are codecs, not separate legacy type wrappers:

```rust,ignore
fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
    r.register_loot_function_type(&Identifier::parse("example:scale").unwrap(), Scale);
    r.register_loot_condition_type(&Identifier::parse("example:enabled").unwrap(), Enabled);
}
```

Ids belong to the declaring manifest's namespace. Duplicates refuse registration,
naming both owners; `minecraft:` and `neoforge:` are reserved. A built-in always
resolves to the engine's built-in, never to a mod codec.

## Entry data and validation

The data form is an object with `type` and the codec's own fields side by side:

```json
{"type":"example:scale", "factor":3}
```

`ModLootFunction` and `ModLootCondition` implement
`validate(&LootEntryData) -> Result<(), String>`. Use `data.value()` to read the
immutable JSON tree. Unknown fields are retained for the codec, not rejected by
the engine. Validation runs at load, including tables edited by `loot_table_load`,
not when the entry happens to roll. Declare parameters with
`referenced_context_params`; a table whose context set does not allow one logs a
warning but still loads. At runtime an absent parameter remains absent.

Unknown loot types, malformed registered entries, failed codec validators,
predicate/item-modifier reference cycles and refused Wire validation calls fail
the candidate preparation, with a file/id, entry path and type in the diagnostic.
Refusing a candidate leaves previously retained snapshots unchanged. Malformed
winning pack JSON never falls back to a lower-priority file. Predicate and item
modifier references, including references selected by tags, validate at load.
Referenced tables retain the caller's context set during parameter validation;
recursive table references stop traversal rather than recursing indefinitely.
Built-in functions and conditions use the same preparation boundary. Declarative
codec shapes check their required fields, optional values, enums, numeric
providers, component payloads and nested entries. The same shapes identify the
owned child functions and conditions for reference validation and tag expansion;
the engine never searches through a registered codec's opaque payload.

Optional `null` fields behave like omitted fields, unknown fields remain ignored,
and supported compact forms (including singleton holder sets, references and
compound SNBT strings) stay accepted. Generic registry IDs are checked for syntax,
not arbitrary registry membership; codecs with a closed registry still enforce it.
Primitive integer fields accept numeric JSON tokens such as `2.0` and `1e1` and
use signed Java-style narrowing rather than defaulting those tokens to zero.
Extreme numeric literals can still lose precision in the JSON representation
([#2105](https://github.com/sunrise-org/sunrise/issues/2105)). Text shape checks do
not yet replace complete selector, coordinate and canonical text-size validation
([#2112](https://github.com/sunrise-org/sunrise/issues/2112)).

`LootEntryData` is API-owned and derives `Wire`. Its one private JSON-value field
uses the shared JSON tree encoding: length-prefixed UTF-8 JSON, with the shared
Wire depth and size checks. An unrepresentable entry refuses load.

## Applying and testing

A function's `apply` receives its entry, the current owned `LootStack` and a scoped
`&mut dyn LootTypeContext`, returning `Result<LootStack, String>`. A condition's
`test` receives its entry and that context, returning `Result<bool, String>`.
The result allows a refused resource call to be diagnosed, not mistaken for a
successful edit. A function's rejected output leaves its preceding stack intact;
a condition call failure is diagnosed and does not pass. Callback failures warn once
per type id in each retained data generation, not once per roll.

The context supplies optional facts through `snapshot()`,
authoritative tags through `tags()`, and `random()`. See [Reading tags](loot-tags.md).
Use only that random source: it is the evaluator's actual selected stream, not a
new generator seeded from a snapshot. Reading facts consumes no random; the
snapshot's `seed` field is zero. Codecs are shared immutable declarations and
must not keep random state between calls.

Functions run entry first, then pool, then table. A `sequence` passes each
function's output to the next. Conditions under `all_of` and `any_of` run in
order and short-circuit, so skipped terms consume no random. A function's
`conditional()` defaults to true: the engine tests its optional `condition`
first and returns the original stack if false. A direct function can return
false from `conditional()` and own that field itself.

`LootStack` carries an `ItemKey`, signed intermediate count, damage, enchantments,
potion, deferred map facts, names and ordered NBT component patches.
Each patch is `(String, Option<Tag>)`; `None` removes a component, and repeated
ids apply in order. Unknown NBT values retain their exact type. Names are API
`Text` trees. Empty and over-max intermediate counts are allowed; ordinary root
splitting happens after the table functions. Boundary conversion validates the
whole candidate before writeback. The items evaluator consumes stacks into its
ordinary item representation and has no structure-search capability.

Both evaluators carry optional entity, damage, player, tool, block, luck, biome
and place facts when their caller supplies them. `place` contains origin,
block-entity data and level facts (dimension, game time, time of day, rain and
thunder). Missing context facts are not fabricated by a table's declared set.

`queried_table_id` is the caller's first supplied id, or
`neoforge:unknown_loot_table` until the root and all nested tables finish.
The root then sets its id before global modifiers; nested tables never write it.
`queried_stack` is distinct from the tool and remains absent in existing callers.
Both facts cross Wire unchanged. Consequently `neoforge:loot_table_id` inside a
table normally matches the fallback rather than the root's name.

`neoforge:can_item_perform_ability` reads the tool and its plain, case-sensitive
`ability` string, such as `shears_dig`. Without a tool it is false. Unknown names
have no default ability. An item part's answer replaces the item-layer default.

Custom types are table-internal operations. Global loot modifiers, which operate
on the completed output list, and `loot_table_load`, which edits table data,
remain separate mechanisms. Use a custom type for logic not expressible by a
standard operation; `copy_components`, `copy_custom_data`, `copy_state` and
`match_block` are vanilla built-ins, not names for custom registration.

## Copying components

`minecraft:copy_components` reads `source` as `block_entity`, `tool`, or an
available entity target. The world evaluator accepts `this`, `attacker`,
`direct_attacker`, and `attacking_player`; the item evaluator accepts `this`,
`target_entity`, and `interacting_entity`. Each entity source is available only
when the caller supplied it.
The block source is its collected component map, including a mod block entity's
stored components, retained before removal; it is not restricted to particular
vanilla block classes. A tool includes its loaded prototype and live component
patch, with removals respected.

An optional `include` list selects component IDs; omitting it selects every
source component, while an empty list selects none. `exclude` is applied after
`include`, so exclusion always wins. Selected values replace their destination
components without removing unselected values. An absent source makes no edit.
This is the same operation in both loot evaluators.

Entity sources currently supply custom name when present and custom data
(including an empty compound when unset). Subtype components such as variants
and entity-type default components will arrive with the separate general entity
component-getter follow-up; the copy function does not manufacture those values.

## Copying custom data

`minecraft:copy_custom_data` reads saved NBT rather than a component map. Its
`source` may be an inline context target such as `"this"` or `"block_entity"`,
the equivalent `{"type":"minecraft:context","target":"this"}` object, or
`{"type":"minecraft:storage","source":"example:cache"}`. Context sources are
available only when the caller supplied that entity or block entity. An unset
storage entry is an empty compound, not an absent context source. Entity target
names have the same evaluator-specific availability as `copy_components` above.

Each entry in `ops` has a `source` NBT path, a `target` NBT path into the item's
custom-data compound, and an `op`:
- `replace` writes the last matching source value to the target path.
- `append` appends every source match to each matching list, creating missing
  lists. It does not append to byte, int or long arrays.
- `merge` recursively merges compound matches into each target compound,
  creating missing compounds and ignoring non-compound matches.

Operations run in order. Missing sources and unmatched source paths leave the
stack unchanged, and the custom-data component is written only if the result
changed. Other components are untouched. Literal NBT types, arrays and Java
UTF-16 payloads are retained. Both evaluators use the same path parser and merge
implementation. Like `set` and `insert`, append and merge refuse writes beyond
the existing 512-level NBT budget before creating any target parents.
