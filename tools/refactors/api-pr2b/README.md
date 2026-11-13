# Parts fragments become modules

Part of #1608. Initial proof base: `5f2ea648d8c02338082aa64def8ec1e98512985e`.
Replay after #1624: `a10e223e2673fca79975f7669db12eb869224f2c`.

The former generator merged repeated `Registrar`, `CapabilityProviders`, their trait
implementations, inherent implementations, and `tests` envelopes in lexical filename
order. Ordinary Rust modules cannot split a trait implementation across files. The
move consolidates the two contracts and implementations in `parts/mod.rs`, while
inherent methods and each kind's code/tests live in its module. Module declarations,
re-exports and the fixture aggregator are generated from the folder into
`parts_modules.rs`; adding a module needs no central discovery-list edit. The
`a_new_parts_kind_needs_no_shared_list` fixture checks additions and removals. `shared.rs` owns the
same typed tables and thread-local state as before. No runtime logic moves crates.

## Replay

Build the existing developer tool with `cargo build -p horizon-mod-api-gen --bin
part-table-layout`, then run `python3 tools/refactors/api-pr2b/move.py` on an unmoved
tree. It uses `git mv` and changes only imports, visibility, enclosing scopes and
paths. A repeated run rewrites paths only, including paths in `horizon-mod-runtime`
if it exists. This is required on `registry_keys.rs` if #1624 lands first. Do not
restore an old copy of that file over #1624's work.

The generator edits (removing the join and supporting directory module roots in
export assembly) are separate and small; apply those from the PR before building.
`tools/split_part_table.py` remains the historical inverse move, not a current tool.

## Proof

Save the base build's `part_table_generated.rs` outside Cargo's build output before
rebuilding. Then:

```
python3 tools/refactors/api-pr2b/compare_items.py SAVED_PART_TABLE --list
python3 tools/refactors/api-pr2b/moved_lines_check.py BASE
python3 tools/refactors/api-pr1/gate.py BASE
```

`compare_items.py` compares normalized signatures and bodies of all 601 items,
including tests, and lists their destination modules. `moved_lines_check.py`
compares a multiset of 6,339 canonical lines from the replayed fragments and
modules. The earlier 595-item / 6,104-line report omitted six functions containing
local `use` declarations; the corrected checker distinguishes actual imports
through the syn layout and covers those functions too. `test_proofs.py` pins that
property by perturbing a function body with a local import.
Canonical formatting occurs after the declared path/visibility edits so wrapping
does not manufacture a difference. Both omit use/mod/trait/impl envelope lines;
comments are excluded from the moved-line check and trait-member rustdoc remains
in the item comparison. The only identifier change is the local variable
`installed` to `installed_parts` in two block test closures, to disambiguate real
module imports. No bodies, constants, conditions or dispatch logic change.

The PR 1 gate compares WIT, hook dispatch and the host manifest. Also compare
`mod-api.wit` directly: for this move it is byte-identical, without reordering.

Focused local checks: `cargo test -p horizon-mod-api --lib parts::`,
`cargo test -p horizon-mod-api-gen exports::tests`, and
`python3 -m unittest tests.test_mod_api_boundary`. A development build with
`cargo build -p horizon-mod-api --features testing` checks the non-unit fixture
configuration too; test-only imports carry `#[cfg(test)]`, while public fixture
exports retain their original visibility. Workspace checks and wasm32 are
left to CI under the room's no-local-CI rule. No in-game trigger applies to a
layout-only move.
