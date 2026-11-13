# Small re-exports and recipe JSON text

## Changes

- KeyConflictContext and FireworkShape are API-owned enums, retaining every case and its order. Key registration and recipe snapshots convert in the game-owned small-values boundary.
- Unresolved-fluid name admission no longer calls the package crate's identifier validator. The same vanilla-shaped namespace/path samples pin admission, including slash only in the path.
- ReloadStageError retains an API-owned JarInputsError. Missing files retain ordered owners, original id/version/hash and expected path. I/O and invalid-record failures retain a distinct case, path and owned diagnostic rather than carrying package, filesystem or serde error objects across the API.
- Recipe ingredient and result helpers take JSON text. Invalid syntax is a typed RecipeJsonError; unknown items/empty tags remain absent. Result objects retain their components. Bare result identifiers are qualified like ingredient identifiers.

## Reproducible focused proof

`python3 tools/refactors/api-pr3/check_small_value_mutants.py` compiles actual production declarations with their case-order tests using rustc, then compiles temporary mutated declarations. Baselines pass. Changing InGame's implicit index, changing Star's implicit index, and refusing slash in an identifier path each fail their corresponding regression. This does not substitute for compilation of the full game boundary.

`python3 -m unittest tests.test_mod_api_boundary tests.test_mod_port_harness tests.test_version_port`: 22 tests passed.

`cargo test -p horizon-mod-api case_order`: 2 passed (both new enum-order tests).
`cargo test -p horizon-mod-api namespaced_identifier_samples`: 1 passed.

`cargo check -p horizon-client --tests --examples` completed with inherited Text ownership errors in native_unspawned_mob, add_attribute_tooltips and a stations test. It also found two new test-only Arc/str comparisons, corrected in a follow-up commit. No second game check or successful game-test run is claimed after that correction. No new production recipe, enum or jar diagnostic implementation produced a compiler diagnostic in that run. Full-workspace compilation and Clippy were not run; the known Text failures block the game, and the room leaves Clippy to CI.

Resource-backed recipe tests and jar diagnostic conversion tests are in `api_values/small_values.rs`. The recipe test requires prepared game resources and is explicitly ignored by default. No successful run is claimed until recorded by the worker result.

## Generated interface comparison

Generated with api-layout, from source snapshots of origin/main and origin/api-lead-api3, against the working head. The host-manifest binary reads the generated host unions and every source area file. Exact recipe exceptions are in recipes-manifest-exceptions.json; manifest_gate.py passes against origin/api-lead-api3. The two exceptions are Host::recipe_ingredient and Host::recipe_result; no method was added, removed, renamed or moved.

Relative to **origin/api-lead-api3**:

- WIT adds only `recipe-json-error`; no existing type changes or removals.
- Only `host/recipe-ingredient` and `host/recipe-result` change signature.
- Every existing case position is unchanged across 199 common definitions.
- JarInputsError is an internal reload diagnostic, not a WIT import type.

Relative to **origin/main**, the integration base also contributes earlier work:

- Added types: `level-read-error`, `recipe-json-error`, `text-node`, `text-relation`.
- Changed types: `placement-context`, `text-form`.
- Removed type: `geometry-vec3`.
- Changed functions: `backend/level-game-rule`, `backend/level-game-rules`, `host/recipe-ingredient`, `host/recipe-result`.
- Every existing case position is unchanged across 197 common definitions.

Commands: report_wit.py and tools/ci/wit_report.py on each generated mod-api.wit pair; manifest_gate.py on the before/after host manifests with the exact exceptions file.

## Remaining ownership edges

ParticleType is still engine-owned and directly shared by the renderer's ParticleOptions; converting this is a particle-family boundary change. TooltipDisplay still borrows an engine stack's decoded component data and is used by the normal tooltip renderer. It needs an owned snapshot at subscribed hooks while the unobserved tooltip path keeps its borrowed engine view. Neither was disguised by an alias or an unconditional hot-path clone here.

ItemBar, ItemRarity and health belong to item-resource-damage. Structure, dimension and generator types belong to the structure item; block-state ids to 3b. Crafting custom data and structure config were deliberately not changed, pending the shared Tag boundary.

## Performance

This work adds no tick dispatch or subscriber scan. Small enum conversion allocates zero. Jar diagnostic copies occur only on failed reload. JSON is parsed on explicit recipe helper calls, which are intended for file/reload processing, never implicitly per tick. No whole-game timing or allocation benchmark is claimed.
