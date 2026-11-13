# Hand-written: fixture resource lifetimes

Fixture registration now finishes before `load_resources` constructs a Game's complete
resource graph. The shared builder reads block names from an owned preliminary facts registry,
registers the fixture keys, then loads the final resources normally. No index is prepared for
the preliminary registry. Goal registrations happen before the shared builder; vanilla/modded
comparisons use separate complete Game lifetimes rather than reinstalling Parts into a live
Game. The living-use fixture no longer resolves an already-completed fixture after teardown.

Production `retain_or_prepare` is unchanged: an existing registry refuses replacement metadata.
There is no cache reset, test-only acceptance path, second numeric table, or partial replacement
of a Game's assets or generation handles.

The generator-data cache now also matches the owning registry pointer, using its existing
`GeneratorData::registry()` ownership. Path and worldgen overrides alone are insufficient for
same-path resource reloads. Repeated queries in one lifetime still share the same data.

## Evidence

Source head `c2fa48e85d6a430c369aaf3889bdf044bc2de52c`, directly based on `3172000e63`:

- The client library test binary built successfully with `CARGO_INCREMENTAL=0 cargo test -p
  horizon-client --lib --no-run --message-format=json`.
- All original 49 affected fixtures actually executed and passed, using an immutable binary
  copy, a separate process and scratch homes per test, at most two concurrent processes, and
  a 540-second timeout per test. No fixture was skipped.
- Six focused tests passed: the late goal/random-stream regression, item-contact regression,
  actual typed level-write regression, reordered retained-index test, racing-owner refusal
  test, and the new `fresh_game_lifetimes_keep_worker_and_generator_snapshots_separate` test.
- The new actual-Game test proves that preparation against the old registry refuses replacement
  metadata, the fresh registry retains a distinct Index Arc, an off-thread old worker still
  reads the unchanged old snapshot, and same-path generator data belongs to its correct
  registry. Both index and generator data are reused within a single lifetime.
- Removing the production cache's registry-identity condition compiled successfully and was
  assertion-killed by that actual-Game test at the distinct-generator-Arc assertion. Neither
  compiler failure nor timeout counts as a kill. The mutation runner restored the original
  source in `finally`; its result is retained under `cache-mutant/result.json`.

Machine-readable results and individual process logs are retained in the shared integration
proof directory `api-blockstate/lifetime-proofs`, under `final-fixture-batch` and `final-focused`.
The earlier new-test run used the resource root instead of its `worldgen` bundle directory and
failed with missing-file I/O; it is not counted as proof. The corrected run above passed.

## Cost and limits

The production change adds one pointer comparison to a cold generator-cache lookup. There is
no tick-path allocation, conversion, registry reconstruction, or per-worker table construction.
This is functional evidence, not a tick-time benchmark. Exact-head CI, workspace/clippy/parity,
wasm suites and in-game benchmarks were not run locally; they remain the containing integration's
responsibility.
