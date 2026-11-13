# Hand-written: typed refusal for state-bearing level-handle writes

The three level-handle operations `set_block`, `update_neighbors` and `neighbor_changed`
accept API-owned `BlockState` and return `Result<bool, LevelWriteError>`. Their IDs are
unchanged. Stale handles, unavailable sides/phases/nonactive dimensions, and checked state
conversion failures are distinct from vanilla `Ok(false)` refusal. The native methods still
receive engine `BlockStateId` values; the generated crossing owns their validation.

`native_level_write_position` reuses the existing `native_level_world_result` read-residency
boundary, then requires the active level. Parked levels are unavailable writes. Unloaded and
out-of-height positions retain vanilla false. There is no second epoch validator and no
post-conversion raw-state validator in the neighbor methods. Entity write contracts and
`destroy_block`/drop collection signatures are not changed.

## Focused evidence

- Changed-client tests/examples check passed before the retained-resource integration merge.
- The actual ignored `level_block_writes_validate_and_change_world` Game test passed with
  prepared vanilla resources and isolated homes. It exercises generated invalid-ordinal and
  phase refusal, stale/nonactive levels, unloaded/height refusal, actual state changes,
  notification flags, and block-log effects, not just mock calls.
- Four production assertion mutants were killed by that actual test: ignored residency,
  disabled phase refusal, collapsed invalid-state errors, and claimed success without a world
  mutation. Compile failure, timeout or undetected failure does not count as a kill. Each
  changed file is restored in `finally` by `state_mutants.py --group write`.
- After merging parent `59fe4f9938` at `dd1eba14dd`, the restored actual Game write test
  passed again (1 passed, 0 failed, 96.29 seconds). This includes the added invalid-ordinal
  refusal through both generated neighbor-input crossings. The test binary compiled cleanly.
  A separately executed late-Parts fixture failed the new resource lifetime refusal; that is
  a fixture resource-lifetime task, not a passing combined test run.

`write_result_callers.py` records the balanced-assertion caller migration. Those edits adopt
`Ok(true)`, vanilla `Ok(false)` and typed errors; they are not a second implementation of the
write contract. All changes in this cut remain labelled hand-written, separate from the final
compiler-guided API type swap.

## Boolean entity/display write contracts

A separate handwritten generator follow-on keeps non-Result writes boolean when their state
input crosses the boundary. Checked Result wrapping is selected only for a Result signature;
invalid input on a boolean write returns false before calling the engine. This preserves the
entity write contract rather than changing its signature to fit a conversion helper.

The focused generator test `boolean_state_writes_refuse_before_mutation_without_changing_the_bool_contract`
compiles and executes the production-emitted argument code for scalar and optional states.
Invalid inputs preserve a sentinel state and zero writes; valid scalar, None and Some inputs
produce their independently expected states and counts. Replacing refusal with a default engine
value was killed by the executed fixture's assertion, and the restored test passed again.

## Hot-path effect

State conversion is an allocation-free forward/reverse index lookup. Typed phase refusal
happens before input conversion. Native writes reuse the existing residency check and retain
normal vanilla update work. There is no all-mod scan or new string lookup on these write paths.
The four-mutant proof is functional evidence, not an in-game tick-time benchmark.

Exact-head CI, wasm target compilation, the final API/WIT delta and the combined integration
surface report are still owed by the containing API PR; they are not claimed here.
