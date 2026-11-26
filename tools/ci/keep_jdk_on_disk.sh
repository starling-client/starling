#!/usr/bin/env bash
# Keeps actions/setup-java's JDK out of a Dyna runner's memory. There $RUNNER_TEMP and $RUNNER_TOOL_CACHE are tmpfs
# counted against the job's memory: setup-java leaves its download and its extraction in RUNNER_TEMP and installs a
# copy into the tool cache.
#   keep_jdk_on_disk.sh before LISTING   (before setup-java) records `ls -1 "$RUNNER_TEMP"` in LISTING, and links the
#                                        JDK an earlier job of this slot kept in $DYNA_CI_BUILD/tool-cache into the
#                                        tool cache, where setup-java finds it and downloads nothing.
#   keep_jdk_on_disk.sh after LISTING    (after setup-java) deletes the entries setup-java added to RUNNER_TEMP (named
#                                        by a UUID, as its downloads and extractions are), and moves a JDK it installed
#                                        to $DYNA_CI_BUILD/tool-cache behind a symlink, so the JAVA_HOME and PATH it
#                                        exported stay valid.
set -euo pipefail
mode=$1 listing=$2
disk="${DYNA_CI_BUILD:?}/tool-cache"
mkdir -p "$disk" "$RUNNER_TOOL_CACHE"
case $mode in
  before)
    ls -1 "$RUNNER_TEMP" > "$listing"
    for kept in "$disk"/Java_*; do
      if [ -d "$kept" ] && [ ! -e "$RUNNER_TOOL_CACHE/${kept##*/}" ]; then
        ln -s -- "$kept" "$RUNNER_TOOL_CACHE/${kept##*/}"
      fi
    done
    ;;
  after)
    uuid='^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
    { comm -13 <(sort "$listing") <(ls -1 "$RUNNER_TEMP" | sort) | grep -E "$uuid" || true; } | while read -r entry; do
      rm -rf -- "${RUNNER_TEMP:?}/$entry"
    done
    for tool in "$RUNNER_TOOL_CACHE"/Java_*; do
      if [ -L "$tool" ] || [ ! -d "$tool" ]; then
        continue
      fi
      # Copied under a temporary name and renamed into place, so a job stopped mid-copy never leaves a partial JDK
      # for the next job to link.
      name=${tool##*/}
      rm -rf -- "${disk:?}/.incoming-$name"
      mv -- "$tool" "$disk/.incoming-$name"
      rm -rf -- "${disk:?}/$name"
      mv -- "$disk/.incoming-$name" "$disk/$name"
      ln -s -- "$disk/$name" "$tool"
    done
    ;;
  *)
    echo "usage: keep_jdk_on_disk.sh before|after LISTING" >&2
    exit 2
    ;;
esac
