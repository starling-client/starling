# Atomic internals ownership move

Part of #1607, U5d. This is one schema/consumer cut: native and wasm mods declare
fingerprinted `internals` at the manifest root. Native build pins and support
modules stay native. There is no dual-key reader; old native declarations fail
normal strict-schema deserialization.

## Replay and equivalence

At this PR's exact head, run `python3 tools/refactors/u5d-internals/move.py`.
`--apply` replays the move from the pinned base. The reviewable operation list is
`edits.json`: exact old/new line arrays and occurrence counts, with no guessed
source matches. All unchanged relocated lines cancel in the removed/added-line
multiset; only the enumerated edits may change its balance. The gate checks the
entire transformed file, not only selected lines, and rejects undeclared changed
paths. This is an archival refactor proof, not a check imposed on future PRs.

The four real manifest fixtures have a separate semantic check: parsed path,
hash and kind records must be identical, in order, and every unrelated manifest
field must be unchanged. Existing CLI tests check retained comments and both
inline-array and array-of-tables formatting through add, refresh and pruning.

The source edits declare the owner move, removal of duplicate wasm storage and
its special native-table extraction, strict common validation, reader/writer
redirection, fixture ordering, corresponding diagnostic assertions and the new
shared-owner status test. New coverage and this replay framework count as
hand-written work; they are not hidden as moved code. The PR body records the
final counts, focused tests, assertion-failing mutants and exact-head CI.
