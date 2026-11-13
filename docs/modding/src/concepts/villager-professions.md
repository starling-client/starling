# Design: villager professions and trades

Provisional proposal for [#1328](https://github.com/arismoko/horizon/issues/1328)
and parent [#1043](https://github.com/arismoko/horizon/issues/1043), one workstream.
Root approval of the typed public API and save/network gates is required before
code. This page implements nothing and does not complete either issue.

## Answer and references

Register professions at mod load through one load-owned typed registry. A resolved
profession joins workstation/POI selection, level-to-trade-set keys, texture and
hat selection, saved resource identity and its network projection. Trade JSON
alone cannot register a profession. Use #2339's general resolved-table contract;
do not introduce a second generation counter or an independent global registry.

Merged prerequisites #1333, #1360 and #1398 provide vanilla metadata, unresolved
names and the frozen vanilla table. Parent prerequisites #1341, #1348, #1352,
#1374 and #1379 cover POI, namespaced rendering and unresolved trading routes.
#2635 adds secondary work-block tags, not custom profession registration.
#2838 overlaps profession sounds/rendering; #2697 overlaps villager hook vocabulary.
Coordinate those implementations rather than copying their declarations here.

Vanilla 26.3 reference: inspect `VillagerProfession` using
`python3 tools/rea_vanilla.py class net.minecraft.world.entity.npc.villager.VillagerProfession`
and `method` for its constructor, bootstrap, static initializer, accessors and
all five `register` overloads (select with `--overload-index`). Its record holds
name, held/acquirable POI predicates, requested items, secondary blocks, optional
work sound and an integer-level-to-TradeSet-key map. `getTrades` looks up that map.
The complete bootstrap declares:

| Profession | Held/acquirable POI | Trade levels | Requested items / secondary blocks |
|---|---|---|---|
| none | PoiType.NONE / ALL_ACQUIRABLE_JOBS | none | empty / empty |
| armorer | ARMORER / ARMORER | 1–5 | empty / empty |
| butcher | BUTCHER / BUTCHER | 1–5 | empty / empty |
| cartographer | CARTOGRAPHER / CARTOGRAPHER | 1–5 | empty / empty |
| cleric | CLERIC / CLERIC | 1–5 | empty / empty |
| farmer | FARMER / FARMER | 1–5 | wheat, wheat seeds, beetroot seeds, bone meal / farmland |
| fisherman | FISHERMAN / FISHERMAN | 1–5 | empty / empty |
| fletcher | FLETCHER / FLETCHER | 1–5 | empty / empty |
| leatherworker | LEATHERWORKER / LEATHERWORKER | 1–5 | empty / empty |
| librarian | LIBRARIAN / LIBRARIAN | 1–5 | empty / empty |
| mason | MASON / MASON | 1–5 | empty / empty |
| nitwit | PoiType.NONE / PoiType.NONE | none | empty / empty |
| shepherd | SHEPHERD / SHEPHERD | 1–5 | empty / empty |
| toolsmith | TOOLSMITH / TOOLSMITH | 1–5 | empty / empty |
| weaponsmith | WEAPONSMITH / WEAPONSMITH | 1–5 | empty / empty |

Each employed profession uses its corresponding work sound and profession-specific
trade-set constants for all five levels; none and nitwit have null sounds.
Textures and hats are separate rendering resources, not fields in this record.
REA returned no body for the static initializer and synthetic lambda methods,
even with raw evidence inspection. Their exact predicate/tag initialization is
unverified; the table records the predicates passed by the complete bootstrap,
not independently verified membership of ALL_ACQUIRABLE_JOBS.

NeoForge's public [26.1 migration primer](https://docs.neoforged.net/primer/docs/26.1/)
documents the move to separate `villager_trade` and `trade_set` datapack registries
and a profession's level map. The [26.3 primer](https://docs.neoforged.net/primer/docs/26.3/)
describes changed trade provider types. Neither establishes the current event
contract. The attempted public 26.3 profession documentation at
<https://docs.neoforged.net/docs/26.3/misc/villager/> was inaccessible.
`VillagerTradesEvent` is the issue's named counterpart; its 26.3 existence,
side, mutation ordering, limits and full callback contract remain **unverified**.
Require public 26.3 documentation/Javadoc before adopting an event contract.
No NeoForge source or patches are a reference for this proposal.

## Current owners and routes

Anchors below were checked against this worktree; they describe current owners,
not implemented custom registration. Paths are repository-relative.

| Route | Current owner and required change |
|---|---|
| Vanilla seed and identity | `crates/horizon-client/src/mobs/villager.rs:32`, `:113`, `:124`: immutable vanilla-only table and unresolved state. Replace context-free access with the owning load's resolved table; retain names. |
| POI classification | `crates/horizon-world/src/poi.rs:12`: vanilla enum. Custom workstation states need typed POI resolution, tickets and classification before admission; extending a vanilla POI alone does not create a custom profession. |
| Job validation/acquisition | `crates/horizon-client/src/mobs/villager_brain.rs:895`, `:898`, `:2131`, `:2169`: held/acquirable filters and assignment routes use the same resolved reverse index. |
| Secondary sites | `crates/horizon-client/src/mobs/villager_secondary.rs:4`: profession-indexed tags and farmer defaults. Preserve #2635's world tag snapshot and sensor cadence; resolve custom membership once. |
| Profession change and auxiliary metadata | `crates/horizon-client/src/mobs/villager.rs:471`, `:481`, `:563`: offer reset, work sound and requested items must read resolved metadata. Do not grant farmer-specific farming AI merely by declaring requested items. |
| Trade loading/selection | `crates/horizon-client/src/merchant.rs:335`, `:364`, `:376`, `:469`: Trades owns data and overlays separately; replace minecraft-prefixed set construction with explicit level keys. |
| Merchant interaction | `crates/horizon-client/src/game/items/trading.rs:220`, `:266`, `:348`, `:361`, `:431`: creation, opening, refresh, direct opening and completion need resolved identity or unresolved refusal. |
| Mod merchant handles | `crates/horizon-client/src/game/modding/native_merchant_refs.rs:69`, `:100`, `:217`: preserve unresolved guards and identity reads; resolve typed writes through the same registry. |
| Save/load and zombie villagers | `crates/horizon-client/src/entity_nbt.rs:1118`, `:1125`, `:3982`, `:4111`: both species preserve the saved profession name and existing offers. |
| Render layers and hats | `crates/horizon-client/src/mobs/render.rs:215`, `:952`, `:2259`; `crates/horizon-client/src/mobs/villager_hats.rs:18`, `:51`: resolve actual texture resources and cached metadata, including zombie layers. |
| Client receive | `crates/horizon-client/src/game/network/remote_mob_data.rs:471`: currently strips minecraft namespace and ignores custom names. Replace with snapshot-scoped key resolution. |
| Server metadata | `crates/horizon-client/src/game/network/lan/mod.rs:1020`: both villager species currently coerce unresolved professions to index 0. Send the qualified identity through the peer mapping instead. |
| Protocol and tracking | `crates/horizon-protocol/src/entity_data.rs:437` encodes VillagerData integers; `crates/horizon-client/src/lan/entity.rs:408` pairs entities and `:474` sends changes. Both require the peer's activated registry mapping before metadata. |
| World preparation and reload | `crates/horizon-client/src/game.rs:11722`; `crates/horizon-client/src/game/administration/functions.rs:260`, `:1319`: initial load and reload routes must prepare dependent tables together. Keep registry logic in sibling modules; game.rs must not grow. |
| Shared infrastructure | `crates/horizon-mod-runtime/src/registry_keys.rs:6`, `:18`: requests/names are lookup machinery, not a new lifecycle holder. `crates/horizon-world/src/native_load_context.rs:81` carries load inputs, not mutable world authority. |

## Proposed public contract

Proposed names, pending root approval: `ProfessionKey`, `PoiKey`, `TradeSetKey`
and `ProfessionDefinition`, owned by the public mod API. A definition contains
its namespaced key and translation component; workstation block-state references;
held and acquirable POI key sets; an explicit map for levels 1–5; requested item
keys; secondary block/tag references; optional work sound; and villager/zombie
texture resource identifiers with hat policy. Empty trade maps are legal. A
missing level means no new offers at that level, never a fabricated vanilla key.
Trade-set identity is independent of profession identity: `test:engineer` may
select `test:engineer` at level 1 without suffix or namespace rewriting.

Collect dependency-ordered declarations during load, before world admission,
with a load-scoped registrar and shared Host services. World mutation is
unavailable at this phase. No per-tick registration callback is proposed.
Generate native and wasm entry points and API-owned `Wire` records together;
never expose Rust pointers, closures, serde JSON values or guest addresses as
public transport. Refuse `UnsupportedProfessionCapability` for arbitrary POI
predicates, executable trade factories or unsupported render policies rather
than accepting inert fields. Do not add a speculative event beside #2697's hook.

Resolve references against the complete candidate graph, including blocks, POIs,
items, tags, sounds and trade sets. Build POI-to-profession and texture indexes
before publication. Recommend refusing overlapping acquirable POIs between
custom declarations until an explicit selection policy is approved. Held and
acquirable sets may differ. Duplicate profession keys refuse deterministically
with `DuplicateProfession`, key and both owners, independent of dispatch order.
Recommend no replacement of vanilla profession declarations in stage one;
`ReservedProfession` refuses it. Normal pack priority for trade resources remains
owned by the existing loader, not a competing profession override mechanism.

The current world epoch belongs to `NativeLifecycle` in
`crates/horizon-client/src/game/modding/native_lifecycle.rs:65` and its `epoch`
accessor at line 83. Use that existing load/world lifecycle identity and #2339's table holder for all
resolved keys. Session/epoch validation and stale-key refusal follow that owner;
no profession-local generation. Publish profession, POI, trade and client
projection tables atomically. Failed resolution publishes zero entries and
retains the old active set on reload. Registration is fixed for the mod load;
reload re-resolves data and tags without rerunning mod startup. Runtime mod
unload requires closing its worlds. In lifted passes, queue retirement and journal
emission until root commit; rollback emits neither. Refuse saves during an open
action savepoint; never persist a candidate registry or provisional offer reset.

## Saves, network and resources

Persist fully qualified names in VillagerData, plus existing serialized offers,
experience and restock state. On restart resolve names against the new owning
load. Missing mods retain `Unresolved(name)` byte-for-byte, disable job assignment,
profession reset and trade mutation, and preserve offers for later restoration.
Do not map an unresolved name through u8 wrapping, farmer or none. Render a neutral
base layer without discarding identity. Restoring the mod resolves the same name;
a different registry order must not change profession or trade-set identity.

Server authority applies to integrated, LAN and dedicated play equally. Negotiate
custom registry support before entity pairing through the #813/#2882
negotiation layer. Until that layer lands, refuse a client without the required
mod at admission with `MissingRequiredProfessionMod(mod_id)`, naming the missing
mod and reason; do not send custom metadata or silently substitute vanilla. API-owned Wire projections carry
qualified profession/POI/trade keys, translation data, texture identifiers and hat
policy under the existing lifecycle identity. Reuse the common registry snapshot
transport: peers validate a complete mapping and acknowledge activation before
receiving its numeric references. VillagerData integers are scoped to that mapping,
never stable save identities or process-local slots. Invalid/out-of-range ids
refuse the packet/session; never leave an old farmer value silently in place.
An unsupported peer is refused at admission when custom professions are present;
vanilla-only sessions retain their existing protocol. Exact configuration and
snapshot packet integration is a protocol review gate, not implemented here.

The server sends resource identifiers, not arbitrary paths or texture bytes.
For `test:textures/entity/villager/engineer.png`, resolve that exact resource and
its metadata; current convention-only hat scanning must be extended to declared
textures. Resolve the zombie texture separately. Cache None/Partial/Full hat
metadata with the effective pack resources; default missing metadata to None and
preserve vanilla type/profession head-visibility composition. Client resource
reload refreshes this cache without replacing server profession identity. Missing
optional textures use the missing-resource visual with a diagnostic, never a
vanilla profession substitution. Required client content is checked at admission.

## Bounds, failure and cost

Treat all mod declarations and network projections as untrusted. Proposed Horizon
limits (not NeoForge limits): 65,536 total professions including all 15 vanilla
entries; identifier length unresolved pending owning vanilla codec evidence;
1 MiB per declaration; 64 MiB
aggregate candidate definitions; nesting depth 64; 4,096 references per set; at
most five level mappings with unique levels 1–5. Reject traversal, malformed
identifiers, missing required references, cycles in trade-set references and
unsupported tags/providers before publication. Reuse bounded trade decoding;
counts, costs, XP and provider results must be finite and fit their engine target
ranges, including item stack limits. Never truncate an oversized input. These
budgets preserve vanilla's profession and five-level capacity; the implementation
must check the complete vanilla trade dataset fits before fixing trade budgets.
Identifier admission is gated on evidence of the owning vanilla codec limit or
an approved bounded-transfer route preserving its capacity. A 256-byte cap is
not approved; dataset fit alone does not establish vanilla capacity.

Native unwind or wasm trap aborts candidate preparation with owner/key context;
apply the runtime's failing-mod policy. Process-aborting native panics cannot be
recovered. No callback executes while partially installing live tables. Future
trade callbacks need a separate approved boundary and budget; they are refused
by this declaration contract.

Resolve names and reverse indexes at load. Hot job, trade and render reads use
validated indexed keys, with O(1) unused-path checks and no scans over mods,
per-villager string construction or registration dispatch. Empty custom registries
add no listeners or custom synchronization payloads. Existing secondary sensor
spatial work keeps its cadence and skips empty membership. Hat application must
use cached resource keys rather than its current formatting lookup.

## Acceptance matrix

These are required future tests, on native and wasm, not results of this docs task.

| Input/route | Expected result |
|---|---|
| Declare test:engineer; workstation test:assembler; level-1 set test:engineer; texture test:textures/entity/villager/engineer.png | Exactly 1 resolved profession; all four relationships preserved on both targets. |
| Save test:engineer and restart with its mod | Same qualified name, level map and saved offers; no reroll solely from load. |
| Restart without the mod, save again, then restore it | Name retained exactly, 0 vanilla numeric assignments, 0 trade mutations while unresolved; restored identity resolves correctly. |
| Two owners declare test:engineer | DuplicateProfession names both owners; 0 partially published generations. |
| Guest joins or receives changed metadata with reversed registry order | Same test:engineer identity and texture; 0 substitutions of minecraft:farmer. |
| Vanilla minecraft:farmer, no custom declarations | Same composter selection and five trade-level keys; 0 custom callbacks/payloads. |
| Mod attempts minecraft:farmer declaration | ReservedProfession; vanilla unchanged. |
| Missing POI/set, duplicate level, cycle, 65,537 entries, depth 65, trap | Candidate refused; old table identity and trades unchanged. |
| Engineer texture hat Full; type hat None | Type-layer head hidden; exact declared texture used for profession layer. |
| Unsupported peer with custom content | Refused before pairing; 0 undecodable entity metadata. |
| Reload through each authoritative route above; failed candidate | One common atomic publication on success; 0 on failure, no mixed POI/trade tables. |
| Rollback within lifted pass; save during open savepoint | 0 retirement/journal effects before root commit; save refused. |

## Proposed stages

1. Root approves keys, declaration/replacement policy, budgets and save/network
   contract after #2339 alignment and public NeoForge contract verification.
2. Implement generated native/wasm registrar and typed POI resolution together;
   validate duplicates, references and unsupported capabilities before publication.
   This server half may land as its own change, but main never accepts a
   profession registration that no client can show: until the client half in
   stage 4 lands, registration stays refused with a named reason, or both halves
   land in the same rollup.
3. Route all owners above through the load-owned table, including merchant handles,
   zombie saves and resource caches; preserve unresolved behavior.
4. Implement negotiated snapshot activation and all metadata send/receive routes.
5. Run the matrix with vanilla parity, adversarial inputs and unused-path cost
   measurements. No registrar code is authorized by this provisional page.

## Open questions

- Exact public registration shape and counterpart? Recommend declarative startup
  registration; verify 26.3 VillagerTradesEvent before choosing any event name,
  side or semantics. Root approves the public API before code.
- Custom POI sharing or vanilla replacement? Recommend refusing both initially;
  permit differing held/acquirable sets but require unambiguous acquisition.
- Network admission and activation? Recommend required custom-content capability
  and shared snapshot acknowledgment before pairing; protocol owners must approve
  packet placement and transfer/reconfiguration behavior.
- Final resource/trade budgets? Recommend the limits above after complete vanilla
  dataset measurement, with existing decoder limits retained wherever stricter
  without reducing vanilla capacity. No NeoForge limit is claimed verified.
