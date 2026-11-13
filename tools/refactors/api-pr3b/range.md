# API-local block state ranges

This is a handwritten prerequisite to the final API 3b swap, not the mechanical swap.

`BlockStateRange` now has private first `BlockState` and count fields. Its fallible constructor validates the u16 ordinal span once. `contains` checks the key as well as the ordinal, and iteration cannot wrap the validated span. Hidden `BlockStateRangeRecord` uses the existing derived via codec and delegates decoding to that same checked constructor. There is no handwritten wasm codec.

`BlockStates::states_of` uses immutable definitions, including the complete 65,536-state domain, rather than the world's numeric interval. Unknown blocks or unpublished metadata preserve the existing Option refusal. Runtime `BlockDefinition::state_count` exposes the immutable table's length. Known definition iteration is checked against independent property values (`false`, `true`), not world numbering.

The previous numeric range implementation moves unchanged from the API root into `horizon_world::registry::state_range`. The engine's legacy ResolvedBlock binding still carries that numeric range pending the final storage/binding swap. Its constructors are migrated atomically; no alias makes the new API range a numeric engine range. Host docs, book and the resource-backed host test's range expectations use local ordinals.

## Executed checks

- `cargo check -p horizon-mod-api --tests`: passed.
- `cargo check --workspace --tests --examples`: passed for this cross-crate type cut, including the new owner-index and paired landing callback tests.
- `cargo test -p horizon-mod-api --lib local_range_validates_transport_and_never_wraps_ordinals`: one test executed, passed. Covers overflow, wrong block key, invalid decoded span, empty span, complete u16 domain and upper-bound iteration.
- `cargo test -p horizon-mod-api --lib metadata_service_reads_edits_and_refuses_invalid_transport`: one test executed, passed. Includes table-backed range iteration and unknown-key refusal.
- `python3 tools/refactors/api-pr3b/state_mutants.py --group range --output …`: all five mutants killed by executed assertion failures (each exit 101): accepted overflow, membership ignoring key, iteration ignoring the start ordinal, transport bypassing the checked constructor, and metadata omitting its last state. Sources restored in finally; both focused baselines above passed again after restoration.
- Touched Rust files formatted; diff whitespace check passed.

Detailed logs and mutant results are retained in the room's blockstate host proof directory. No broad test suite, clippy, parity or in-game run was performed. The resource-backed host fact test was type-checked, not executed for this cut. Local wasm32 compilation remains blocked by the unavailable target std/rustup; final exact-head lint/verify and full API 3b WIT/Host/hook reports are still owed.

## Hot path

No new per-tick lookup or conversion. The definition length lookup is constant time. Range iteration produces API values without allocation; containment is constant time. These complexity statements are not a new timing measurement.
