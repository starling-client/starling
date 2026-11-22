#!/usr/bin/env python3
"""Reuse a vanilla oracle's last results when nothing it reads has changed.

tools/parity.py puts `java` and `javac` links to this script first on PATH.
A run is identified by its command line, stdin when it is a file, and the
real program. After a successful run, the files it created or changed under
the paths its arguments name (classpath and sourcepath entries included) are
recorded as its outputs, with their size and mtime and its stdout and stderr;
every other file under those paths is recorded as an input with its content
hash. The next run of the same command replays the recorded output instead
of running the oracle if every input has the same content, no file was added
beside them, and the outputs are exactly as it left them. On a Rust-only
change a harness then pays only for its Rust side.

Files an oracle writes outside its named paths are not tracked. A harness
that depends on them fails on reuse; tools/parity.py then reruns it fresh and
stops reusing its oracle results.

A run that reads stdin from a pipe, or fails, is never cached.
PARITY_JAVA_CACHE=0 disables reuse (tools/parity.py --fresh).

A run's identity includes the JVM option variables (JVM_ENVIRONMENT), which
change what java does without changing its command line, and the variables the
oracles read (ORACLE_ENVIRONMENT): their values, and the files under any path a
value names as inputs, as for a path in the arguments.

PARITY_ORACLE_KEEP names a folder that outlives the checkout (the daily's
per-repository CI keep). There a run is also kept by content: its command,
the content of the JDK it ran on (toolchain()), and the hash of every file
under its named paths before it ran, with its stdout, stderr and the content
of its outputs. With a keep, only the JDK's own launcher ($JAVA_HOME/bin/java
or javac) is reused at all, from either cache: behind a wrapper or selector
the JDK that runs is unknown, so such a run always runs. A checkout with no record of its own replays a kept
run whose command, JDK and files match exactly, writing its outputs back where
none of them exists yet; an output already in place makes it a miss. Entries are
written under a temporary name in the keep and moved into place, so a reader
never sees half of one; a run whose outputs or streams are larger than
KEPT_FILE or KEPT_RUN is not kept. The keep only ever saves work: an entry that
can't be read whole is a miss, and one that can't be written is skipped.
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import threading

CACHE = Path(os.environ.get("PARITY_SHIM_CACHE", ".local/parity-runner/oracle-cache"))
KEEP = Path(os.environ["PARITY_ORACLE_KEEP"]) if os.environ.get("PARITY_ORACLE_KEEP") else None
KEEP_FORMAT = 2
# Read by the JVM itself: the same command line runs differently under each.
JVM_ENVIRONMENT = ("JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "JDK_JAVAC_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH")
# Read by the oracles (System.getenv in mods/sunrise/src/test/java): harnesses pass files and choices through them. A
# variable an oracle starts reading belongs here.
ORACLE_ENVIRONMENT = ("AI_TRACE", "CAULDRON_TRACE", "DUMP_TOOLTIPS", "MENU_TRACE", "ORACLE_TRACE", "SCULK_TRACE",
                      "SUNRISE_CATALOG_TRACE", "SUNRISE_CHUNKS", "SUNRISE_DATAPACKS", "SUNRISE_DIMENSION", "SUNRISE_FLAT",
                      "SUNRISE_HEIGHTS", "SUNRISE_ISOLATED", "SUNRISE_OLD_CHUNKS", "SUNRISE_ORDER", "SUNRISE_PHASE",
                      "SUNRISE_PIECES", "SUNRISE_PLACED", "SUNRISE_RINGS", "SUNRISE_START_NBT")
# The files of a JDK that decide what its programs do, relative to its home (the launcher's folder's parent): an
# update in place changes at least these.
JDK_FILES = ("release", "lib/modules", "lib/server/libjvm.so", "lib/server/libjvm.dylib")
# Larger oracle outputs (chunk dumps) run again instead: the keep holds 20 GiB for every kept run.
KEPT_FILE, KEPT_RUN = 64 << 20, 256 << 20
PATH_OPTIONS = {"-cp", "-classpath", "--class-path", "-sourcepath", "--source-path", "-p", "--module-path"}


# How deep the shim may run inside itself before it stops: it never needs to, since the real program
# gets PARITY_SHIM_REAL_PATH as its PATH, but a wrapper that puts the shim back would start one
# process after another until the machine runs out of memory.
MAX_DEPTH = 4


def real(name):
    """The first `name` on PARITY_SHIM_REAL_PATH that is not this script (a nested run's PATH holds the shim)."""
    me = Path(__file__).resolve()
    for folder in os.environ.get("PARITY_SHIM_REAL_PATH", "").split(os.pathsep):
        found = folder and shutil.which(name, path=folder)
        if found and Path(found).resolve() != me:
            return found
    sys.exit(f"parity shim: no real {name} on PARITY_SHIM_REAL_PATH")


def enter():
    """Count this run and give what it starts the real PATH, so a wrapper looking `java` up on PATH
    (a JDK selector, a build wrapper) finds the real one and not the shim again."""
    depth = int(os.environ.get("PARITY_SHIM_DEPTH") or 0)
    if depth >= MAX_DEPTH:
        sys.exit(f"parity shim: started inside itself {depth} times: what {os.path.basename(sys.argv[0])} runs "
                 "leads back to the shim (a wrapper that puts it back on PATH?)")
    os.environ["PARITY_SHIM_DEPTH"] = str(depth + 1)
    os.environ["PATH"] = os.environ.get("PARITY_SHIM_REAL_PATH", "")


class Hashes:
    """Content hashes keyed by path, size and mtime, shared by parallel harnesses."""

    def __init__(self):
        CACHE.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(CACHE / "hashes.sqlite", timeout=60)
        self.db.execute("pragma journal_mode=wal")
        self.db.execute("create table if not exists hashes (path text primary key, size int, mtime int, sha text)")

    def file(self, path):
        stat = os.stat(path)
        row = self.db.execute("select size, mtime, sha from hashes where path = ?", (path,)).fetchone()
        if row and row[0] == stat.st_size and row[1] == stat.st_mtime_ns:
            return row[2]
        with open(path, "rb") as stream:
            sha = hashlib.file_digest(stream, "sha256").hexdigest()
        self.db.execute("insert or replace into hashes values (?, ?, ?, ?)", (path, stat.st_size, stat.st_mtime_ns, sha))
        self.db.commit()
        return sha


def tree(root):
    for directory, _, files in os.walk(root):
        for name in files:
            yield os.path.join(directory, name)


def environment_paths(environment):
    """Every existing filesystem path a keyed variable names, a path list split as a classpath is."""
    return [os.path.abspath(entry) for value in environment.values() for entry in value.split(os.pathsep)
            if entry and os.path.exists(entry)]


def named_paths(arguments):
    """Every filesystem path the arguments name, with classpath lists split."""
    paths = []
    for index, argument in enumerate(arguments):
        if index and arguments[index - 1] in PATH_OPTIONS:
            paths += [entry for entry in argument.split(os.pathsep) if entry]
        elif argument.startswith("-D") and "=" in argument:
            paths.append(argument.split("=", 1)[1])
        elif argument.startswith("-javaagent:"):
            # The jar is an input; its options, after "=", are in the command line already.
            paths.append(argument.removeprefix("-javaagent:").split("=", 1)[0])
        elif not argument.startswith("-"):
            paths.append(argument)
    return [os.path.abspath(path) for path in paths if path]


def snapshot(paths):
    state = {}
    for path in paths:
        files = tree(path) if os.path.isdir(path) else [path]
        for file in files:
            try:
                stat = os.stat(file)
                state[file] = [stat.st_size, stat.st_mtime_ns]
            except FileNotFoundError:
                state[file] = None
    return state


def stdin_identity(hashes):
    try:
        stat = os.fstat(0)
    except OSError:
        return "closed"
    if os.path.samestat(stat, os.stat(os.devnull)) if os.path.exists(os.devnull) else False:
        return "null"
    import stat as modes
    if modes.S_ISREG(stat.st_mode):
        # Hash the content without moving the shared offset (no /proc on macOS).
        digest, offset = hashlib.sha256(), 0
        while chunk := os.pread(0, 1 << 20, offset):
            digest.update(chunk)
            offset += len(chunk)
        return "f:" + digest.hexdigest()
    if modes.S_ISCHR(stat.st_mode):
        return "tty"
    return None  # a pipe or socket: its content is unknown, so never cache


def run_and_tee(command):
    """Run the real program, forwarding and recording its stdout and stderr."""
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    captured = {"stdout": bytearray(), "stderr": bytearray()}

    def pump(source, sink, buffer):
        for chunk in iter(lambda: source.read1(65536), b""):
            buffer += chunk
            sink.write(chunk)
            sink.flush()

    threads = [threading.Thread(target=pump, args=(process.stdout, sys.stdout.buffer, captured["stdout"])),
               threading.Thread(target=pump, args=(process.stderr, sys.stderr.buffer, captured["stderr"]))]
    for thread in threads:
        thread.start()
    status = process.wait()
    for thread in threads:
        thread.join()
    return status, captured


def files_under(paths):
    found = set()
    for path in paths:
        if os.path.isdir(path):
            found.update(tree(path))
        elif os.path.isfile(path):
            found.add(path)
    return found


def note_outputs(outputs):
    """Tell tools/parity.py which files are oracle outputs, which it keeps when clearing large outputs."""
    listing = os.environ.get("PARITY_SHIM_OUTPUTS")
    if listing:
        with open(listing, "a") as stream:
            stream.writelines(f"{path}\n" for path in outputs)


def reusable(saved, paths, hashes):
    """The recorded inputs are unchanged, nothing was added beside them, and the outputs are as left."""
    outputs = saved["outputs"]
    if files_under(paths) - set(outputs) != set(saved["inputs"]):
        return False
    if any(snapshot([path]).get(path) != stamp for path, stamp in outputs.items()):
        return False
    return all(hashes.file(path) == sha for path, sha in saved["inputs"].items())


def direct_jdk(program, name):
    """Whether `program` is $JAVA_HOME's own `name`, resolving to a file in the JDK's own bin/: a wrapper's or
    selector's JDK, or a link out of the JDK, can change behind the same file."""
    home = os.environ.get("JAVA_HOME")
    resolved = os.path.realpath(program)
    return (bool(home) and os.path.basename(resolved) == name
            and os.path.dirname(resolved) == os.path.join(os.path.realpath(home), "bin"))


def toolchain(program, hashes):
    """The content of the JDK `program` belongs to: the launcher and JDK_FILES that exist."""
    home = Path(program).parent.parent
    files = [Path(program), *(home / name for name in JDK_FILES)]
    return {str(path): hashes.file(str(path)) for path in files if path.is_file()}


def keep_entry(command, inputs):
    """The keep's folder for this command (including its JDK) run on exactly these files."""
    digest = hashlib.sha256(json.dumps({"format": KEEP_FORMAT, "command": command, "inputs": inputs},
                                       sort_keys=True).encode()).hexdigest()
    return KEEP / digest[:2] / digest


def discard(paths):
    for path in paths:
        try:
            Path(path).unlink(missing_ok=True)
        except OSError:
            pass


def restore(entry):
    """Put a kept run's outputs in place; returns their paths, its stdout and its stderr. None, with no existing file
    touched, when the entry is missing or can't be read whole (pruned meanwhile, a damaged keep), names an unusable
    path, or any of its outputs already exists."""
    staged = []
    try:
        kept = json.loads((entry / "run.json").read_text())
        if not isinstance(kept["outputs"], list) or not all(isinstance(path, str) and os.path.isabs(path)
                                                             and "\0" not in path for path in kept["outputs"]):
            return None
        for path in kept["outputs"]:
            os.fsencode(path)  # a path this system can't name (UnicodeEncodeError, a ValueError) is a miss
        # Replay only adds files: one already in place (a stale output, a folder in the way) leaves the oracle to run.
        if any(os.path.lexists(path) for path in kept["outputs"]):
            return None
        stdout, stderr = (entry / "stdout").read_bytes(), (entry / "stderr").read_bytes()
        for index, path in enumerate(kept["outputs"]):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            temporary = f"{path}.{os.getpid()}.restore"
            # Created here or not at all: a file already at that name is never written over, nor removed.
            with open(temporary, "xb") as target:
                staged.append((temporary, path))
                with open(entry / "files" / str(index), "rb") as source:
                    shutil.copyfileobj(source, target)
    except (OSError, ValueError, KeyError, TypeError):
        discard(temporary for temporary, _ in staged)
        return None
    published = []
    try:
        for temporary, path in staged:
            # A link never replaces: a file that appeared meanwhile stays, and the replay is a miss.
            os.link(temporary, path)
            published.append(path)
    except OSError:
        # Half a replay is worse than none: what it published (only files it created) goes, and the oracle runs.
        discard(published)
        return None
    finally:
        discard(temporary for temporary, _ in staged)
    # Its mtime orders the keep's pruning (tools/parity.py): a replayed run stays. Pruned meanwhile, it was still read.
    try:
        os.utime(entry)
    except OSError:
        pass
    return kept["outputs"], stdout, stderr


def keep(entry, outputs, captured):
    """Keep a run for later checkouts, unless its outputs or streams are too large. Best effort: a full or vanished
    keep costs only the reuse."""
    temporary = entry.parent / f".{entry.name}.{os.getpid()}.tmp"
    try:
        sizes = [os.path.getsize(path) for path in outputs] + [len(captured["stdout"]), len(captured["stderr"])]
        if entry.exists() or any(size > KEPT_FILE for size in sizes) or sum(sizes) > KEPT_RUN:
            return
        (temporary / "files").mkdir(parents=True)
        for index, path in enumerate(outputs):
            shutil.copyfile(path, temporary / "files" / str(index))
        (temporary / "stdout").write_bytes(captured["stdout"])
        (temporary / "stderr").write_bytes(captured["stderr"])
        (temporary / "run.json").write_text(json.dumps({"outputs": list(outputs)}))
        os.rename(temporary, entry)
    except OSError:
        # Another harness kept the same run first, or the keep is full.
        shutil.rmtree(temporary, ignore_errors=True)


def record_run(record, key, command, inputs, outputs, stdout, stderr):
    (CACHE / f"{key}.stdout").write_bytes(stdout)
    (CACHE / f"{key}.stderr").write_bytes(stderr)
    temporary = record.with_suffix(f".{os.getpid()}.tmp")
    temporary.write_text(json.dumps({"command": command, "inputs": inputs, "outputs": outputs}))
    temporary.replace(record)
    note_outputs(outputs)


def main():
    name = os.path.basename(sys.argv[0])
    arguments = sys.argv[1:]
    program = real(name)
    enter()
    if (os.environ.get("PARITY_JAVA_CACHE") == "0" or "-version" in arguments or "--version" in arguments
            or KEEP and not direct_jdk(program, name)):
        os.execv(program, [program, *arguments])
    hashes = Hashes()
    stdin = stdin_identity(hashes)
    if stdin is None:
        os.execv(program, [program, *arguments])
    environment = {name: os.environ[name] for name in (*JVM_ENVIRONMENT, *ORACLE_ENVIRONMENT) if name in os.environ}
    paths = named_paths(arguments) + environment_paths(environment)
    command = {"program": os.path.realpath(program), "arguments": arguments, "cwd": os.getcwd(), "stdin": stdin,
               "environment": environment}
    if KEEP:
        # The JDK's content is part of both caches' keys: an update in place replays nothing of the old one.
        command["jdk"] = toolchain(command["program"], hashes)
    key = hashlib.sha256(json.dumps(command, sort_keys=True).encode()).hexdigest()
    record = CACHE / f"{key}.json"
    saved = json.loads(record.read_text()) if record.exists() else None
    if saved and reusable(saved, paths, hashes):
        note_outputs(saved["outputs"])
        sys.stdout.buffer.write((CACHE / f"{key}.stdout").read_bytes())
        sys.stderr.buffer.write((CACHE / f"{key}.stderr").read_bytes())
        return 0
    before = snapshot(paths)
    inputs = {path: hashes.file(path) for path in files_under(paths)}
    entry = keep_entry(command, inputs) if KEEP else None
    kept = restore(entry) if entry else None
    if kept:
        paths_written, stdout, stderr = kept
        outputs = {path: snapshot([path])[path] for path in paths_written}
        record_run(record, key, command, {path: sha for path, sha in inputs.items() if path not in outputs}, outputs,
                   stdout, stderr)
        sys.stdout.buffer.write(stdout)
        sys.stderr.buffer.write(stderr)
        return 0
    status, captured = run_and_tee([program, *arguments])
    if status != 0:
        return status
    after = snapshot(paths)
    outputs = {path: stamp for path, stamp in after.items() if stamp is not None and before.get(path) != stamp}
    if entry:
        keep(entry, outputs, captured)
    record_run(record, key, command, {path: sha for path, sha in inputs.items() if path not in outputs}, outputs,
               captured["stdout"], captured["stderr"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
