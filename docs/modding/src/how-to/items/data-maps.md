# Reading data maps

A data map attaches a value to the entries of a registry (NeoForge's `DataMapType`): any registry
the game names, built in (`minecraft:item`, `minecraft:entity_type`, `minecraft:worldgen/biome`...),
a pack's or a datapack registry a mod declares. Data packs and mods' files set the values; a mod reads
what the server made of them, so a server pack can rebalance a mod's numbers without a rebuild.

Describe the map once with `DataMap`: its id, the registry its entries are in (`Entry`: an
`ItemKey`, `BlockKey`, `FluidKey`, `EffectKey`, `EntityTypeKey`, `EnchantmentKey`,
`BlockEntityTypeKey` or `ProfessionKey`), and how a value decodes. Values arrive as the NBT form of the JSON a
file held: a whole number is an `Int`, or a `Long` past `i32`, a fraction a `Double`, a boolean a
`Byte`, an object a `Compound` and an array a `List`. Integer tokens must fit signed
64-bit NBT `Long`; larger unsigned integers are never rounded to `Double`.
Fractional/exponent tokens use the JSON parser's `f64`
value (including an integral value written as `1.0`). A file containing an
out-of-range integer or a JSON `null` anywhere in a value cannot be converted to
NBT and is skipped atomically at load time with an error log, preserving earlier layers.
Empty and heterogeneous arrays preserve their elements as in-memory `List` values.

```rust,ignore
use mod_api::{DataMap, DataMapTypesEvent, ItemKey, ServerHost, Tag};

/// `mymod:emc`: an item's value.
struct Emc;
impl DataMap for Emc {
    type Entry = ItemKey;
    type Value = i64;
    fn id(&self) -> &str {
        "mymod:emc"
    }
    fn decode(&self, value: &Tag) -> Option<i64> {
        match value {
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
}

// In `impl Hooks`: register it each time the server loads its data.
fn data_map_types(&self, _: &mut dyn ServerHost, event: &mut DataMapTypesEvent<'_>) {
    event.types.push(Emc.ty());
}
```

Its values are `data/mymod/data_maps/item/emc.json` in any data pack, and files the mod pushes
through `data_pack_files` under `data_maps/mymod:item/emc`. The folder is the registry's path for
a `minecraft` registry (`entity_type`, `worldgen/biome`) and `<namespace>/<path>` for another.
An id the registry doesn't hold is skipped, with one warning per file. A layer must have a
`values` object. If present, `replace` must be a boolean and `remove` a list of removal keys.
A malformed layer is logged and skipped without changing earlier layers. A value may carry
its own load conditions (`WithConditions`), including on an unwrapped object: it counts only
when they hold, and the condition metadata isn't part of the value. A top-level
`neoforge:conditions` key on the data-map file is ignored.

```json
{ "values": {
    "minecraft:coal": 10,
    "#minecraft:logs": 32,
    "create:zinc_ingot": {
      "neoforge:conditions": [{ "type": "neoforge:mod_loaded", "modid": "create" }],
      "value": 64
    }
} }
```

Read an entry with its key's `data` (`item.data(host, &Emc)`, `entity_type.data(host, &map)`).
A typed read returns an owned decoded value. To read raw NBT by name, use
`data_map_value(host, "minecraft:worldgen/biome", "minecraft:plains", "mymod:heat")?`
for an `Option<&Tag>`, or `data_map_entries(host, registry, map)?` for a
`&[(String, Tag)]` with every entry/value pair sorted by entry ID (`IRegistryExtension.getDataMap`).
A bare entry ID such as `plains` uses the `minecraft` namespace.

The raw values borrow from the host's published data, not the lookup strings. Iterate over
bulk results by reference. Keep IDs or keys and read again with the next host, or make an owned
copy only when needed: `data_map_value(host, registry, entry, map)?.cloned()` for a tag, or
`data_map_entries(host, registry, map)?.to_vec()` for all the rows. A point read answers:

- `Ok(Some(value))`: the merged value. The newest file wins, a `#tag` gives every entry in it the
  value, `replace` drops what earlier files set and `remove` takes entries away.
- `Ok(None)`: the map gives this entry no value.
- `Err(DataMapError::UnknownType)`: neither the engine nor a mod registered the map for that registry in this load.
- `Err(DataMapError::Undecodable)`: a typed read's `decode` refused the entry's NBT value.
- `Err(DataMapError::Unavailable)`: no maps are loaded here. Data maps live on the server (the
  single player server thread included) and are not synced to clients, so a client returns
  this error, as does a server before its world's data first loads.

A known empty map yields an empty slice. As with a point read, an unknown map type or an
unavailable publication is an error.

The server merges every map when its data loads: a world's first open and every `/reload`. A
read sees the last load whole, never part of one, and a value a reload drops is gone at once.
`data_maps_updated` fires after each load. Use it to rebuild anything computed from the
values. Reads also work in world hooks. A lookup reads the loaded map: it parses nothing and
touches no file.


## NeoForge's built-in mob and item maps

These maps are already registered in the server's common store. Do not register them again.
Their files are `data/neoforge/data_maps/<registry path>/<name>.json`:

| Name | Registry path | Codec field |
| --- | --- | --- |
| `acceptable_villager_distances` | `entity_type` | `acceptable_villager_distance` |
| `parrot_imitations` | `entity_type` | `sound` |
| `villager_types` | `worldgen/biome` | `villager_type` |
| `raid_hero_gifts` | `villager_profession` | `loot_table` |
| `vibration_frequencies` | `game_event` | `frequency` |
| `villager_compostables` | `item` | `can_villager_compost` |

Every name has the `neoforge:` namespace. Values may be a scalar, a record with the codec
field above, or the usual conditional `{"value": ..., "replace": false}` wrapper.
Entry conditions run before decoding their values; top-level conditions are ignored.
The whole file is decoded before applying its replacement, values or removals: an invalid
value, even for an unknown target, refuses the whole file. Valid unknown targets and unknown
removals are skipped. Tags use the target registry, including profession and game-event tags.

Distances parse the original decimal directly to `f32`, including negative values,
signed zero and overflow to infinity (`1e500`). Frequencies truncate the exact decimal,
retain the low 32 bits as a signed integer, then require 1 through 15: `4294967297`
becomes 1, `4294967311` becomes 15, and `15.999999999999999999999` becomes 15.
Sound and villager-type values must name registered sounds and supported villager types;
loot tables are resource identifiers. Compostability is boolean.

Builtin reads return canonical NBT compounds with the field above, rather than the
input's shorthand: distances are `Float`, frequencies `Int`, compostability `Byte`, and
identifiers `String`. Infinity remains an NBT Float, never JSON null. Biome values
are queryable through `BiomeKey`; typed game-event coverage remains in #4199.
Registering these stores and firing `data_maps_updated` does not sync them to clients.

The gameplay readers and structure generation use immutable world-owned snapshots.
Villager types apply to finalizeSpawn only; bred villagers retain the existing biome-facts
rule rather than reading this map. Reload replaces the whole generation; a structure finalization already in progress may
finish with its old snapshot. Custom entity types are matched by their exact registered
ID, not by the vanilla species used as their behaviour template.

## NeoForge's built-in block maps

The game itself reads three of NeoForge's block maps (`NeoForgeDataMaps`), with no code in your
mod: `neoforge:flammable`, `neoforge:oxidizables` and `neoforge:waxables`, in
`data/neoforge/data_maps/block/<name>.json`. They load from the world's data packs and mods'
files on every server data load, merged as your own maps are, and each load replaces the last.

```json
{ "values": { "mymod:tin": { "next_oxidation_stage": "mymod:dull_tin" } } }
```

- `neoforge:flammable` (`ignite_odds`, `burn_odds`): fire asks a mod's fire hook first, then this
  map, then vanilla's table. Burn odds are clamped to 300, as a hook's answer is.
- `neoforge:oxidizables` (`next_oxidation_stage`): the stage a random tick weathers a block to, its
  properties kept. A mapped block weathers only where the game's weathering already ticks it
  (vanilla's copper blocks), so naming another block does not make it age by itself.
- `neoforge:waxables` (`waxed`): the twin honeycomb waxes a block to, for the player's honeycomb
  and the dispenser's, its properties kept.

The axe reads the inverse maps before its own transform table: the oxidation inverse scrapes one
stage back, the waxing inverse takes the wax off. They are derived from the forward maps on every
load, never declared. Where several blocks share a next stage or a waxed twin, the one with the
lowest block registry id is the one given back. That is a Sunrise choice: NeoForge keeps whichever
its map iteration met last.

Random-tick oxidation, honeycomb, dispenser waxing and the axe each move exactly one step, so a
chain that loops only alternates there. Lightning resets a struck block along the whole oxidation
inverse. So that mod data can never hang it, an oxidation chain that loops (a block that is its own
next stage included) is refused at load with an error naming `neoforge:oxidizables` and the
blocks, and those blocks get no scraping step; the lightning walk also stops where it would come
back to a block it already passed. Lightning's waxed check, the carved pumpkin and the copper chest
keep vanilla's own lists. A block the maps don't name keeps vanilla's behaviour.

Oxidation and waxing targets must be registered blocks. If `generate_block_transform` is present,
it must be a boolean. An invalid target or flag skips the whole file layer, even with `replace`.
Sunrise still derives its axe steps from the inverse maps, not from `generate_block_transform`.
`neoforge:transformables` and `block_transform_appenders` are not read.

`host.biome_key("minecraft:plains")` resolves a `BiomeKey` in the installed
server world registry. Its `id(host)` borrows the canonical name from the installed snapshot
(`Result<&str, BiomeKeyError>`): use `key.id(host)?` directly, or copy the result only to retain
it beyond the host borrow. The borrow does not affect the result of `data(host, &map)`: it still
returns an owned value from maps whose `DataMap::Entry` is `BiomeKey`, including
`neoforge:villager_types` in `minecraft:worldgen/biome`. Missing entries return
`Ok(None)`. Lookup returns `None` on clients and before world data is installed;
data maps there return `DataMapError::Unavailable`. Reacquire biome keys after
reload: tokens from earlier generations or another world return
`BiomeKeyError::WrongGeneration` from `id`, or `DataMapError::InvalidKey` from
`data`. An unavailable host returns `BiomeKeyError::Unavailable` from `id`.
Validation uses the receiving host’s installed snapshot, so a mod retaining a
key across reload receives an error without panicking the server.

Game events use `GameEventKey`, resolved with `host.game_event_key("minecraft:step")`
after startup registry completion. The key is load-owned and survives world data
reloads. `key.id(host)` returns a pinned [`RegistryName`](../../concepts/registry-keys.md#pinned-registry-names)
or `GameEventKeyError`; unknown slots return `InvalidKey` rather than panicking. Use
`let name = key.id(host)?;` and `name.as_str()` instead of copying it. A `DataMap` whose
`Entry` is `GameEventKey` reads `minecraft:game_event`, including
`neoforge:vibration_frequencies` (canonical NBT compound with an `Int` `frequency` field). Missing values return
`Ok(None)`; clients and reads before data installation return
`DataMapError::Unavailable`. Invalid keys return `DataMapError::InvalidKey`.
