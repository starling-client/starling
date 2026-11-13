# Typed bulk atomicity proof

`typed_bulk_late_refusals_preserve_the_world_and_emit_no_changes` exercises the actual
Game bulk entry point against a loaded chunk, rather than only checking preparation.
It uses the existing fresh Game fixture and enables its dedicated-server authority
surface. Production authority and dimension checks are unchanged.

The test supplies a valid first filter and a forged invalid second filter, then a
valid first palette state and a forged invalid second state. The valid run references
only the first entries. Each late invalid entry must nevertheless reject the whole
edit: the target stays air and the world's change log stays empty. Removing the bad
entry provides a positive control: the same run must write stone and report one change.

Observed focused results:

- Baseline: one resource-backed Game test passed.
- Dropping invalid filters during preparation: compiled, then assertion-killed by the
  changed world sentinel, not by an error-string or source-text assertion.
- Dropping invalid palette states: compiled, then assertion-killed by the changed world
  sentinel.
- Captured production compiler source restored in `finally`; the Game test passed again.

This closes the preparation-only coverage gap identified by independent review. It
adds no production behavior or wasm form. It does not claim a whole-game benchmark,
GPU proof, parity-suite result or wasm-target result.
