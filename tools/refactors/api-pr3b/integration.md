# API 3b / 3c integration

Inputs preserved as merges, without rebase:

- Block metadata/resource: `0092d5364dfa22d2629ba7e35b8012147e9418a7`.
- Initial API 3 chain: `169f900bba643ca6f99272d0502655348ec79437`.
- Corrected complete API 3 chain, including flat Text and rule fixes: `a81932005725c6368393f969ef490313305bebe7`.

Merge seams retain the immutable `Blocks` table and the particle table together. The production block-definition importer, its default/immutability regression and the ModBar converter remain independent items. The schema-aware test installation from main is retained.

Compiler-guided repairs:

1. The unspawned-mob game's getter returns engine Text, letting the existing generated forwarder use the single game-owned conversion. The edit resource still accepts API Text and converts through the existing TryIntoEngine implementation before mutating the lent mob.
2. Attribute-tooltip output converts the API lines back through the existing tuple/vector Text conversion at the game boundary. Its mod-side fixture creates API Text.
3. The narrow production-load probe extracts its two named functions rather than accidentally including a neighboring ModBar implementation between comment markers. It continues executing the production importer and load call; no loader or Text algorithm is copied.

Proof: `cargo check --workspace --tests --examples` passed on the complete combined tree after the repairs. No workspace tests or clippy suite were run. The initial check against the incomplete chain was stopped when the corrected input arrived; the first corrected-tree check exposed Text seam errors and was not counted as passing. Existing API future-incompatibility warning remains.

Focused runtime execution passed on this integration tree:

- Completed block keys: 1 test.
- Runtime immutable definitions/refusals/worker allocations: 3 tests.
- Table-backed metadata service and borrowed backend result: 1 test.
- Explicit worker metadata and zero-allocation read loop: 1 test.
- Flat Text forms, malformed trees and depth/order/style: 7 tests.
- Verify-discovered Python load/report tests: 2 tests, including 2 compiled production importer/load cases.
- Actual game Text capture/encoding: 11 tests.
- Actual ignored game rule getter unknown/stale/unavailable/value regression: 1 test, explicitly enabled and executed using prepared resources.

The game test executable ran from a fresh scratch working directory with isolated Horizon, launcher and XDG homes and audio disabled. No tests above were merely discovered, ignored or compiled without execution. Logs are local development outputs, not public artifacts.

No in-game launch or rendering proof is claimed. These are API merge seams, not new firing points, rendering behavior or network behavior.
