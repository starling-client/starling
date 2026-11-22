# Modularizing `gfx.rs`

`crates/sunrise-client-render/src/gfx.rs` held the renderer's construction, frame planning, passes and types in one 13,837-line file at `d1f3c86`. Subsystems move out of it into `crates/sunrise-client-render/src/gfx/`, which already held cull, occlusion, translucency, timing and the pack and native passes, in move-only steps. This note records the steps taken, the rules that keep each step reviewable, and what remains.

## Rules

- One extraction per pull request. A stacked branch carries the earlier commits unchanged; it is not a second extraction.
- Move only: no logic change, no renaming, no signature rewrite, no reformatting of the moved code. The diff is `gfx.rs` losing one block, the new module, the `mod` line, and at most `pub(super)` plus a `pub use` re-export where a sibling reads the item.
- Paths through `gfx.rs` stay unchanged: a moved type is re-exported (`pub use module::Type;`) instead of rewriting the crate's imports.
- A moved method becomes `pub(super)` only when `gfx.rs` or a sibling calls it.
- The gfx validation run must end in `ALL_CHECKS_PASSED`: rustfmt over `gfx.rs` and `gfx/*.rs`, `cargo check --lib --tests`, `tools/check_layers.py`, `tools/check_public_tree.py`.
- A block that moves with `include_str!` takes one path fix with it, because the literal now resolves next to the new file (`../screen.wgsl`). That is not a logic change.

## Phases

Phases 3 to 6 are the unmerged stack under PR #2406 at the time of writing; each phase also has its own pull request. `gfx.rs` counts are lines in the commit that ends the phase.

| Phase | Commit | Pull request | Moved out | `gfx.rs` after |
|---|---|---|---|---:|
| 3 | `b8f0c8b` | #2222 | `section_upload` (900 lines) | 12,942 |
| 3, entities | `5718774` | #2310 | `entity_draw` (261) | 12,694 |
| 4 | `9923bb7` | #2327 | `culling_control` (255), `hdr_config` (67), `surface_lifecycle` (176) | 12,215 |
| 5 | `f3fbf06` | #2330 | `draw_frame` (3,423) | 8,034 |
| 5, frame passes | `5eae255`, `53b464c` | #2330 | `frame::{acquire, post, submit, shadow, sky, water_mirror}` (1,135) | 8,034 |
| 6 | `0cb0575` | #2406 | `frame_data` (206), `section_state` (247), `prepared_textures` (193) | 7,404 |
| 7 | phase 7 commits | this note's pull request | `renderer_construction` (2,488) | 4,922 |

The two zero-delta phases split code that had already left `gfx.rs`: `draw_frame` became `frame/` passes, then `frame/acquire`, `frame/post` and `frame/submit`.

Phase 7 moves `Renderer::new` (2,486 lines: device and surface opening, every pipeline and buffer, the `previous` renderer's reuse) into `gfx/renderer_construction.rs` as its own `impl Renderer`. `Renderer::new` keeps its path and signature. The module pulls the parent's names with `use super::*;`; its one `include_str!("screen.wgsl")` becomes `../screen.wgsl`.

## What is left

Measured on the phase 7 branch, `gfx.rs` at 4,922 lines:

| Block | Lines | Status |
|---|---:|---|
| Remaining `impl Renderer` methods (1310-2417) | 1,108 | not one module: shader-pack wiring, lightmap and animation, view and frustum helpers, screenshots |
| `mod tests`, `mod shadow_face_tests`, `mod shadow_caster_tests` (2995-4619, 4625-4677, 4680-4897) | 1,896 | claimed by #2185, which conflicts |
| `pub struct Renderer` (605-1129) | 525 | unclaimed; 291 fields |
| `draw_overlay` (2418-2796) | 379 | claimed by #2308 |
| Shader-pack methods (part of the 1,108) | ~480 | claimed by #2306 |
| `outline_*`, `GpuChecks`, screenshot helpers | ~150 | claimed by #2186 |

`gfx.rs` was 13,837 lines; the stack plus phase 7 removes 8,915 of them (64%).

## Open work that overlaps

Every pull request below edits `gfx.rs`, so they conflict mechanically with each other and with any new extraction. #2186 (nine low-level modules) and #2206 (`Frame`) predate the stack and are not part of it. #2222, #2310, #2327 and #2330 duplicate commits that are already in #2406. The branch base is also behind `origin/main`, which advances by thousands of commits a day through rollups; #2406 is still textually mergeable.

## Known state of the stack

The stack's `verify` jobs were red for two reasons that the phase 7 branch fixes first, in their own commits:

- `0877c58` drops the unused `prepared_textures` glob import and the needless borrows around the command encoder in the shadow and water-mirror passes (clippy `-D warnings`).
- `6830007` teaches `tools/test_entity_shadow_ownership.py` to look up `native_shadow` and `pack_entity_shadows` in `gfx.rs` and its submodules; the phase 4 extraction moved them to `gfx/culling_control.rs` and the tool only read `gfx.rs`.

Both are outside the move-only perimeter but were required for the stack's own checks; neither changes renderer behavior.

## Options considered and deferred

- `pub struct Renderer` into a module of its own: 525 lines for 291 field declarations. Every field a sibling module reads would need `pub(super)`, so the diff would touch every open `gfx.rs` branch at once. Deferred until it can be reviewed alone.
- The test modules: 1,896 lines, but #2185 already proposes that extraction and is marked conflicting, so redoing it would restart a phase.
- `draw_overlay` (379) and the shader-pack methods (~480): already extracted in #2308 and #2306.
- Splitting `renderer_construction.rs` by construction stage: it needs functions pulled out of the constructor, which changes code, not just its location.

## Next

Candidates, in the order this note would take them once the open extractions land: the remaining `impl Renderer` groups by domain (views and frusta, lightmap and animation, screenshots, about 1,108 lines together and no single coherent module on its own), then `pub struct Renderer` (525 lines) when the field-visibility churn can be reviewed without competing branches, then the tests once #2185 is resolved.
