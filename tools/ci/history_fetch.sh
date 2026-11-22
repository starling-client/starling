#!/usr/bin/env bash
# Unshallow this checkout's HEAD for the gate's history checks (public-tree ranges, sync ancestry, selection): two
# tries of at most 300 s each, then fail closed naming $FETCH_STEP.
#
# On a Dyna runner a bare mirror of the repository lends its objects to the checkout as an alternate, so GitHub sends
# only what the mirror lacks (a full unshallow was about 146 s); the mirror takes this head afterwards. A mirror that
# is missing or broken costs nothing but the saving: the fetch goes on without it.
#
# The mirror lives in $DYNA_CI_KEEP, the host's folder for this repository, shared by every slot and every concurrent
# job (a slot's own $DYNA_CI_BUILD is emptied whenever the slot switches repositories, which left a mirror there cold
# on about half the gate's jobs). Because other jobs' checkouts borrow from it while this one updates it:
# - each borrower pins what it borrows: a ref refs/borrowed/<time>-<job> at the head it negotiates against (the
#   checkout lists only that ref of the mirror: core.alternateRefsPrefixes), so a later, divergent head can't leave
#   those objects unreachable; pins older than PIN_DAYS are deleted by the next update;
# - one job at a time updates it (an exclusive lock taken without waiting; a job that can't take it skips the
#   update), with Git's automatic maintenance off;
# - its repack runs under a second lock that a borrower holds shared while it reads the head and pins it, so nothing
#   is pruned between the two;
# - no job deletes it.
# Where only $DYNA_CI_BUILD is given, the slot keeps a mirror of its own, which one job at a time uses: no pins or
# locks, and a broken one is dropped.
#
# HISTORY_FILTER (a `git fetch --filter` spec) fetches less than the whole history, for a step that reads only part of
# it: tree:0 brings the commits alone, and Git fetches a tree from GitHub when a command first reads it. Such a
# checkout neither borrows from a mirror nor fills it, since the mirror must hold every object.
set -uo pipefail
step=${FETCH_STEP:?FETCH_STEP names the step in errors}
# Seconds from this script's start that the mirror's upkeep may use: the steps' timeout-minutes is 11.
UPKEEP_BUDGET=${UPKEEP_BUDGET:-600}
# How long a borrower's pin keeps its objects: longer than any job lives.
PIN_DAYS=${PIN_DAYS:-2}
[ "$(git rev-parse --is-shallow-repository)" = false ] && exit 0
head=$(git rev-parse HEAD)
alternates=$(git rev-parse --git-path objects/info/alternates)
filter=${HISTORY_FILTER:-}
mirror= borrowed= shared=
if [ -n "${DYNA_CI_KEEP:-}" ] && [ -z "$filter" ]; then
  mirror="$DYNA_CI_KEEP/git-mirror/${GITHUB_REPOSITORY:?}.git" shared=1
elif [ -n "${DYNA_CI_BUILD:-}" ] && [ -z "$filter" ]; then
  mirror="$DYNA_CI_BUILD/git-mirror/${GITHUB_REPOSITORY:?}.git"
fi

# Pin the mirror's head for this checkout, holding the repack lock shared so no repack runs in between.
pin() {
  local tip status=1
  exec 8> "$mirror.repack.lock" || return 1
  # The two fetch attempts below need 620 s of the step's 660: a borrower waits for a repack only briefly.
  if flock -s -w 20 8; then
    tip=$(git -C "$mirror" rev-parse -q --verify refs/heads/ci-head) &&
      pinned="refs/borrowed/$(date +%s)-${GITHUB_RUN_ID:-0}-${GITHUB_RUN_ATTEMPT:-0}-$$" &&
      git -C "$mirror" update-ref "$pinned" "$tip"
    status=$?
  fi
  exec 8>&-
  return "$status"
}

if [ -n "$mirror" ] && git -C "$mirror" rev-parse -q --verify refs/heads/ci-head >/dev/null 2>&1; then
  if [ -z "$shared" ]; then
    echo "$mirror/objects" >> "$alternates"
    borrowed=1
  elif pin 2>/dev/null && git config core.alternateRefsPrefixes "$pinned"; then
    # Only with the prefix set: the fetch must negotiate against the pinned head alone.
    echo "$mirror/objects" >> "$alternates"
    borrowed=1
  else
    echo "$step: the history mirror could not be pinned; fetching without it."
  fi
fi

# Keep this head in the mirror for the next job; a failure only loses the saving.
update_mirror() {
  local left=$((UPKEEP_BUDGET - SECONDS)) status
  if [ "$left" -lt 15 ]; then
    echo "$step: no time left to update the history mirror."
    return
  fi
  git init -q --bare "$mirror" &&
    timeout --kill-after=5s "${left}s" git -C "$mirror" -c gc.auto=0 -c maintenance.auto=false fetch -q --no-tags \
      "$(git rev-parse --show-toplevel)" "+$head:refs/heads/ci-head"
  status=$?
  if [ "$status" -eq 124 ] || [ "$status" -eq 137 ]; then
    echo "$step: out of time to update the history mirror; the next job takes it from where it is."
    return
  elif [ "$status" -ne 0 ]; then
    echo "$step: the history mirror could not take this head."
    # A checkout borrowing from a shared mirror may be another job's: only a slot's own is dropped.
    [ -n "$borrowed$shared" ] || rm -rf "$mirror"
    return
  fi
  if [ -n "$shared" ]; then
    local cutoff=$(($(date +%s) - PIN_DAYS * 86400)) ref
    git -C "$mirror" for-each-ref --format='%(refname)' refs/borrowed/ | while read -r ref; do
      local made=${ref#refs/borrowed/}
      [ "${made%%-*}" -lt "$cutoff" ] 2>/dev/null && git -C "$mirror" update-ref -d "$ref"
    done
    left=$((UPKEEP_BUDGET - SECONDS))
    if [ "$left" -le 0 ]; then
      echo "$step: the history mirror was not repacked this time."
      return
    fi
    exec 8> "$mirror.repack.lock" && flock -x -w "$((left < 60 ? left : 60))" 8 || {
      exec 8>&-
      echo "$step: the history mirror was not repacked this time."
      return
    }
  fi
  # A repack cut short leaves the mirror as it was: packs and refs are written whole, then renamed.
  left=$((UPKEEP_BUDGET - SECONDS))
  [ "$left" -lt 15 ] || timeout --kill-after=5s "${left}s" git -C "$mirror" -c gc.autoDetach=false gc --auto \
    --quiet || echo "$step: the history mirror was not repacked this time."
  [ -z "$shared" ] || exec 8>&-
}

for attempt in 1 2; do
  if timeout --kill-after=10s 300s git fetch -q --no-tags --unshallow ${filter:+"--filter=$filter"} origin "$head"; then
    # The upkeep gets only what is left of the step's 11 minutes, less a margin, so it never costs the job.
    if [ -n "$mirror" ] && [ -z "$shared" ]; then
      update_mirror
    elif [ -n "$mirror" ] && [ $((UPKEEP_BUDGET - SECONDS)) -lt 15 ]; then
      echo "$step: no time left to update the history mirror."
    elif [ -n "$mirror" ]; then
      if mkdir -p "$(dirname "$mirror")" && exec 9> "$mirror.lock"; then
        flock -n -E 75 9
        case $? in
          0) update_mirror ;;
          75) echo "$step: another job holds the history mirror's update lock; this one leaves the update to it." ;;
          *) echo "$step: the history mirror's update lock could not be taken; the mirror is not updated." ;;
        esac
        exec 9>&-
      else
        echo "$step: the history mirror's update lock could not be opened; the mirror is not updated."
      fi
    fi
    exit 0
  fi
  echo "$step: GitHub fetch attempt $attempt failed or timed out."
  if [ -n "$borrowed" ]; then
    # The objects fetched before the alternate was added stand alone. Retry without the mirror, in case it is what
    # broke; a slot's own mirror that then can't take the head is dropped above.
    echo "$step: retrying without the history mirror."
    sed -i "\\|^$mirror/objects\$|d" "$alternates"
    git config --unset core.alternateRefsPrefixes 2>/dev/null
    borrowed=
  fi
done
echo "::error::$step: history fetch from GitHub failed or timed out twice."
exit 1
