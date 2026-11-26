# Villager professions and trades

Declare custom professions once per mod load through
`Hooks::register_villager_professions`. The callback receives a
`RegistrationContext` and a `RegisterVillagerProfessionsEvent`; call
`event.professions.register(definition)` on its `VillagerProfessions` registrar.
The engine records the declaring mod as owner and publishes only [successful listeners’ declarations](../../concepts/load-phases.md#registration-and-setup).
World mutation is unavailable
at registration, and datapack reload resolves the declarations again without
rerunning the callback.

## Definitions and resources

`ProfessionDefinition` contains:

- A `ProfessionKey`, wrapping a qualified `Identifier` such as `example:engineer`.
- `workstations`: `ProfessionWorkstation` selectors, each with a `PoiKey`, block
  identifier and required block-state properties. Omitted properties match all
  values.
- `held_poi` and `acquirable_poi`: `PoiKey` lists for retaining and acquiring a job.
- `requested_items`, `secondary_blocks` and an optional `work_sound`, using
  resource identifiers. Requested items do not grant farmer AI.
- `trades`: explicit level-to-`TradeSetKey` pairs for levels 1–5. A missing level
  adds no offers; an empty map is legal.

`PoiKey` and `TradeSetKey` also wrap `Identifier`. A trade-set key is independent
of the profession key: no namespace or level suffix is inferred. Provide trade
sets and offers through the `trade_set` and `villager_trade` datapack
registries; trade JSON alone does not register a profession.

## Registration refusals

References resolve against the complete candidate tables before publication.
`ProfessionRegistrationError` describes refusals:

| Variant | Reason |
|---|---|
| `DuplicateProfession` | Multiple declarations use one profession key; owners are reported in sorted order. |
| `ReservedProfession` | A profession key uses the reserved `minecraft:` namespace. |
| `OverlappingPoi` | Two professions acquire jobs from the same custom POI. |
| `UnacquirablePoi` | A declared custom workstation has no acquiring profession. |
| `InvalidWorkstation` | A workstation selector is invalid or conflicts with another POI. |
| `UnknownPoi`, `UnknownTradeSet`, `UnknownItem` | A referenced POI, trade set or item is missing. |
| `UnknownBlock`, `UnknownSound` | A secondary block or work sound is missing. |
| `InvalidTradeLevel` | A level is outside 1–5 or appears more than once. |

Custom workstation POI identities must also stay outside `minecraft:`. Missing
workstation blocks and selectors matching no valid states refuse preparation.
A failed candidate publishes nothing; a failed reload keeps the active tables.

## Identity, ordering and multiplayer

Profession and custom POI table order depends only on registered qualified IDs,
not mod load order or declaring owner. Vanilla's 15 professions retain the first
15 profession slots; custom professions follow in sorted order. Numeric slots
are runtime lookups, not save identities. Saves retain qualified profession and
POI names. A missing profession remains unresolved without discarding its name
or existing offers, so restoring the mod can resolve it again.

Clients build the same deterministic identity table from their installed mods and
use it for villager rendering. Install the profession mod on both the server and
its clients, with the mod declared for both sides. The mod-list handshake refuses
missing or mismatched both-side mods; client-only mods do not take part in that
comparison. Missing professions are not replaced with vanilla
identities; an unresolved server identity projects to `none` on the network without
changing the saved identity.

## Profession textures and hat metadata

Profession layers use the registered profession's qualified identity. For
`test:engineer`, provide
`assets/test/textures/entity/villager/profession/engineer.png` and
`assets/test/textures/entity/zombie_villager/profession/engineer.png`.
Nested identity paths remain nested texture paths. Vanilla professions use the
same resource lookup.

The texture's `.png.mcmeta` supplies its hat policy:

```json
{"villager":{"hat":"full"}}
```

`none` keeps the type layer's head visible, `full` hides it, and `partial`
hides it only when the type texture has a full hat. Missing metadata means
`none`. Pack reload refreshes the cached metadata. Unresolved saved professions
retain their names and render the neutral base without a profession layer.

## Secondary work blocks

Add blocks to a vanilla profession's secondary work sites with the block tag
`sunrise:villager_secondary_poi/<profession>`, one of the documented
[extension tags](../extension-tags.md) that every package kind may add to. For example,
`data/sunrise/tags/block/villager_secondary_poi/farmer.json`:

```json
{"values": ["minecraft:soul_sand"]}
```

Entries add to the profession's defaults. Farmers retain farmland, including
blocks using the farmland template. Other vanilla professions can use their own
names, such as `fisherman`. For a custom profession, declare
`ProfessionDefinition::secondary_blocks` through `register_villager_professions`.
These blocks participate in the same secondary-site scan.

Tags use the normal world-open and `/reload` tag resolution. Successful reloads
replace membership; failed reloads retain the previous world data. Removing all
entries restores vanilla behavior.

The server sensor scans inclusively within four blocks
in x and z and two in y. Professions with empty secondary sets skip the scan.
Results populate secondary job-site brain memory. The shared work activity can stroll toward a randomly selected secondary
site while near its primary job site. Farming requires a farmer and valid crops and farmland.
