# API PR 2c scripts

Part of #1608. Initial base: `cb01ae304c7136f1598f97650c63ccfc1da47b90`.
The first main merge brought in `57f0b36960`. The paused-main replay used `5d27928c5ce0ca8c041b0856ef56bfe5430bb6fc`.
The authorized follow-up merge includes sync #1660; final comparisons now use
main `658fb76daddef2101540d4c716baa09a2c6595ca`.

## Generated boundary

An area trait carries `#[horizon_mod_api_macros::host_area]`. Its methods declare
`#[access(WorldOnly)]`, `#[access(Metadata)]`, `#[access(ClientSetup)]` or
`#[access(PublishedResources)]` and have no body. The proc macro emits their
panicking test-double defaults. `#[static_call]` marks the two inherent statics;
`#[service]` marks the six service accessors returning `self`.

The generator reads only the API area declarations to emit the game's impls.
`game_host_modules.rs` in OUT_DIR contains explicit methods, access checks and
`Game::name(self, args)` calls. It retains ordinary source paths in diagnostics.
No generated Rust is committed. The six service accessors are generated as
check-then-self; draw contexts and service implementations remain hand-written.

The generator test `game_provides_every_area_method_explicitly` compares all
parsed declarations with all generated explicit impl methods. Each generated
impl also denies unconditional recursion: Rust places the current impl's trait
in scope even without importing its prelude, so a missing inherent method would
otherwise resolve recursively through its forwarder. A missing implementation
therefore fails compilation instead of silently inheriting a default.

## Replay order on the initial base

These scripts describe one refactor, not a general migration framework. Do not
run the entire replay on an arbitrary newer branch. The TSVs and compiler span
log are the declared rename edits.

1. Build/run the compiler parser `audit_game_methods` and run `rename.py` without
   `--apply` to audit targets and collisions. This initially reports 26 inherent
   collisions and one public `AttachmentHost::attachments` collision.
2. `internal_names.py` writes `internal-renames.tsv` and changes only the 26
   inherent definitions to descriptive names. The public attachment trait stays
   intact: it is not game-private.
3. Run `cargo check --workspace --tests --examples --message-format=json` and feed
   its JSON output to `rewrite_error_spans.py`; repeat. Definitions were renamed
   first, then 243 initial caller identifiers were rewritten at rustc spans.
4. `declare_access.py` emits the area declaration edits and Metadata phase edit.
5. `rename.py --apply` emits the target table and target renames, removing the
   wholly generated game area files. `mixed_areas.py` extracts the area impls
   sharing files with draw contexts and services, leaving the latter intact.
6. `preserve_other_receivers.py` and `preserve_nonmethods.py` retain homonymous
   free functions, GameLevel methods, fields and modules. Hook catalog docs stay
   unchanged. This explicitly excludes `hunger::native_living_heal`, GameLevel's
   random/block-property methods, the `native_mod_screen` field and the
   `native_living_effects` module. Follow with compiler-guided span rewrites.
7. Format each touched Rust file with rustfmt, edition 2024, skip_children=true.
8. Remove only the unused prelude imports diagnosed by rustc. The general
   `drop_unused_imports.py` can apply fix-its, but rustc supplies none for these
   globs; `drop_unused_preludes.py CHECK_JSON` handles their primary spans and
   records the exact removed import/attribute blocks in `unused-import-edits.json`.
   The moved-lines proof lists and accounts for those seven declared removals.
   `add_method.py` retains and uses the DESCRIPTION data shared by the regroup tool.

After the one main merge, its new declaration/signature changes were retained
and `declare_access.py` was rerun on the four affected areas. `merge_targets.py`
converts the three additional, collision-free Game targets. Missing old-name
calls are again rewritten from compiler spans; `merge_error_receivers.py`
handles five newly merged internal callers whose type errors point at a returned
field or argument rather than at the method identifier. No wrappers are merged.

For the final paused-main replay, `rename.py --base origin/main --apply` reads the
old forwarders from the main tree, retains already-converted targets and extends
the declared table with the two additions. The merged `open_chat` API target
collides with the existing screen-initialisation method, so the latter becomes
`begin_chat_screen` first; its callers are compiler-span rewrites, not a blanket
rename. The target table now has 683 rows (including #1660's `level_world_border`)
and the internal table has 27 rows.

The handwritten changes are the generator, proc macro, phase policy, docs,
contributor helper and proof scripts. The declarations, target renames, internal
renames and caller edits are script outputs. `compiler-edits.jsonl` records the
caller byte spans, old identifiers and replacements in application order.

## Independent proofs

- `python3 tools/refactors/api-pr1/gate.py BASE` compares the WIT and host manifest,
  including signatures, rustdoc and hidden flags. No manifest-tool changes were
  needed. The gate now selects OUT_DIR from Cargo's build-script JSON event, not
  the directory creation time (which selected stale output after the main merge).
  The initial run moved zero WIT items: its bytes were identical, not just
  equivalent after the gate's name-set ordering.
- `python3 tools/refactors/api-pr2c/compare_access.py BASE [GENERATED_FILE]` reads
  the full old forwarder table with `git show`, normalises Metadata and declared
  target renames, and compares access levels, call targets, receiver placement
  and every argument against the actual generated file. Full tables are written
  to `access-expected.json` and `access-actual.json` beside that generated file.
- `python3 tools/refactors/api-pr2c/moved_lines_check.py BASE` prints both declared
  rename tables and compares the line multisets of every rename-only Rust file,
  with declared identifiers and rustfmt wrapping canonicalised. It does not claim
  the handwritten generator, macro, phase policy or changed declarations are pure
  moves. Generated forwarders are covered by the independent access/call proof.

All build/proof output belongs under `target/`, never in this directory.
