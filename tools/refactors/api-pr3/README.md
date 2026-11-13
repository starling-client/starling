# API 3 closeout: replay order

This directory holds the scripts and gate tooling for the one PR that closes API 3
(`horizon-mod-api` depends on no engine crate). Commits in the PR are grouped by item, so review
can split by commit group:

1. Hand-written items (one commit series each): flat `Text` follow-ups, JSON to `Tag` converter,
   structure and chunk-generator contexts, `ItemResource` and damage records, 3b's `BlockState`
   layers, the loot consumer cut and the typed `Text` node.
2. Scripted cuts, applied last, each as its own commit (`3b-swap`, `3c-cut`, `3e-cut`): signature
   swaps, import rewrites and the engine dependency removal, with a rewrite log, a moved-lines check
   and the WIT and host-manifest diffs.
3. The boundary check (`tools/ci/check_api_boundary.py`).

Each script is re-run, never hand-edited, after any change to its inputs.
