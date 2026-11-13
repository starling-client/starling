# Final owned block declaration cut

Comparison base: `4a6571d404`. Production/test source head before this report:
`d4783232f8`.

## Mechanical provenance

`c57d84d5e0` contains only the transformation script, its JSON rewrite log, and
what the script produced. The invocation was:

```text
python3 tools/refactors/api-pr3b/swap_state_symbols.py --root . --report tools/refactors/api-pr3b/swap-state-log.json --defer-constructor-fixtures --format --apply
```

The log records exact input/output SHA-256 hashes for 24 source files. The API
cut replaces 52 old state tokens and removes one old key token in the raw
reexport. Two private engine imports retain numeric provenance. Only the two
transitional explicit-raw dispatch lookup arms are removed; recursive inferred
state normalization and numeric engine storage remain. The source-only
mechanical diff is 52 inserted / 80 deleted lines; this is a symbol rewrite,
not a file/module move (0 moved lines).

Known constructor fixtures were explicitly deferred, not interpreted as ordinals
by the script. Subsequent compiler repairs are separate `Hand-written` commits:
owned key-import/sample records, private native imports and engine carriers,
checked DTO/LevelHost crossings, test callers, and API/book documentation. No
aliases, shims, second state map, new wasm form or wasm feature were added.

## Surface comparison

The generated Host manifest plus sorted Host area sources uses the same count
convention on both sides. `swap-surface.json` records the detailed comparison;
`swap-wit.diff` contains the full generated interface diff.

| Contract | Before | After |
|---|---:|---:|
| Host methods under this convention | 1,327 | 1,327 |
| WIT functions | 1,638 | 1,638 |
| WIT function names added / removed | 0 | 0 |
| Existing enum/variant case orders compared | 211 | 211 unchanged |
| Hook declaration names/order compared | 649 | 649 unchanged |

There are 23 directly changed WIT function signatures and 22 changed record
bodies. No records or enums/variants were added or removed. The obsolete
`block-id` and `block-state-id` aliases disappear. The existing owned state record
is reused rather than introducing another transport form.

## Observed verification

- Full `cargo check --workspace --tests --examples`: passed in 1 minute 44 seconds.
- `tests.test_mod_api_boundary`: 5 passed before the source push.
- `tests.test_block_state_swap`: 5 passed.
- Changed Parts fixture regressions: 13 passed; placement replacement/refusal
  short-circuit regression: 1 passed.
- Integrated source-identity regression: 1 passed, including assertion-killed
  wrong-source and removed-routing mutations of the actual observer body.
- Native `wasm-host` key-import regression: 1 passed, preserving refusal before
  backend execution and comparing the returned record's field tuple.
- Ten isolated resource-backed Game tests passed: typed world writes, immutable
  facts, late-invalid bulk atomicity, LevelWriteHost block-entity replacement,
  tag/fluid metadata, committed cauldron observations, display state/refusal,
  minecart state persistence, signal output clamping, and comparator/live-level
  signal refusal.
- Independent production review found no actionable defects, including the final
  LevelHost adapters and all mechanical input/output hashes. The reviewer did not
  independently execute the full workspace or a wasm target.

The earlier actual-world bulk regression also killed skipped-invalid-filter and
skipped-invalid-palette mutations and passed after restoration. The swap's final
resource run confirms that regression remains passing with the owned public
signatures.

The wasm sample target compiles, but its two artifact-dependent tests remained
ignored. Native `wasm-host` tests are not wasm-target certification. Exact-head
CI, wasm-target execution, broad suites, parity, GPU rendering and whole-game
tick benchmarks are not claimed here.

## Tick-path effect

Routing still uses the prepared numeric state index and scoped key table before
converting event payloads. The changed-neighbor identity is lifted only after
primary-state authority/routing admission, using the existing native default-state
table and the same checked index. No new names, allocations, registry scans or
mod scans were introduced on these paths. The generated fire path retains its
own routing check for subscribed calls. Engine storage and queues remain numeric;
all mutable output conversion is staged before application.
