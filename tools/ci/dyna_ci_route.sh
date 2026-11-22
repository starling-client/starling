#!/usr/bin/env bash
# Prints self_hosted=true|false for this run's heavy jobs (tools/ci/dyna_ci_runner.sh decides), from the pool's
# switch and lease read live when the token may read Variables, else from the run's `vars` snapshot. A first
# attempt's snapshot is fixed when the run is created, so it is judged against that time, never against now: a run that
# waited in a concurrency group or GitHub's queue would otherwise see a lease that lapsed while it waited. A re-run
# (a later attempt) takes a new snapshot when it starts, so it is judged against that attempt's start: against the
# first attempt's creation, a gate re-run more than an hour later went to GitHub's runners (run 44472251704).
#
# Env: VARS_ENABLED and VARS_LEASE (the run's snapshot of DYNA_CI_SELF_HOSTED and DYNA_CI_POOL_UNTIL), EVENT,
# SAME_REPO, GH_TOKEN (actions: read, for the attempt's creation or start time), GITHUB_REPOSITORY, GITHUB_RUN_ID and
# GITHUB_RUN_ATTEMPT. A
# reusable workflow's GITHUB_RUN_ID is its caller's run, whose snapshot it shares.
set -uo pipefail
ENABLED=
LEASE=
if [ "${SAME_REPO:-}" = true ]; then
  ENABLED=$(gh api "repos/$GITHUB_REPOSITORY/actions/variables/DYNA_CI_SELF_HOSTED" -q .value 2>/dev/null || true)
  LEASE=$(gh api "repos/$GITHUB_REPOSITORY/actions/variables/DYNA_CI_POOL_UNTIL" -q .value 2>/dev/null || true)
fi
# A refused read prints GitHub's error JSON: keep only a well-formed answer, else use the snapshot.
case "$LEASE" in '' | *[!0-9]*) LEASE= ;; esac
case "$ENABLED" in true | false) ;; *) ENABLED= ;; esac
echo "pool lease: live=${LEASE:-unreadable} snapshot=${VARS_LEASE:-none}" >&2
SOURCE=live
CREATED=
if [ -z "$LEASE" ]; then
  SOURCE=snapshot
  # A run's run_started_at is its latest attempt's start; GitHub keeps created_at from the first attempt.
  [ "${GITHUB_RUN_ATTEMPT:-1}" -gt 1 ] 2>/dev/null && FIELD=.run_started_at || FIELD=.created_at
  CREATED_AT=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID" -q "$FIELD" 2>/dev/null || true)
  [ -z "$CREATED_AT" ] || CREATED=$(date -d "$CREATED_AT" +%s 2>/dev/null || true)
fi
ENABLED=${ENABLED:-${VARS_ENABLED:-}} LEASE=${LEASE:-${VARS_LEASE:-}} LEASE_SOURCE=$SOURCE CREATED=$CREATED \
  exec bash "$(dirname "${BASH_SOURCE[0]}")/dyna_ci_runner.sh"
