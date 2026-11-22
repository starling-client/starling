# Design: general entity types

> The native contract below still applies. WebAssembly mods were removed; the parts about guests are history.
> This applies to the implemented general-entity registry, tracking and renderer handoff, not the later proposals.
> See [entity rendering](../../modding/src/how-to/client/entity-renderers.md) for the current native API.

Provisional answer to [#792](https://github.com/arismoko/horizon/issues/792): register a type once, store each instance in a game-owned general entity row, and dispatch its catalog-generated EntityPart behavior through a resolved type slot. The row owns identity, residency, persistence and tracking. It must never create a throwable or other template entity. This is a proposal for Ari's decision on the issue, not implemented API or a closure claim.

## Implemented stage 2 API

`Hooks::register` receives `&mut dyn Registrar`, whose `GeneralTypesRegistrar`
supertrait provides `declare_general_entity(path, category, width, height,
passenger_attachment, save_policy, summonable, pickable, piston_push_reaction,
tracking_chunks, update_interval, payload_version, required_features)`. Use `GeneralEntityCategory::Misc` and
`GeneralSavePolicy::{Save, NoSave}`. The path must belong to the declaring
mod's namespace. Registration is only available during the registration phase,
requires a both-side package, and refuses duplicates and unavailable required
mods with `GeneralEntityRegistrationError` while recording a refusal.

Dimensions and passenger attachment coordinates must be finite. A passenger
attachment is a vehicle-local point in blocks, rotated with the vehicle's yaw.
Use `None` for `[0.0, height, 0.0]`, the top center of the box. `pickable` opts the
type's declared box into the player's crosshair pick once the row is shown. It does not
add collision or a middle-click result. Tracking distance and interval use signed `i32`
values directly, matching the vanilla builder; `i32::MAX` means no update
interval. There is no 64-block, 32-chunk or 1200-tick ceiling.

The engine freezes declarations in mod-id/type-id order into non-narrowed
catalog indices. The record validator preserves the general-record policies
of 65,536 payload bytes and decoded payload nesting 32. A passenger chain has no bound of its own, as in vanilla: only the saved NBT nesting limit applies.
A record of a namespace no loaded mod owns is quarantined: kept byte for
byte as it loaded, never ticked or visible to the mod API, and written back
unchanged on save, up to 69,632 encoded bytes per record and 256 records
per chunk — past either bound the record skips with a warning naming the
type and position, as vanilla's own unknown kinds warn and drop. A loaded
mod's namespace with a type it never declared skips with a warning;
reinstalling the owning mod restores a quarantined record through the
normal load path.

Stage 4 ships the authoritative tick for native mods: the
`general_entity_tick` entity-scope moment on `EntityPart` (the scope's
first; its goal, behavior and spawn-size registrars are plain methods, not
hooks), dispatched from one join-order pass per level tick — each root's
bookkeeping, its subscribed behavior and the global tick pair, then its
riders in the maintainers' flat order (vehicle Pre, tick, Post, then each
passenger's own triple, then that passenger's passengers, depth first),
`Batch::General` keeping the mobile kinds' join order against the rows —
with a per-pass resolved slot table and a first-child rider index reused
across passes, the `ordered.born` cutoff holding this tick's joins for the
next, parked levels untouched, liveness re-checked at every step (a
listener's removal or dismount takes effect mid-pass), and a transaction
plus catch around each row's dispatch so a native unwind rolls the row's
writes back. The moment fires only for general rows: vanilla families tick
their own paths, and bringing them to one walk is #6509's, extendable
later if Ari wants it wider.

**Historical guest route (stage 4b):** the host called back into a guest Part,
each row kept its own attachments, and a trap rolled back the row's writes and
stopped the dependency group. Guest overrides of `register_goals`,
`register_behaviors`, `spawn_cluster_size`, `is_shearable` or `on_sheared`
were refused by name at load. That restriction does not apply to native Parts.

The remaining sections describe the wider design, not shipped lifecycle support.

## Evidence and limits

Vanilla 26.3 was queried from `/workspace/sunrise` with `python3 tools/rea_vanilla.py`: `method net.minecraft.server.level.ServerLevel tickNonPassenger` runs commonTick, tick, then passenger ticking. `--overload-index 0 method net.minecraft.world.entity.EntityType loadEntityRecursive` delegates compound input through registry-aware TagValueInput and a problem reporter; this wrapper alone does not establish unknown-type recovery. `callers net.minecraft.server.level.ServerLevel addFreshEntity` reports 36 callers, including dispenser vehicles, loot drops, TNT, hatching and dragon creation. Creation is consequently wider than summon. Exact recursive-load internals, passenger tick semantics and global vanilla iteration ordering remain unverified; do not infer them from these queries.

The issue names NeoForge 26.3 RegisterEvent and Registries.ENTITY_TYPE. The permitted [entity guide](https://docs.neoforged.net/docs/entities/) describes registered singleton types, instance behavior, separate renderers, server insertion and configurable tracking. It is explicitly labelled **26.1**, not 26.3. The [26.3 RegisterEvent Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/registries/RegisterEvent.html) was inaccessible through the web reader. Exact NeoForge 26.3 registration, lifecycle ordering, persistence and tracking contracts are **unverified**. No NeoForge source, patches or other project's implementation was read. The following is an explicit Sunrise proposal, not asserted NeoForge equivalence.

## Registration and ownership

Freeze a namespaced entity-type registry before worlds or connections open. A declaration supplies category, finite dimensions, save/summon policy, signed integer tracking distance and update interval, codec schema and required client features. Reject duplicate keys, unresolved required dependencies and invalid limits at registration. Resolve registry keys to compact slots, behavior subscribers and codecs once. Content declarations and code declarations use one registry, with no reserved throwable fallback or compatibility alias.

Use one dense general-entity store per level, with constant-time identity-to-slot lookup and generation validation; removal may swap rows while updating the index. A row contains type slot, UUID, runtime network id, incarnation/join identity, dimension, pose, velocity, lifecycle flags and attachment holder. Never persist runtime slots, network ids or pointers. EntityRef validates world epoch, dimension and incarnation before exposing a row. Reusing id 42 cannot make an old handle address its replacement. UUID continuity on reload does not revive an old live handle.

EntityPart is shared behavior, not mutable per-instance storage. Follow [PR #2328](https://github.com/arismoko/horizon/pull/2328), the provisional #796 state design: existing attachments are the sole Part-data store, with typed authoring erased at registration, snapshot reads and checked transactional edits. Do not introduce a parallel entity data map. #779 supplies entity scope and scoped moments; it does not by itself supply entity ownership. Generate new methods only for named verified moments, with ServerHost or ClientHost and the existing dependency dispatch order. No Game references escape.

Content entity types and general entity types share one key space. A package that declares the same
key through both routes is refused when its content loads.

## Tick and residency

At the authoritative entity phase, walk eligible general rows once in stable join order, with an indexed iteration schedule maintained on mutation rather than sorting each tick. For each root run common bookkeeping, its subscribed server behavior, then its passenger subtree. A passenger is not also ticked as a root. Newly committed spawns first tick next phase; a row removed before its turn receives no tick. Buffer structural edits until safe boundaries. Revalidate deferred operations after listener writes and refuse stale decisions without recomputing.

Join ordering and next-phase spawn visibility are proposed Sunrise choices pending vanilla oracle comparison. They must not reorder existing vanilla families incidentally. Client interpolation/render updates never execute authoritative behavior.

Use #1606's residency journal and visibility boundary for insert, lift, restore, move and retirement. Unloaded or parked entities persist but do not tick; ordinary entities do not acquire chunk tickets merely by existing. Temporary lift is not removal and cannot retire attachments or emit a tracking despawn. Commit publishes changes once; abort restores the original join identity, row and attachments. Save refuses while a lifted pass or entity transaction is open. Dimension transfer preserves UUID and data, invalidates source-scoped handles, removes source tracking and creates destination tracking atomically. World close invalidates its epoch.

## Save, load and missing mods

Save common fields plus namespaced type key, payload schema version and bounded attachment payloads to the existing entity-region owner. Serialize committed snapshots only. Load validates sizes, nesting, UUID uniqueness, passenger cycles and coordinates before insertion; decode and construct privately, then publish through the same join boundary as fresh spawn. A failed codec cannot leave a partial live row. noSave types produce zero persisted records.

For a missing type/mod or unsupported payload version, preserve the original bounded record as opaque quarantined data, with zero ticks, collisions or tracking. Report the unavailable type once per load summary. Save preserves that record without rewriting its payload; reinstalling the mod permits a later load. Never silently delete it or instantiate a vanilla substitute. Invalid or over-budget records produce an explicit recoverable load error and retain the original region backup; never truncate into a valid-looking entity. Migration is explicit schema decoding, not an old-API compatibility path.

## Tracking and rendering handoff

Negotiate required type/schema/features before joining. A client missing a required entity implementation is refused with a clear reason before world state is sent. Map stable keys to connection-local protocol slots; server ids alone are insufficient. Interest entry sends one bounded spawn snapshot (identity, type, pose and explicitly synchronized fields); subsequent revisioned deltas follow declared update cadence. Late join receives current state, not replayed ticks. Interest exit, removal and dimension departure send one removal; reconnect gets a fresh snapshot. Ignore stale-incarnation deltas and request a bounded resnapshot after revision gaps.

Persistence does not imply synchronization: only declared fields cross the network. #813's shared payload transport should own attachment synchronization rather than adding a competing channel. Decode into client-owned tracked rows. Hand immutable render snapshots to a client type renderer through the existing drawing context; no server Part tick or mutable simulation access during drawing. Missing optional visual assets use a diagnostic visual; missing required behavior refuses connection. Resource reload disposes cached render resources without replacing entity identity.

The [moving-structures carrier](moving-structures.md) remains a game-owned specialized type and does not depend on general registration. Share identity, residency, tracking and save boundaries; its cell ownership, swept collision and assembly journal remain specialized. Never create two carrier tickers or serialize its cells twice.

### Implemented: tracking, the client table and the renderer handoff (stage 5)

- **The tracker.** General rows join the level's one tracker (`lan_general_states` in `game/network/lan_entities.rs`). A row's range and interval are its declaration's (`tracking::general` in `lan/tracking.rs`): the range in chunks is capped by the view distance, and an interval of `i32::MAX` or below 1 never updates. Deltas are on. A quarantined record never becomes a row, so it is never tracked.
- **Pairing.** A general key is in no client registry, so pairing sends a `sunrise:general_entity` spawn (`crates/sunrise-protocol/src/general_entity.rs`) in place of `AddEntity`. Only a client that negotiated that engine channel is paired (`Viewer::general_entity`, `Guest::negotiated_general_entity`). After the spawn:
  - moves, position syncs, teleports and the removal are the vanilla packets keyed by the network id;
  - the row's declared fields are its synced attachments, whose snapshot tracking start queues as for any entity.
- **Vanilla clients.** A vanilla client is never paired with a row: no packet names it. Refusing such a client when required general types exist is still the open question below.
- **The client.** A Sunrise client keeps the rows it was shown in `game/network/remote/general.rs`. Each row's type is resolved in the client's frozen catalog. It moves with the non-living interpolation (3 steps), keeps the position and rotation of the tick before for a frame's partial tick, and is dropped on removal or a world change. An unknown key or a duplicate id is ignored with one warning. No mod code runs on the client.
- **Shown with its fields.** The server sends every row its first attachment snapshot at tracking start, empty or not, and the client shows a row only once the spawn and that snapshot are both there.
- **The renderer handoff.** `resolve_entity_renderers` resolves a renderer registered for a general key into a slot per catalog slot (`EntityRendererTable`, `game/modding/native_entity_renderers.rs`), so a frame does no lookup by name. Each frame, shown rows are culled by distance, frustum and section eligibility. General rows take their client entity lookup slot at spawn and submit there once their first attachment snapshot has arrived. Each extracted row goes to its type's renderer (`game/mobs/general_draw.rs`) through the guard every route shares, at `lerp(previous, position, partial)`, with `EntityRenderState`'s identity, age and angles interpolated the same way. Extraction captures a bounded, immutable snapshot of only that row's synced attachments, built when sync changes. Native renderers read it via `EntityRenderState::attachments` and draw through `EntityRenderer::submit`, without a live host. A type with no renderer draws nothing and is named once a resource load; a failed renderer draws nothing until the next load, and a reload re-resolves the slots without touching the rows.

## Lifecycle route owners and migration obligations

Paths below are verified existing integration owners relative to this repository. They identify boundaries to extend, not proof that general types already traverse them.

| Route | Existing file:line owner; required treatment |
| --- | --- |
| Part registration and dispatch | `crates/sunrise-mod-vocab/src/parts/entities.rs:124` registers behavior; `:53` resolves a type's Parts. Resolve into slots at load. |
| Mod spawning, including callers using level handles | `crates/sunrise-client/src/game/modding/native_level_spawning.rs` owns `level_spawn_entity` (#2641; `level_spawn_mob` is its living route at rest): `SpawnBody::General` builds a fresh record and admits it through `admit_general_record`, the single-record admission chunk loading shares, without pretending every entity is a mob. |
| Natural/spawner policy | `crates/sunrise-client/src/game/mobs/natural_spawning.rs:43` gates eligible chunks. General MISC types do not gain natural spawning implicitly. Explicit spawn adapters use the same insertion boundary. |
| Identity, reads and queries | `crates/sunrise-client/src/game/modding/native_entity_refs.rs:101` resolves targets, `:176` creates refs, `:245` queries entities. Extend all three to general rows; do not scan families for each lookup. |
| Temporary lift | `crates/sunrise-client/src/game/modding/native_entity_refs.rs:86` owns held-out access. General rows must retain residency visibility and balance restoration after panic. |
| Residency insert/remove/unload/restore/world reset | `crates/sunrise-client/src/join_seq/residency.rs:95` journals events, `:108` drains them, `:121` resets epoch. Integrate the general owner here; retirement waits for root commit. |
| Save and chunk reload | `crates/sunrise-client/src/save.rs:3101` loads entity tags; `:3316` and `:3320` save synchronously/asynchronously. Both consume the same committed general snapshots, including quarantine. |
| Dimension/passenger transfer | `crates/sunrise-client/src/game/entities/vehicle_teleport.rs:231` transfers vehicles, `:391` passengers. Validate the entire tree before moving any row. |
| Server tracking and ids | `crates/sunrise-client/src/game/network/lan_entities.rs:130` runs native tracking, `:197` builds object states, `:578` supplies UUIDs, `:590` assigns ids. Include general rows once. |

Every creation source must reach the general insertion coordinator: summon, mod/fake-player spawn, item use, dispenser, projectile creation, breeding, natural and block spawners, conversion, chunk load and dimension arrival. Every exit must reach retirement: discard, death, despawn, conversion, chunk unload, dimension departure, disconnect-owned cleanup and world close. Client spawn/update/remove packets and renderer disposal require parallel coverage. Exact leaf owners for those sources, the authoritative general tick seam and client renderer/packet dispatch are **not yet verified** by this note; implementation must trace and record file:line callers before enabling them. Existing vanilla-only routes remain unchanged until explicitly migrated. This is a coverage gap, not an exhaustive verified caller inventory.

## Trust and cost

The native loader checks the exact build. The old proposal gated enablement on both adapters providing registration, tick, codecs and client handoff; the guest adapter is no longer part of the contract.

Treat registration, callback results, serialized input and packets as untrusted. Reject NaN/infinite positions or dimensions, negative/overflowed lengths, unknown owned type ids, duplicate UUIDs and stale handles before writes. Unrelated unknown ids retain their existing behavior. Propose payload limit 64 KiB per entity, nesting 32, positive dimensions at most 64 blocks, tracking at most 32 chunks and update interval 1–1200 ticks; confirm these provisional limits before implementation. Quarantined records of a missing mod are bounded too — 69 632 encoded bytes per record, 256 records per chunk; both provisional, with no vanilla analogue (vanilla deletes such entities rather than preserving them). Bound aggregate decoding and outgoing bytes per connection too.

Catch native unwinding at dispatch: abort the callback transaction, discard buffered effects, restore lifts, disable the failing subscriber and report it. A native process-aborting panic cannot be recovered. No irreversible external I/O inside transactional callbacks. A subsequent built-in action must still succeed.

No per-entity mod list, type-name lookup, serialized-data read or family scan. Indexed type dispatch costs O(1) plus actual subscribers; identity lookup O(1); one O(n) eligible-row walk is the necessary tick work. No subscribers means zero hook payload allocation and zero mod calls. Maintain interest/spatial indexes on movement and insertion, and encode only dirty tracked fields. Measure actual callee costs before claiming this bound; current string-based Part lookup alone does not prove it.

## Acceptance matrix

Future implementation tests, not results of this documentation change:

| Fixture | Concrete expected result |
| --- | --- |
| Register test:counter, spawn at (0,64,0), counter=40 | One general row, zero throwable rows; one server tick produces 41. |
| Two instances initially 40 and 7 | One tick gives 41 and 8; no shared mutable Part state. |
| Parent and passenger, each counter=0 | One phase gives 1 and 1; callback trace parent then passenger. |
| Callback spawns B during A's tick | B counter=0 at phase end, 1 after next phase. |
| Save at 41, reload, late client joins | Same UUID, counter=41, one client entity, zero template entities; first resumed tick gives 42. Old handle refuses. |
| Missing mod, save again, reinstall | Zero live rows/ticks/packets while absent; payload bytes preserved; reinstall restores counter=41. |
| Park for 10 ticks then resume | Counter stays 41 while parked, becomes 42 on first eligible tick. |
| Lift/restore and outer abort after tentative removal | Zero despawns, original join identity and counter=41 restored. Committed removal emits one despawn and retires holder once. |
| Transfer dimension with one passenger | Zero source rows, two destination rows, UUIDs unchanged; source handles refuse; one destination tick each. |
| Reuse runtime id 42 after removal | Old ref and old delta cannot affect replacement; replacement counter remains 7. |
| Join/reconnect/interest exit | One spawn snapshot per entry, one removal per exit, no server behavior on client. |
| Duplicate type, NaN pose, 65537-byte payload, passenger cycle | Refused with zero published rows/effects; next valid built-in step succeeds. |
| Native panic during tick | Counter and listener edits rolled back, balanced lift, zero outbound deltas; next valid built-in step succeeds. |
| No Parts on 1000 general entities | 1000 common ticks, zero callback calls and zero callback payload allocations; no per-entity scans. |

Run the scene on integrated, LAN and dedicated servers. Pin ordinary entity ordering and lifecycle behavior against vanilla oracles separately from Sunrise-specific expected values.

## Proposed stages (file none)

1. Complete caller inventory, vanilla load/tick oracles and permitted 26.3 public-contract evidence; obtain Ari's decision on the issue.
2. Add frozen type registry, bounded codecs, general row/index and attachment identity; test duplicate keys and stale refs.
3. Integrate transactional insertion, retirement, residency, passenger transfer and committed save/quarantine; test rollback and recovery.
4. Generate behavior and both target adapters; integrate authoritative scheduling with explicit ordering tests.
5. Negotiate tracking, snapshot/delta transport and renderer handoff; test late join, reload and malformed packets.
6. Enable the acceptance scene only after all routes are audited and hot-path allocation/index measurements pass. No Rust or API changes belong to this note.

## Open questions

- Exact NeoForge 26.3 contract remains unavailable. Recommendation: obtain its public Javadoc before claiming counterpart parity; ask Ari to approve explicit Sunrise choices where public documentation is silent.
- Must general ticks share one order with all vanilla families immediately? Recommendation: maintain stable general join order initially and require oracle evidence before migrating existing family scheduling.
- Missing-mod recovery versus fail-open world load? Recommendation: quarantine losslessly with visible diagnostics; malformed/oversized records require explicit recovery, never silent deletion.
- Which exact bounds are acceptable? Recommendation: start with the provisional limits above, measure valid workloads and agree aggregate world/connection quotas before enablement.
- Which EntityPart moments ship first? Recommendation: authoritative tick plus required lifecycle/codec integration only after matching existing named moments; do not invent parallel join/remove hooks. Coordinate with #779 and #796 before API generation.
- How does a vanilla client connect? Recommendation: refuse when required general types exist; approve a separate explicit protocol decision before advertising any vanilla-client representation.
