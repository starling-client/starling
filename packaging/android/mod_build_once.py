#!/usr/bin/env python3
"""One build of the mod service on a machine that lives for one build: GitHub's runner, in the
mod-builds repository's build.yml (docs/starling-native-mods.md). The mods are checked and compiled as
mod_build_server.py does, sandbox included, but without its count of mods: that one keeps a player
from filling the service's own small machine, and this runner is nobody else's.

    mod_build_once.py BODY COMMIT OUT

BODY is a request's body, COMMIT the game's. OUT gets status.json, what the service would answer for the
job, and on success the library. MOD_BUILD_ROOT holds base/, source/ and cargo/ as the service's does.
A build that fails for the mods' fault still exits 0: the failure is the answer, and it has to arrive.
"""
import json
import pathlib
import shutil
import subprocess
import sys

import mod_build_server as server

# As long as the sandbox itself is told to wait (MOD_BUILD_SECONDS), and a little more.
BUILD_SECONDS = 55 * 60


def failed(error):
    return {"state": "failed", "error": error, "settled": bool(server.SETTLED.search(error))}


def build(body, commit, out):
    work = server.ROOT / "jobs" / "once"
    shutil.rmtree(work, ignore_errors=True)
    try:
        # No count here: this runner builds one request and goes, so the body's size, what it
        # expands to and the clock below are its limits, not how many mods a player picked.
        files = sorted(server.split_body(body, most=None))
        server.unpack(files, work)
    except server.Refused as e:
        return failed(str(e))
    built = work / "out"
    built.mkdir()
    log = work / "build.log"
    try:
        with log.open("wb") as sink:
            code = subprocess.run(
                ["bash", str(server.SANDBOX), str(server.ROOT), commit, str(work)],
                stdout=sink,
                stderr=subprocess.STDOUT,
                timeout=BUILD_SECONDS,
            ).returncode
    except subprocess.TimeoutExpired:
        return failed("The build took too long.")
    print(log.read_text(errors="replace")[-20000:])
    if code != 0 or not (built / "build.json").exists():
        return failed(server.why_failed(log))
    about = json.loads((built / "build.json").read_text())
    shutil.move(built / "libhorizon_android.so", out / "libhorizon_android.so")
    return {"state": "done", "sha256": about["sha256"], "size": about["size"], "build": about}


def main():
    body, commit, out = sys.argv[1:]
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    status = build(pathlib.Path(body).read_bytes(), commit, out)
    (out / "status.json").write_text(json.dumps(status))
    print(json.dumps({k: v for k, v in status.items() if k != "build"}))


if __name__ == "__main__":
    main()
