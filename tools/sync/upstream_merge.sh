#!/usr/bin/env bash
# Run from a clean branch; unresolved code is intentionally left for area owners.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
target=${1:?usage: upstream_merge.sh upstream-sha}
git rev-parse --verify "$target^{commit}" >/dev/null
git config merge.horizon-lang.driver 'python3 tools/sync/merge_lang.py %O %A %B %L %P'
git config rerere.enabled true
git config rerere.autoupdate false
cache=${HORIZON_SYNC_RERERE:-/box/shared/upstream-sync-2/rerere}
mkdir -p "$cache"
rr="$(git rev-parse --git-common-dir)/rr-cache"
if [[ -d "$rr" && ! -L "$rr" ]]; then
    cp -a "$rr/." "$cache/"
    mv "$rr" "$rr.before-shared-$(date +%s)"
fi
[[ -L "$rr" ]] || ln -s "$cache" "$rr"
status=0
git merge --no-commit --no-ff "$target" || status=$?
if ((status > 1)); then exit "$status"; fi
python3 tools/sync/resolve_policies.py "$target"
# Generators may require resolved Rust sources. Never claim regeneration succeeded
# while those sources are still conflicted; rerun this step after area exports.
if [[ -z $(git diff --name-only --diff-filter=U) ]]; then
    while IFS=$'\t' read -r path command; do
        [[ -z "$path" || "$path" == \#* ]] && continue
        bash -c "$command"
        git add -- "$path"
    done < tools/sync/generated.txt
else
    echo 'Regeneration deferred until code conflicts are resolved.'
fi
