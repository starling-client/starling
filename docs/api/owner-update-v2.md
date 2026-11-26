# What the registry's `owner-update` workflow must check (trailer v2)

This is the `owner-update` contract for the old registry index. It does not apply to the new release route,
where a reviewer must approve every release pull request.

The submission service (`crates/sunrise-mod-registry`) opens owner updates in
[sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods). Every registry pull request, owner updates
included, is reviewed before it merges, and the review looks only for malware, dangerous patterns and suspicious or
obfuscated code in the release (never for licences, provenance or copyright). `update_trailer` signs the
`owner-update` trailer with `REGISTRY_UPDATE_KEY`. The trailer is an ownership check: it says who submitted the
update and that they own the project, and it never replaces the review. The checks below belong to the registry
repository's workflow (`scripts/owner_update.py`), not here, and every one must pass before the review's approval
can merge. Any check that fails leaves the pull request to a reviewer (comment why).

## The trailer

The last HTML comment of the pull request body:

```text
<!-- owner-update
format: v2
id: <project id>
version: <version>
sha256: <hex>
file: <id>-<version>.<extension>
submitter: <login>
submitter_id: <numeric GitHub ID>
hmac: <hex>
-->
```

`hmac` is HMAC-SHA256 with `REGISTRY_UPDATE_KEY`, lowercase hex, over exactly these bytes (`\n` between fields, no
trailing newline):

```text
owner-update-v2\n<id>\n<version>\n<sha256>\n<file>\n<submitter>\n<submitter_id>
```

## The checks

1. **Format.** `format: v2` is present and every field above is there once. Refuse `v1` trailers (no `format` line)
   once the service is deployed: a v1 trailer names the submitter by login only.
2. **HMAC.** Recompute it over the v2 message and compare in constant time (`hmac.compare_digest`).
3. **The submitter is the pull request's.** `submitter_id` equals the numeric ID in the pull request head commit's
   `Co-authored-by: <login> <<id>+<login>@users.noreply.github.com>` line, which the service writes. Compare the IDs,
   not the logins.
4. **The submitter owns the project, by ID.** Read `mods/<id>.toml` **on main** (not the pull request's copy):
   `submitter_id` must be in its `owner_ids`. An entry without `owner_ids` never merges on its own (the service
   doesn't sign those, so a trailer on one is forged or stale).
5. **Nothing else changes.** The pull request touches only `mods/<id>.toml`. Its new entry equals main's except
   for one added `[[versions]]` entry, first, whose `version`, `sha256` and `file` are the trailer's. `owners`,
   `owner_ids`, `key`, `kind`, `repo` and the page fields are identical (the service already only signs such updates;
   check again, since the pull request branch can be pushed to after it opened).
6. **The file.** `staging` holds `<sha256>.<extension>` and its SHA-256 is `sha256`. The registry
   repository's CI checks for the old index, including signature verification when the entry has a `key`,
   pass at the head that is merged.
7. **The head merged is the head checked.** Merge with the head SHA the checks ran on (`gh pr merge --match-head-commit
   <sha>`), so a push between check and merge can't slip in.

## Today

Every owner update waits for a reviewer's approval, whatever its trailer says. The checks above only confirm who
submitted the update and what it changes; the v2 trailer and `owner_ids` make the review's notes and the service's
refusals decide by ID.

## Order of deployment

1. In the registry repository: update `scripts/owner_update.py` and the workflow to the checks above, accepting v2
   only. Keep CI's pinned tool on a revision that supports both `owner_ids` and building the old index.
2. Once a maintainer has read `tools/registry_backfill_owner_ids.py`, run it on a checkout (dry run, then `--write`)
   and merge its pull request by hand.
3. Deploy the submission service with this change. Until step 1 is live, its v2 trailers fail verification and the
   updates simply wait for a reviewer.
