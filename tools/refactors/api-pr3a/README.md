# API PR 3a: item boundary

Part of #1608. This directory holds the repeatable proof and edit log for the
item type swap. The host manifest tool is reused from API PR 1, not duplicated.
`manifest_gate.py` compares method names, owners, documentation and hidden flags
while allowing signatures to change. Every deliberate service change needs an
exact old/new line exception; missing or stale exceptions fail the gate.

## Boundary decisions

`ItemKey` remains a process-local index in the existing runtime key table.
Registration can return a forward reference before the game loads its registry;
a copied key must not carry unresolved item metadata.

The API-owned stack guarantees a nonzero count and stores numeric damage as
`Option<u32>`: absence inherits the prototype, `Some(0)` is explicit zero, and
removal is an explicit component-patch entry.
Its constructors and mutators do not promise registry-dependent validity. The
game owns the item registry and checks the effective max count and max damage
(including per-stack component overrides) at the write boundary, returning a
typed error rather than clamping. Mods can query metadata before attempting a
write. This gives native and wasm mods the same implementation and validation
contract without target-specific globals.

Component data remains an optional, owned NBT patch. Plain stacks need no heap
allocation. Component access is through typed descriptors rather than the
engine's component struct. The wasm record is generated from a declared data
form, not a hand-written stack codec.

## Known consumers

Contributor #1730 introduced `mod_api::testing::FakeHost` in `src/testing.rs` and
`src/testing/{level,players,services,registries,journal}.rs`, and migrated
handwritten hosts in `transaction.rs`, `lib.rs`, `hooks_tests.rs`, wasm
`groups`/`sample`/`points`, `horizon-mod-points/tests/wire.rs`,
`horizon-client/tests/mod_api_hooks.rs`, `brewing.rs` and `menu.rs`. These consumers
are migrated after merging main, including #1730 and #1651. Widget additions from
#1694 carry no stack in `widgets.rs`; GUI stack drawing still converts the API
value in `game/host/draw.rs` through `api_values.rs` before the engine renderer.

Those hosts must use owned stack constructors and getters, `ItemKey` resolution
for vanilla and mod items, `Option<ItemStack>` for absence, and the keyed `Tags`
service. Implement `max_stack_size(ItemKey)` and `max_damage(ItemKey)` metadata
reads without substituting a made-up maximum for unavailable metadata. Raw
component setters are fallible, and resident writes preserve all original values
on typed refusal. Numeric damage distinguishes inheritance, explicit zero and
removal, including in fake registries and journal assertions.

## Proof commands

The focused gate tests are independent of Cargo:

```sh
python3 -m unittest discover -s tools/refactors/api-pr3a -p 'test_*.py'
```

The final gate consumes manifests generated at the base and the new head.
Signature changes, WIT record changes and hook dispatch equivalence require
separate review/proofs; the manifest comparison does not claim those are equal.

Generate each checkout's API outputs with its own generator (the same
`horizon_mod_api_gen::generate(root, out, Target::Api)` entry point used by the
API build script). The refreshed structural baseline is merged main `8843d83923`, including the widget
consumers. Earlier archived reports used `8fcd440a3e675499b83277490376dfa92001d14e`.
Run `host-manifest` on the generated `mod_api_generated_host.rs`, generated
`mod_api_host_areas.rs`, and that checkout's `src/host/*.rs` files. Then run:

```sh
python3 tools/refactors/api-pr3a/manifest_gate.py before.txt after.txt \
  tools/refactors/api-pr3a/host-manifest-exceptions.json
python3 tools/refactors/api-pr3a/report_wit.py "$BASE_OUT/mod-api.wit" "$HEAD_OUT/mod-api.wit"
python3 tools/ci/wit_report.py "$BASE_OUT/mod-api.wit" "$HEAD_OUT/mod-api.wit"
```

The ten exact manifest exceptions are intentional: the old tag helpers leave
for `Host::tags`; `item_key` now covers every item; `max_damage` is added;
container transfer docs distinguish resource identity from transferred amount;
and merchant setter docs describe typed refusal. Other names, owners, docs and
hidden flags must remain unchanged. Existing enum/variant case positions are
checked, not merely the relative order of surviving cases.

## Review regressions

With an idle Cargo build tree, run:

```sh
python3 tools/refactors/api-pr3a/check_review_mutants.py
```

It runs the current production boundary tests, temporarily bypasses clicked-slot
staging and drops the component snapshot, requires each executable regression to
fail, then restores the exact source bytes and reruns the baseline. Logs and the
production source hash stay under `target/review-1674-mutants/`.

The clicked-slot owner, cursor and cancellation flags are staged together,
including menu operation queues and map reservations. The three block-entity tick,
menu-button and menu-quick-move wrappers stage only their captured outputs.
Independent host storage writes and non-storage effects are not rolled back by
output validation. General callback storage journaling is separate work in #1770;
no journal changes or deferred restricted `LevelHost` callbacks are included here.

## Actual native benchmark

`bench_native.py` compiles the selected checkout's real generated `fire_use_item`
shim, one subscribed ItemPart, and (for the owned case) the checked-in game
converter with source hashes. Timings use the system allocator; allocation
counts use a separate instrumentation build. Nine batches of 200,000 calls
follow warmup. The baseline checkout must be main, not a modeled stack.

```sh
python3 tools/refactors/api-pr3a/bench_native.py --repo . --mode owned --output target/stack-bench-head
python3 tools/refactors/api-pr3a/bench_native.py --repo "$MAIN_CHECKOUT" --mode engine --output target/stack-bench-main
```

Reports include timings, allocations, bytes and sizes. Damage presence is read
through the engine's existing component-presence helper, while the single
engine component encoder omits that separately carried scalar. Numeric-only
snapshots, including explicit zero, allocate nothing. Other component patches
remain eagerly owned, not lazy; the PR body lists hot callers and the named
follow-up for lazy/cached patches. No whole-game tick or wasm speedup is claimed.
