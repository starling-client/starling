#!/usr/bin/env bash
# Decides whether this run's heavy jobs use Dyna's self-hosted runners and prints
# self_hosted=true|false. The answer is false unless every condition holds, so a
# repository with no variables set keeps GitHub-hosted runners.
#
# ENABLED is the owner's kill switch. SAME_REPO is false for a fork's pull request.
# LEASE is a Unix time the pool renews while it has runners, so a dead pool sends
# new runs to GitHub-hosted runners instead of leaving them queued.
#
# This only routes jobs. A pull request can edit this file, so the check a fork
# cannot change lives in Dyna's runner image: job-started.sh.
set -euo pipefail

now=${NOW:-$(date +%s)}
reference=$now
reason="live lease checked against current time"
# A run queued behind main can outlive the short renewable lease snapshot. In
# snapshot mode trust only a lease valid at the snapshot's reference time (CREATED:
# the attempt's creation or start, from dyna_ci_route.sh), and never over 1 hour after it.
if [ "${LEASE_SOURCE:-live}" = snapshot ]; then
  reference=${CREATED:-}
  case "$reference" in
    '' | *[!0-9]*) reference=$now; reason="snapshot rejected: the snapshot's reference time is unavailable" ;;
    *) if [ "$reference" -gt "$now" ] || [ "$((now - reference))" -gt 3600 ]; then
         reference=$now
         reason="snapshot rejected: the snapshot's age is outside 0..3600 seconds"
       else
         reason="snapshot lease checked at its reference time; snapshot age=$((now - reference)) seconds (limit 3600)"
       fi ;;
  esac
fi
self_hosted=false
if [ "${ENABLED:-}" = true ] && [ "${SAME_REPO:-}" = true ] && { [ "${EVENT:-}" = push ] || [ "${EVENT:-}" = workflow_dispatch ] || [ "${EVENT:-}" = pull_request ]; }; then
  case "${LEASE:-}" in
    '' | *[!0-9]*) ;;
    *) if [ "$LEASE" -gt "$reference" ] && [[ "$reason" != snapshot\ rejected:* ]]; then self_hosted=true; fi ;;
  esac
fi
echo "self_hosted=$self_hosted"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "routing_reason=$reason; enabled=${ENABLED:-unset}; lease=${LEASE:-unset}; now=$now; reference=$reference" >> "$GITHUB_OUTPUT"
fi
