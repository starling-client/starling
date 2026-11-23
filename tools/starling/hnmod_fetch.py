"""The website's native mods for the compatibility check (.github/workflows/hnmod-compat.yml).

    hnmod_fetch.py OUT            each native mod's newest version from the registry index, its hash checked, the
                                  mixins/ folder of its .hnmod/.smod unpacked to OUT/<id>/, and OUT/_list.tsv
                                  (id, version, ok | revoked | package-fail, the Sunrise commit it names)
    hnmod_fetch.py --shards N OUT  after a fetch: the ids that are ok, split in N lists, as JSON for a job matrix

Only reads and writes files: no mod code runs here (the mods compile later, on a runner with no secrets).
It was the build VM's ~/compat/compat_fetch.py (sync 6).
"""

import hashlib
import io
import json
import pathlib
import re
import shutil
import sys
import urllib.request
import zipfile

INDEX = "https://horizon-rust.github.io/horizon-mods/index.json"


def fetch(root: pathlib.Path) -> None:
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    index = json.load(urllib.request.urlopen(INDEX, timeout=30))
    revoked = {r.get("id") if isinstance(r, dict) else r for r in index.get("revoked") or []}
    rows = []
    for project in sorted(index["projects"], key=lambda p: p["id"]):
        if project["kind"] != "native-mod":
            continue
        newest = project["versions"][0]
        mod = project["id"]
        if not re.fullmatch(r"[A-Za-z0-9_.-]+", mod):
            print("bad-id", mod[:60])
            continue
        file = newest["file"]
        try:
            data = urllib.request.urlopen(file["url"], timeout=60).read()
            if hashlib.sha256(data).hexdigest() != file["sha256"]:
                raise ValueError("hash differs from the index")
            count = 0
            with zipfile.ZipFile(io.BytesIO(data)) as package:
                for info in package.infolist():
                    path = pathlib.PurePosixPath(info.filename)
                    if info.is_dir() or path.parts[:1] != ("mixins",):
                        continue
                    if ".." in path.parts or path.is_absolute():
                        raise ValueError(f"bad path {info.filename}")
                    target = root / mod / pathlib.Path(*path.parts[1:])
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(package.read(info))
                    count += 1
            if not count:
                raise ValueError("no mixins folder")
            state = "revoked" if mod in revoked else "ok"
        except Exception as error:  # noqa: BLE001 - one broken package must not stop the others
            print("package-fail", mod, repr(error)[:200])
            state = "package-fail"
        parts = str(newest.get("api", "")).split("/")
        rows.append((mod, newest["version"], state, parts[1][:10] if len(parts) > 1 else ""))
    (root / "_list.tsv").write_text("".join("\t".join(r) + "\n" for r in rows))
    print("mods", len(rows), "ok", sum(r[2] == "ok" for r in rows))


def shards(root: pathlib.Path, n: int) -> list[str]:
    ok = [line.split("\t")[0] for line in (root / "_list.tsv").read_text().splitlines() if line.split("\t")[2] == "ok"]
    return [" ".join(ok[i::n]) for i in range(n)]


def main() -> None:
    args = sys.argv[1:]
    if not args or (args[0].startswith("-") and args[0] != "--shards"):
        sys.exit(__doc__)
    if args[:1] == ["--shards"]:
        print(json.dumps(shards(pathlib.Path(args[2]), int(args[1]))))
    else:
        fetch(pathlib.Path(args[0]))


if __name__ == "__main__":
    main()
