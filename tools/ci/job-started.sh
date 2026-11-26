#!/usr/bin/env bash
# ACTIONS_RUNNER_HOOK_JOB_STARTED of the checks runners (tools/ci/checks-runners.sh): runs before each job's steps;
# a failure fails the job. A fork's pull request runs its own copy of the workflows, so its runs-on can name these
# runners whatever ci.yml says: such a job stops here, before any of its code is on disk.
set -euo pipefail
case ${GITHUB_EVENT_NAME:-} in
  pull_request|pull_request_review|pull_request_review_comment|merge_group)
    head=$(jq -r '.pull_request.head.repo.full_name // "unknown"' "${GITHUB_EVENT_PATH:-/dev/null}" 2>/dev/null || echo unknown)
    if [ "$head" != "${GITHUB_REPOSITORY:-}" ]; then
      echo "::error::A pull request from $head never runs on a self-hosted runner: use runs-on ubuntu-latest for forks"
      exit 1
    fi;;
esac
if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -d "$CARGO_TARGET_DIR" ]; then
  # A failed size check is housekeeping, not a failed job; leave the folder alone.
  size=$(du -s --block-size=1G "$CARGO_TARGET_DIR" | cut -f1) || size=0
  if [ "$size" -gt "${MAX_TARGET_GB:-40}" ]; then
    echo "Cargo target folder at ${size} GB (over ${MAX_TARGET_GB:-40}): emptied, this job builds from scratch"
    rm -rf "${CARGO_TARGET_DIR:?}"
  fi
fi
# The checkout's history: actions/checkout fetches into the same .git every job and never repacks. git-slim.sh (beside
# this hook) collects it past 1.5 GB (a healthy one is ~0.5 GB) and folds many packs; one that is still over MAX_GIT_GB
# is removed: the checkout then clones again (and replaces the rest of the folder).
# A git-slim.sh failure does not fail the job: it is disk housekeeping (f9d90fcb534), and the size check below is what
# bounds the folder when it did not shrink (it removes .git over MAX_GIT_GB), so a failed collect costs a reclone, not a job.
# The same holds for the size check itself: a du that fails leaves the folder as it is (size 0), not a failed job.
if [ -n "${GITHUB_WORKSPACE:-}" ] && [ -d "$GITHUB_WORKSPACE/.git" ]; then
  MAX_GIT_MB=${MAX_GIT_MB:-1536} bash "$(dirname "$0")/git-slim.sh" "$GITHUB_WORKSPACE" || true
  size=$(du -s --block-size=1G "$GITHUB_WORKSPACE/.git" | cut -f1) || size=0
  if [ "$size" -gt "${MAX_GIT_GB:-4}" ]; then
    echo "Checkout's .git at ${size} GB (over ${MAX_GIT_GB:-4}): removed, this job clones again"
    rm -rf "${GITHUB_WORKSPACE:?}/.git"
  fi
fi
# The runner's own diagnostic logs (a file per job, 1.3 GB a light runner in an hour on 2026-11-03): a day's worth.
# Housekeeping that never fails a job (b8a348f56da): the runner writes and rotates these files while find walks them, so
# a vanished or unreadable log is an expected find error, and the job has no use for the result.
if [ -n "${RUNNER_WORKSPACE:-}" ] && [ -d "$RUNNER_WORKSPACE/../../_diag" ]; then
  find "$RUNNER_WORKSPACE/../../_diag" -maxdepth 1 -type f -name '*.log' -mmin +1440 -delete 2>/dev/null || true
fi
