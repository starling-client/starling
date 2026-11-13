# Owned block values and explicit worker metadata

Code cut: `97016ab3b3`, based on the combined API 3 head `1b1b7d1aa4`.
This item is an input to the single API 3 PR, not an independently merging layer.

## Boundary

`BlockState` is one private-field, Copy API value, derived through the existing hidden
`BlockStateRecord` transport. The former tint snapshot is removed atomically. Metadata is
an explicit `BlockMetadata` argument, borrowed from the host service or captured for a
worker. Neither a `Host` nor an engine registry crosses into a tint callback.

The game converts engine state records by resolving their complete property constraints
against published immutable definitions. Numeric enumeration never assigns API ordinals.
The renderer receives a prepared value array and metadata, copies values and invokes the
existing callbacks. Without a custom source, no tint value array is prepared. All 31 mesh
constructor callers are wired by `tint_callers.py`; `tint-callers.json` records its edits.
The covered Host `block_state_property` method is removed rather than kept as an alias.

## Executed proof

- `cargo check --workspace --tests --examples`: passed in 5 minutes 14 seconds.
- Table-backed metadata/facade service: 1 focused test passed. Edits preserve the original,
  update the returned value and propagate invalid-value/unavailable errors.
- Worker facade: 1 focused test passed; 1,000 known plus 1,000 missing property reads
  allocated zero times, with the original value unchanged on the owning thread.
- Verify-discovered Python load/report file: 2 Python tests passed, including 3 compiled
  production cases. The new case reorders world IDs while retaining east ordinal 1 and
  north ordinal 0, then refuses an unknown property value and unknown block.
- Real-resource CPU tint tests: 2 passed, including all three callback modes, neighbor
  block/property lookup, one call per meshed block, missing-metadata refusal without
  replacing prior layers and independently expected face colors. These tests loaded the
  prepared resource manifest; they did not take the no-resources early return.
- `state_mutants.py`: all 7 mutants were killed by executed assertions, not compilation:
  property-name loss, no-op edit, lost transport ordinal, property-read allocation,
  default-instead-of-property reload mapping, source installation without metadata and
  returning air for every neighbor state. All focused baselines passed again after restore.
- Touched Rust files were formatted; `git diff --check` passed.

Local wasm32 checking was attempted but could not start compilation: this box lacks the
`wasm32-unknown-unknown` standard library and has no `rustup`. This is not a wasm32 pass.
The required CI `lint` check owns that proof at the final ready head. No local clippy,
workspace test suite or in-game rendering run is claimed. The CPU tests cover this cut's
rendering behavior; no firing point, network or world-write behavior changes here.

## Generated surface report

Compared with the combined integration tree (unchanged API surface at `1b1b7d1aa4`):

- WIT removes only `host/block-state-property`; no added or changed function signatures.
- The sole changed declaration is `block-state`: the old snapshot record becomes the
  alias of the derived `block-state-record` transport. No type is added or removed.
- All 200 common enum/variant declaration orders are unchanged.
- Host manifest: 1,321 to 1,320 entries; only the covered property helper is removed,
  with no signature substitutions or added entries.
- Generated hook/dispatch (`mod_api_generated.rs`) and generated base Host
  (`mod_api_generated_host.rs`) artifacts are byte-identical. The area declarations
  change only with the intended helper removal.

Tick-time effect: no additional tick work. Worker reads copy a small value and borrow
immutable metadata; known/missing property reads allocate nothing. Registry/property
resolution and array allocation occur only while preparing mesh tables at load/reload.
The later numeric-index item reuses the same canonical conversion and supplies the
prepared array without repeating property resolution.
