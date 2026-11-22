# Design: resource-backed special item renderers

> The current native contract is in “Settled on the build.” The original proposal below it is historical.
> See [special item renderers](../../modding/src/how-to/client/special-item-renderers.md) for the current native API.

Provisional answer to [#876](https://github.com/arismoko/horizon/issues/876):
what resource-backed baked geometry and typed stack-derived state may a special
renderer submit? Geometry belongs in resources; callbacks select and pose baked
parts only where ordinary item-model properties cannot express the behavior.

## Settled on the build

The [original build plan](https://github.com/arismoko/horizon/issues/876#issuecomment-6864610602)
set the registration and resource lifecycle. This section describes the current native contract,
which supersedes the proposal below.

- **Registration.** The client fires `register_special_model_renderer` once at startup, before
  loading resources. A `SpecialModelFactory` pairs a decoder with a bake function that consumes
  its typed output. Each resource load uses the registered factories to decode and bake each
  `minecraft:special` node, without firing the event again. The last registration of a type
  wins, including for vanilla types; the game logs the replacement.
- **Definition failures.** An unknown type or an error or panic in decode or bake refuses the
  enclosing item definition, not the whole resource load. The item draws the missing model; it
  does not keep its old definition or try one from a lower pack.
- **Draws.** On each draw, `extract_argument` receives a `ClientHost` and the borrowed stack
  that selected the node. It can read effective components without constructing an owned patch.
  `submit` takes the typed argument and the actual display and time context, and submits baked
  geometry or nested stacks in scoped poses. The game keeps no draw recipe or answer cache.
  These callbacks run on the client's frame, not inside a tick's lifted pass or action transaction.
- **Draw state.** The engine applies the `base` model's GUI light, particle and display
  transforms, plus the node's own `transformation`. The outer item's foil applies to direct
  geometry, and a nested stack uses its own foil, tints and display transform. Packed light,
  overlay and outline stay on the ordinary item draw path. Extents come from the submitted
  geometry; the mod does not declare them. Tints are packed ARGB, not HDR colors.
- **Draw failures.** An error, panic or invalid geometry or pose undoes the entire special layer,
  including its nested submissions. Ordinary refusals are reported each time without disabling
  the renderer. A panic logs and shows an error toast naming the mod, and disables only that
  registration's draw callback for the session, even across reloads. Only the outermost refused
  layer draws the missing model; independent layers remain intact.
- **Leases.** The bake context issues leases for the current resource load. Submit them only in
  that load. Once resolved, an item render state retains its baked geometry independently and
  can finish drawing after a reload without a separate retirement step.
- **Cost.** Unmatched registrations do not invoke mod callbacks. The allocation regression check
  compares built-in scenes with and without an unmatched registration. The [direct-submission
  benchmark](../../performance/special-item-renderers.md) compares the same geometry drawn
  through a built-in model and through a registered renderer.
- **Out of scope.** Block renderers use a separate registration and are not part of this contract.

### Why recipes became submissions

`8019bce5490` introduced named-model recipes to let a renderer choose and pose pack geometry
while the engine retained the normal item draw state. The recipes used Wire for the then-existing
guest boundary; they did not need disk or network transport. `685572c8626` retained the stack
cache and added a frame policy for answers that depend on display or time. Direct submission
replaces both policies: each draw extracts a fresh argument and submits baked geometry under the
actual context. Neither recipe-part nor nested-stack count caps apply; only `(stack, display)`
recursion cycles are refused.

## Reference and boundary

The sections below record the original proposal, including the guest route and its
acceptance matrix. Where they differ from “Settled on the build” above, they do
not describe the current contract.

Vanilla 26.3 `SpecialModelRenderers.bootstrap` registers type identifiers and
unbaked codecs. `SpecialModelWrapper.update` extracts an argument from the stack,
creates a layer, records extents, transform and foil, and installs the special
renderer plus argument. Verified using `tools/rea_vanilla.py method` for both
methods from `/workspace/sunrise`. The `callers` query for the wrapper's `update`
returned zero direct references; interface dispatch means that is not a complete
context census. Sunrise routes are listed below instead.

The named NeoForge counterpart is `RegisterSpecialModelRendererEvent`. Its
[26.3 public Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/event/RegisterSpecialModelRendererEvent.html)
was inaccessible during this investigation. Public [client-item documentation](https://docs.neoforged.net/docs/resources/client/models/items/)
describes stack state preparation and submission, but currently labels itself
26.1. Exact 26.3 registration timing, cancellation, codec shape and capability
limits remain **unverified**. Require public 26.3 documentation before adopting
those details; the contract below is proposed Sunrise policy, not asserted parity.
No foreign renderer source was read.

`level-extracted-state.md` is absent from `arismoko/main` at this worktree's base
`ff35cdee95`. [PR #2357](https://github.com/arismoko/horizon/pull/2357) is an
adjacent proposal only; this design neither copies it nor requires its contents.

## Proposed registration, bake and preparation

One `register_special_model_renderer` client registration moment takes a
`ClientHost` and a typed registrar. Dependency-ordered listeners register a
namespaced type and one preparation callback, which reads the whole stack. A later registration of a type replaces the earlier one
and the log names both owners; vanilla built-in identifiers can be replaced. Resolve
identifiers and callback slots at load/bake, not on every draw. No second event
bus or compatibility alias is introduced.

API-owned `SpecialRendererRegistrationWire`, `SpecialResourceSchemaWire`,
`SpecialBakeRecipeWire`, `SpecialStackInputWire`, `SpecialDrawContextWire`,
`SpecialDrawListWire` and `SpecialPartInstanceWire` define explicit integer
widths, enum tags and lengths. Generated native and wasm adapters share these
representations and validation; game JSON values, Rust engine structs, GPU
pointers and borrowed live stacks never cross the boundary. Wire is an API
boundary, not a network synchronization format.

Resources declare named geometry parts, texture/material references, local
transforms. Reuse existing model loading/baking and
prepared geometry ownership. Bake once per resource reload; callbacks cannot
create geometry, compile shaders, read arbitrary files or recursively render
items. `UnsupportedSpecialRendererCapability` refuses registration requesting
raw pipelines, live world access, arbitrary codecs or recursive item draws. Unknown special types produce
`UnknownSpecialRendererType` with resource path and type, replacing the foreign
base-model workaround. This refusal applies only to special-model type decoding;
unrelated unknown identifiers keep their existing behavior. Built-in specials
retain their existing renderer paths. A failed candidate resource reload leaves
the previous complete resource set active; initial failure uses the ordinary
missing-model diagnostic rather than silently drawing `base`.

Client-lib decides preparation input; game fires the registered callback with
`ClientHost` and a typed drawing context; client-lib validates and applies in one
transaction. Input is an immutable copy of declared component values, display
context, owner snapshot, deterministic seed, pose, packed light and foil policy.
The drawing context records instances of this recipe's baked parts with local
poses and vanilla-space tint. It exposes no device, frame format or live entity.
The output is an immutable draw list stored in the ordinary item render state;
GUI and world consumers execute it without calling the mod again. Engine display
transforms, clipping, lighting, glint, sorting and bounds remain authoritative.
Bounds come from the drawn parts' own quads; the mod declares none.

Apply rechecks the captured stack revision, resource holder and mod lifetime;
changed preconditions refuse the prepared result, never recompute within apply.
A later ordinary preparation may retry. Preparation writes and staged draws roll
back together. Inside lifted passes, publication, retirement and journal emission
wait for root commit; rollback publishes nothing. Saves are refused during an
open action savepoint under the existing transaction rule, even though rendering
itself adds no persistent state.

## Lifetime and colour

Use the existing resource lifecycle holder and its identity for baked references
and leases, with resolved part slots. Do not add a special-renderer generation
counter. Reload builds a complete candidate, validates/bakes it, then publishes
atomically at root commit. Old queued states hold leases until submission ends;
they cannot publish into the replacement holder. Retire resources after leases
drain. Unload stops new callbacks, drains outstanding callback/draw leases, then
unloads native code or wasm stores. Device recreation rebuilds engine GPU objects
from resource data without exposing their identity to mods.

Follow [mod render colour](mod-render-colour.md). Packed ARGB tint and light
0..=15 remain vanilla-space inputs, converted once by the engine. Shader colours
are linear BT.709, paper white 1.0, values above 1 allowed. Generated entry/exit
boundaries own storage conversion. Texture data channels stay data; coverage does
not grant ownership of frame alpha. Shader-pack and SDR/HDR changes preserve this
contract and cannot make a callback depend on the frame format.

## Current route owners

Verified against base `ff35cdee95`; paths are relative to `crates/`. These are
integration owners, not claims that the proposed callback already exists.

| Route | File:line | Required behavior |
| --- | --- | --- |
| Decode / foreign workaround | `sunrise-assets/src/item_model/special.rs:82`, `:90`, `:103` | Resolve registered types; report unknown special types |
| Resource parts and baked extents | `sunrise-client/src/item_render/special.rs:51`, `:307`; `sunrise-client/src/item_render.rs:748`, `:841` | Bake resource recipe and conservative bounds |
| Shared stack preparation, hands | `sunrise-client/src/game/interface/item_draw.rs:376`, `:398` | Freeze declared inputs and prepare once per resolution |
| Inventory, cursor and other GUI icons | `sunrise-client/src/game/interface/inventory_slots.rs:6`; `sunrise-client/src/game/interface/item_draw.rs:455`; `sunrise-client/src/item_render/gui.rs:180` | Same prepared parts, GUI clipping and lighting |
| Dropped/ground items | `sunrise-client/src/game/interface/item_draw.rs:705`, `:789`, `:793` | Copy poses reuse state on GPU and CPU paths |
| Frames and armor-stand equipment | `sunrise-client/src/game/entities/frames.rs:749`; `sunrise-client/src/game/entities/armor_stands.rs:1786`, `:1847` | Fixed/head/hand contexts use same preparation |
| Other world item consumers | `sunrise-client/src/game/blocks/campfire.rs:172`; `blocks/vault.rs:250`, `blocks/shelf.rs:230`, `blocks/brush.rs:347` under the same `game/` directory | Preserve context and engine lighting |
| Entity/effect and mod-submitted items | `sunrise-client/src/game/items/potions.rs:2219`, `items/totem.rs:224`, `items/item_particles.rs:18`; `sunrise-client/src/game/modding/native_block_entity_draw.rs:1237` | Share resolver, no recursive callback escape |
| CPU special submission / GPU submission | `sunrise-client/src/item_render.rs:267`, `:299`; `sunrise-client/src/item_render/special.rs:466`; `sunrise-client/src/game/interface/item_draw.rs:556` | Consume identical frozen draw list |
| Reload and GPU resource ownership | `sunrise-client/src/game.rs:14259`; `sunrise-client-render/src/gfx/prepared_model.rs:256` | Rebuild affected item states and retain leased resources |

Native and wasm need generated catalog registration, dispatch and callback exports
in the eventual implementation; neither adapter exists for this feature today.
Both must enter the same game preparation boundary and enforce identical failures.
The implementation review must trace first/third-person callers through the
shared hand resolver and reload publication through its full owner chain before
claiming exhaustive integration.

## Untrusted inputs and cost

Proposed extension budgets: 256 parts per recipe, 65536 vertices per part,
16 MiB decoded geometry per recipe, 64 KiB resource parameters, 256 instances and 64 KiB encoded draw output per preparation.
Use checked arithmetic before allocation; bound encoded and decoded sizes,
resource nesting to 32 and identifiers to 256 UTF-8 bytes. Reject invalid tags,
indices, foreign/stale part slots and dependency cycles. Transforms must be
finite; their size is not capped. These extension budgets do not cap vanilla's own
models, stack components or draw capacity, and the stack itself has no cap beyond
the reference's.

Catch unwind-capable native callback panics; wasm traps and fuel exhaustion have
the same policy: discard the entire draw list and callback transaction, diagnose
once and disable the registration for the holder lifetime. Draw the missing-model
marker for affected custom layers, retaining independent built-in layers. A later
built-in item still renders normally. Native aborts cannot be caught and native
callbacks cannot safely be forcibly preempted; wasm gets finite execution fuel.
Reject an invalid command atomically, never retain a partially accepted list.

With no registrations, retain the existing built-in branch: one O(1) resolved
variant test, zero extension allocations, serialization or listener scans. A
registered draw dispatches directly by resolved slot, copies only declared inputs
and records bounded commands. GPU/CPU submission has no callback or mod scan.
These are performance acceptance requirements, not measured results.

## Acceptance matrix

Future implementation tests use original assets, not third-party model files.

| Case | Concrete expected result |
| --- | --- |
| Original cube A and two separated cubes B; boolean component false/true | Exactly 6/12 resource quads selected in both inventory and hand |
| Same true stack in ground, frame, head and other-player hand | B selected in all four; engine context transforms differ |
| Resource reload changes B to three cubes | True stack selects 18 quads after commit; 0 new draws reference old holder |
| Reload with an unknown special type | Named error naming the item; that item draws the missing model and the reload goes on (Settled) |
| Queue old state, reload, drain | Old state still draws its 12 quads after the reload; its baked models are released once, when the last state holding them drops (no retirement step: Settled, Leases); a new resolve draws 18 |
| Rollback lifted preparation/reload | 0 published draws, retirements or journal entries; the renderer is asked 0 times inside a tick's lifted pass (Settled, When it runs) |
| Save during action savepoint | Refused; no persisted partial state |
| NaN transform, 257 instances, stale part, oversized input | Entire custom layer refused; 0 partial instances |
| Native panic / wasm trap followed by vanilla cube | Custom layer missing marker; vanilla cube still 6 quads |
| Unsupported raw-pipeline capability / duplicate registration | Unsupported capability: named registration refusal. Duplicate or vanilla id: not refused; the last registration wins, with one log line naming both (Settled) |
| White 1, encoded tint 0.5 | Shader values 1 and approximately 0.21763764, through vanilla's tint path; frame alpha engine-owned. Linear RGB 2 waits for a typed HDR tint (Settled, Colour) |
| Native and wasm, same inputs | Identical part slots, transforms, tint and refusal outcomes |
| No listener, repeated built-in scenes | Identical baseline output; 0 extension callbacks/allocations |

## Proposed stages (file none)

1. Review this contract and obtain public NeoForge 26.3 registration evidence.
2. Finalize API-owned Wire schemas, generated native/wasm boundary and refusal
   diagnostics; connect resource decode/bake to existing lifecycle ownership.
3. Integrate shared stack preparation and both submission paths, transactional
   reload/teardown and colour rules; trace all caller routes above.
4. Add original-asset acceptance scenes, failure injection and empty-registration
   counters, then measure cost. No implementation files are proposed in this note.

## Open questions

- Exact NeoForge 26.3 registration contract is unverified. Recommendation: require
  public 26.3 docs/Javadoc before locking the counterpart's signature and timing;
  do not infer it from older docs or source.
- R2 prepared-part API and practical extension budgets need implementation review.
  Recommendation: use the existing lifecycle holder with the budgets above,
  test representative original scenes, and enlarge only extension limits with
  measured evidence; preserve vanilla capacity throughout.
