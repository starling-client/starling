#!/usr/bin/env python3
"""List launcher restoration differences for a human fork-preservation audit.

This is a read-only mechanical inventory, not a verdict that a fork hunk is safe
or that a whole-file take is wrong. Review the listed commits and differences.
"""
import argparse
import json
import subprocess


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def blob(ref, path):
    result = subprocess.run(["git", "rev-parse", "--verify", f"{ref}:{path}"],
                            capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/main")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--upstream-base", required=True)
    args = parser.parse_args()
    paths = git("diff", "--name-only", f"{args.base}...{args.head}", "--", "launcher/").splitlines()
    rows = []
    for path in paths:
        head, upstream = blob(args.head, path), blob(args.upstream, path)
        base, old = blob(args.base, path), blob(args.upstream_base, path)
        rows.append({
            "path": path,
            "whole_upstream_take": head is not None and head == upstream,
            "fork_changed_since_upstream_base": base != old,
            "fork_commits": git("log", "--format=%h %s", f"{args.upstream}..{args.base}", "--", path).splitlines(),
        })
    print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
