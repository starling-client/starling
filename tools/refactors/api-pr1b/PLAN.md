# API PR 1b

Base: ba916ffc30d3b67eafb7124ebad78308d3f6bb91 (main after #1578).

1. Copy the curated table (since removed from the tree: it was a shared list that every host-method PR edited; the merged commit keeps it); explicitly assign the 13 methods added since its snapshot.
2. Write and run the regroup script, including game forwarders, doubles, macros, paths and imports. Open a draft against main early.
3. Prove method bytes and moved-line equivalence; run the WIT/manifest gate and focused tests. CI owns workspace checks.
4. Separate follow-up commits: reject unknown host traits, validate gate scratch worktrees, fix compile-fail imports.
5. Update the draft body with proofs, decisions and branch replay instructions; report PR and head.
