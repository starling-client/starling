# Upstream integration checks

## Upstream changes the merge dropped

A conflict resolved to the fork's side drops upstream's change silently: the sync builds, and the fork later looks
like it reverted upstream's work. Every sync runs, on its merge commit, before the senior read:

    python3 tools/sync/dropped_upstream.py <merge commit> --markdown

It lists each upstream commit with a file whose upstream-added lines are missing from the merge (kept fork side,
partly missing, no fork file, an upstream deletion undone, a binary that differs), and every file it could only find
by guessing where the fork moved it. The list over-reports, and it can miss a change that only removes or reorders
lines or whose lines already exist elsewhere in the file: sort each commit by hand
(`git show`, `git grep` at the merge) into carried, rightly kept fork side (say why), or lost, re-apply every lost
one in the sync, and put the table with its "Sorted as" column filled in the sync PR body.
