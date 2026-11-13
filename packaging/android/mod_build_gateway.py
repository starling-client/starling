#!/usr/bin/env python3
"""The door to Starling's mod build service (mod_build_server.py), on a small machine that stays on.

The build machine costs money by the hour, so it sleeps: a request wakes it, and it is put back to sleep
once it has had nothing to do for a while. Finished libraries and known failures stay here, so asking for
them again doesn't wake it. Behind Caddy, which holds the HTTPS. The machine's own identity may only start,
stop and read that one VM. Python's standard library only.

A player may ask for GitHub's machines instead (`via=github`): slower, with no machine of ours awake. The
gateway keeps the request's mods for the runner to fetch, runs the mod-builds repository's workflow, and
brings the library here when it ends, so it is served as any other.

    BUILD_BACKEND          http://host:8787
    BUILD_VM               the VM's Azure resource id; empty once there is none, and then only the
                           versions GitHub builds are compiled
    GITHUB_VERSIONS        versions always built on GitHub's machines, Compile or Use GitHub alike
                           (comma-separated, the ones mod-builds' versions.json has)
    GATEWAY_PORT           default 8788
    STATE_DIRECTORY        where results stay (systemd's StateDirectory); without it nothing stays
    STORE_BYTES            how much of them, default 10 GiB
    GATEWAY_PUBLIC_URL     where GitHub's runners reach this; without it, or the token, no GitHub builds
    GITHUB_AT_ONCE         GitHub runs at once, default 8; GITHUB_LINE, waiting here beyond them, 24
    GITHUB_BUILDS_PER_DAY  GitHub builds in any 24 hours, everyone's together, default 300
    CREDENTIALS_DIRECTORY  systemd's LoadCredential: github-token, allowed to run mod-builds' workflows
    PERF_BYTES             how much of the test builds' performance reports stay, default 256 MiB

The test builds' performance test sends its results here (POST /v1/perf): each report is kept as it
came, one JSON line in STATE_DIRECTORY/perf/<day>.jsonl, with the time it arrived and nothing about who
sent it (the address only counts the reports per hour, in memory).
"""
import base64
import calendar
import hashlib
import http.client
import http.server
import json
import os
import re
import secrets
import shutil
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

BACKEND = urllib.parse.urlparse(os.environ["BUILD_BACKEND"])
VM = os.environ.get("BUILD_VM", "")
PORT = int(os.environ.get("GATEWAY_PORT", "8788"))
ARM = "https://management.azure.com"
IDENTITY = (
    "http://169.254.169.254/metadata/identity/oauth2/token"
    "?api-version=2018-02-01&resource=https%3A%2F%2Fmanagement.azure.com%2F"
)
MAX_BODY = 32 << 20
# Nothing to do for this long, and the machine goes back to sleep. Short, as it bills by the hour and a
# player's next build waits only the minute it takes to wake.
IDLE_SECONDS = 2 * 60
# Started and never answered: put back to sleep rather than left on.
START_SECONDS = 20 * 60
# Between asks to start it, while it boots.
START_AGAIN_SECONDS = 90
# Builds a day, so one player or everyone together can't run the bill up.
PER_ADDRESS = 32
# A build that failed gives its place back, but it took the machine's time, so failing has its own limit.
FAILED_PER_ADDRESS = 24
PER_DAY = int(os.environ.get("BUILDS_PER_DAY", "80"))
FAILED_PER_DAY = PER_DAY
DAY = 24 * 3600
# As many as the service keeps results of (KEEP_RESULTS): beyond them it would build again, uncounted.
BUILT_REMEMBERED = 12
YOUR_LIMIT = "You have reached today's build limit. Try again tomorrow."
EVERYONES_LIMIT = "The build server has reached today's limit for all players. Try again tomorrow."
STORE = os.environ.get("STATE_DIRECTORY", "")
STORE_BYTES = int(os.environ.get("STORE_BYTES", str(10 << 30)))
# This machine has other work to do: results never bring its free space below this.
MIN_FREE_BYTES = 6 << 30
JOB = re.compile(r"[0-9a-f]{24}")
LIBRARY = "libhorizon_android.so"
CHUNK = 256 * 1024
GITHUB_API = os.environ.get("GITHUB_API", "https://api.github.com")
GITHUB_REPO = os.environ.get("GITHUB_BUILDS_REPO", "starling-client/mod-builds")
GITHUB_WORKFLOW = "build.yml"
PUBLIC_URL = os.environ.get("GATEWAY_PUBLIC_URL", "")
# Runs at once: the rest wait here, where their place in line can be told. A free organization runs
# 20 jobs at once in all, its other repositories' CI included.
GITHUB_AT_ONCE = int(os.environ.get("GITHUB_AT_ONCE", "8"))
GITHUB_LINE = int(os.environ.get("GITHUB_LINE", "24"))
# GitHub's machines cost nothing, but a repository that keeps them busy all day may be asked to stop.
GITHUB_PER_ADDRESS = 32
GITHUB_PER_DAY = int(os.environ.get("GITHUB_BUILDS_PER_DAY", "300"))
# Versions whose builds go to GitHub's machines however they are asked for: the build machine is
# never woken for them.
GITHUB_VERSIONS = {v.strip() for v in os.environ.get("GITHUB_VERSIONS", "").split(",") if v.strip()}
# Asked for a version GitHub doesn't build, with no build machine left.
NO_MACHINE = (
    "This version of Starling can't compile native mods anymore: update Starling, then compile again."
)
# The test builds' performance reports (`/v1/perf`): one is a few kilobytes.
PERF_MAX_BODY = 256 << 10
PERF_PER_HOUR = 30
PERF_BYTES = int(os.environ.get("PERF_BYTES", str(256 << 20)))
# Reports per address in the last hour, only here in memory.
perf_sent = {}
GITHUB_LOOK_SECONDS = 20
# Asked for and no run after this, or a run this old: it isn't coming.
GITHUB_NO_RUN_SECONDS = 15 * 60
GITHUB_SECONDS = 75 * 60
# A failure that wasn't the mods' fault is told for this long; asking again builds again.
GITHUB_FAILURE_SECONDS = 60 * 60
# How often mod-builds' versions.json is read again: the commit each version's builds are made from.
RELEASES_SECONDS = 5 * 60
ASK = re.compile(r"[0-9a-f]{32}")

lock = threading.Lock()
index_lock = threading.Lock()
last_activity = time.time()
started_at = None
asked_at = 0.0
accepted = []
built = {}
# Requests answered here: request to job, and request to the job, when and the service's words.
kept = {}
known_failures = {}
# Jobs whose library is on its way here, and proxied downloads: the machine stays on for them.
copying = {}
proxying = 0
# Jobs on GitHub, job to what it needs, and when and by whom each was asked for today.
github_jobs = {}
github_asked = []
# versions.json as last read: when, and each version's commit (`release_commits`).
releases = {"at": 0.0, "commits": {}}


def power(action):
    """Asks Azure to `start` or `deallocate` the VM. False when it would not."""
    try:
        request = urllib.request.Request(IDENTITY, headers={"Metadata": "true"})
        with urllib.request.urlopen(request, timeout=10) as answer:
            token = json.load(answer)["access_token"]
        request = urllib.request.Request(
            f"{ARM}{VM}/{action}?api-version=2024-07-01",
            data=b"",
            method="POST",
            headers={"Authorization": f"Bearer {token}"},
        )
        with urllib.request.urlopen(request, timeout=30):
            pass
        print(f"asked Azure to {action}", flush=True)
        return True
    except OSError as e:
        print(f"{action} failed: {e}", flush=True)
        return False


def connection(timeout):
    """To the service, connected: a sleeping machine doesn't refuse, it says nothing, so the connecting
    has a short wait of its own."""
    c = http.client.HTTPConnection(BACKEND.hostname, BACKEND.port, timeout=4)
    c.connect()
    c.sock.settimeout(timeout)
    return c


def backend_status():
    """The service's `busy`, or None while it does not answer."""
    try:
        c = connection(4)
        c.request("GET", "/v1/busy")
        answer = c.getresponse()
        return json.loads(answer.read()).get("busy") if answer.status == 200 else None
    except (OSError, ValueError, http.client.HTTPException):
        return None


def wake():
    """Whether the service answers; if not, it is being woken."""
    global started_at, asked_at
    if backend_status() is not None:
        return True
    with lock:
        ask = time.time() - asked_at > START_AGAIN_SECONDS
        if ask:
            asked_at = time.time()
    if ask and power("start"):
        with lock:
            if started_at is None:
                started_at = time.time()
    return False


def kept_awake(now):
    """Whether someone asked for the machine to stay on until a later time: `keep-awake` in the store
    holds that time (Unix seconds). Work of ours the service doesn't see (a worker compiling there)
    keeps it on this way, and the asking runs out by itself."""
    if not STORE:
        return False
    try:
        with open(os.path.join(STORE, "keep-awake")) as f:
            return now < float(f.read().strip())
    except (OSError, ValueError):
        return False


def sleeper():
    """Puts back to sleep what this started, when it has been idle. A machine someone else started
    is left alone, and so is one whose service answered and then stopped: someone is working on it,
    or it is already off. One asked to stay on (`kept_awake`) stays on until the time asked."""
    global started_at
    answered = False
    while True:
        time.sleep(30)
        with lock:
            started, active = started_at, last_activity
            working = bool(copying) or proxying > 0
        if started is None:
            answered = False
            continue
        now = time.time()
        busy = backend_status()
        if busy is None:
            if answered:
                with lock:
                    started_at = None
                continue
            give_up = now - started > START_SECONDS
        else:
            answered = True
            give_up = (
                not busy
                and not working
                and now - active > IDLE_SECONDS
                and now - started > IDLE_SECONDS
            )
        if give_up and not kept_awake(now) and power("deallocate"):
            with lock:
                started_at = None


def refusal(address, request):
    """What to say if this build doesn't count within the day's, asked before it reaches the line:
    refused after, it would still be built. What is built already costs no build, and neither does the
    same request again, unless it failed and would build again. None when it may go on."""
    now = time.time()
    with lock:
        if request in built:
            return None
        accepted[:] = [a for a in accepted if now - a["at"] < DAY]
        # Failed for the mods' own fault: the service says so again without building.
        if any(a["request"] == request and a["settled"] for a in accepted):
            return None
        own = [a for a in accepted if a["address"] == address]
        if any(a["request"] == request and not a["failed"] for a in own):
            return None
        if (
            sum(not a["failed"] for a in own) >= PER_ADDRESS
            or sum(a["failed"] for a in own) >= FAILED_PER_ADDRESS
        ):
            return YOUR_LIMIT
        if (
            sum(not a["failed"] for a in accepted) >= PER_DAY
            # By job: the same mods asked in another order fail without building.
            or len({a["job"] for a in accepted if a["failed"] and a["job"]}) >= FAILED_PER_DAY
        ):
            return EVERYONES_LIMIT
        accepted.append(
            {
                "at": now,
                "address": address,
                "request": request,
                "job": None,
                "failed": False,
                "settled": False,
            }
        )
    return None


def note_job(address, request, job):
    """Ties a counted request to its job. With no job nothing will build, and its place comes back."""
    with lock:
        for a in accepted:
            if a["address"] == address and a["request"] == request and not a["failed"]:
                a["job"] = job
                a["failed"] = job is None


def job_failed(job, settled=False):
    with lock:
        for a in accepted:
            if a["job"] == job:
                a["failed"] = True
                a["settled"] = settled


def job_done(job):
    """Remembers what this job was asked with: anyone asking the same gets its library, no build."""
    with lock:
        for a in accepted:
            if a["job"] == job:
                built.pop(a["request"], None)
                built[a["request"]] = job
        while len(built) > BUILT_REMEMBERED:
            del built[next(iter(built))]


def save_index():
    """What a restart must not forget. The results themselves are their own record."""
    if not STORE:
        return
    path = os.path.join(STORE, "index.json")
    with lock:
        data = json.dumps({"kept": kept, "failures": known_failures, "github": github_jobs})
    try:
        with index_lock:
            with open(path + ".part", "w", encoding="utf-8") as f:
                f.write(data)
            os.replace(path + ".part", path)
    except OSError as e:
        print(f"could not save the index: {e}", flush=True)


def load_index():
    if not STORE:
        return
    try:
        with open(os.path.join(STORE, "index.json"), encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return
    with lock:
        kept.update(data.get("kept") or {})
        known_failures.update(data.get("failures") or {})
        # One whose mods were still being written when this stopped has nothing to build.
        github_jobs.update(
            {j: e for j, e in (data.get("github") or {}).items() if e.get("state") != "writing"}
        )


def result_folder(job):
    return os.path.join(STORE, "results", job)


def kept_status(job):
    """What the service said of a finished job whose library is here, or None. The status is written
    last: with it there, the library is whole."""
    if not STORE:
        return None
    try:
        with open(os.path.join(result_folder(job), "status.json"), encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def failure_of(job):
    """The service's words on a job that failed for the mods' own fault, while they hold."""
    now = time.time()
    with lock:
        for failure in known_failures.values():
            if failure["job"] == job and now - failure["at"] < DAY:
                return failure["status"]
    return None


def answered_here(request):
    """The job of a request that needs no machine: built and kept, or failed for the mods' own fault.
    A kept build of a commit its version no longer names is forgotten, and the request builds again."""
    now = time.time()
    with lock:
        failure = known_failures.get(request)
        if failure and now - failure["at"] < DAY:
            return failure["job"]
        job = kept.get(request)
    status = kept_status(job) if job else None
    if status and stale(status):
        forget_stale(job)
        return None
    return job if status else None


def remember_failure(job, status):
    """The same mods on the same game fail the same way, for as long as a day's count remembers it."""
    now = time.time()
    changed = False
    with lock:
        for a in accepted:
            if a["job"] == job and a["request"] not in known_failures:
                known_failures[a["request"]] = {"job": job, "at": now, "status": status}
                changed = True
        for request in [r for r, f in known_failures.items() if now - f["at"] >= DAY]:
            del known_failures[request]
            changed = True
    if changed:
        save_index()


def forget_oldest():
    """Down to the store's size, what was asked for longest ago going first."""
    root = os.path.join(STORE, "results")
    found = []
    for job in os.listdir(root):
        try:
            asked = os.path.getmtime(os.path.join(root, job, "status.json"))
            found.append((asked, os.path.getsize(os.path.join(root, job, LIBRARY)), job))
        except OSError:
            continue
    found.sort(reverse=True)
    total = 0
    gone = set()
    for _, size, job in found:
        total += size
        if total > STORE_BYTES:
            shutil.rmtree(os.path.join(root, job), ignore_errors=True)
            gone.add(job)
    with lock:
        for request in [r for r, j in kept.items() if j in gone]:
            del kept[request]


def place_result(job, part, status):
    """The library at `part` as this job's, the status written last."""
    folder = result_folder(job)
    os.makedirs(folder, exist_ok=True)
    os.replace(part, os.path.join(folder, LIBRARY))
    with open(os.path.join(folder, "status.part"), "w", encoding="utf-8") as f:
        json.dump(status, f)
    os.replace(os.path.join(folder, "status.part"), os.path.join(folder, "status.json"))


def copy_result(job, status):
    """Brings a finished library here, checked, before the machine sleeps."""
    os.makedirs(os.path.join(STORE, "results"), exist_ok=True)
    folder = result_folder(job)
    part = folder + ".part"
    try:
        if shutil.disk_usage(STORE).free - status["size"] < MIN_FREE_BYTES:
            print(f"no room to keep {job}", flush=True)
            return
        c = connection(60)
        c.request("GET", f"/v1/job/{job}/lib")
        response = c.getresponse()
        if response.status != 200:
            return
        digest = hashlib.sha256()
        with open(part, "wb") as f:
            while chunk := response.read(CHUNK):
                digest.update(chunk)
                f.write(chunk)
        if digest.hexdigest() != status["sha256"] or os.path.getsize(part) != status["size"]:
            print(f"{job} came different from what the service said", flush=True)
            return
        place_result(job, part, status)
        with lock:
            for a in accepted:
                if a["job"] == job:
                    kept[a["request"]] = job
        forget_oldest()
        save_index()
    except (OSError, ValueError, KeyError, http.client.HTTPException) as e:
        print(f"could not keep {job}: {e}", flush=True)
    finally:
        if os.path.exists(part):
            os.remove(part)
        with lock:
            copying.pop(job).set()


def keep_result(job, status):
    if not STORE or kept_status(job):
        return
    with lock:
        if job in copying:
            return
        copying[job] = threading.Event()
    threading.Thread(target=copy_result, args=(job, status), daemon=True).start()


def github_token():
    """The token that may run mod-builds' workflows, or nothing."""
    try:
        path = os.path.join(os.environ["CREDENTIALS_DIRECTORY"], "github-token")
        with open(path, encoding="utf-8") as f:
            return f.read().strip()
    except (KeyError, OSError):
        return ""


def github_open():
    return bool(STORE and PUBLIC_URL and github_token())


def github_headers():
    return {
        "Authorization": f"Bearer {github_token()}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "starling-gateway",
    }


def github_api(method, path, payload=None):
    """One call to the repository's API: its JSON, or None when it answers nothing."""
    request = urllib.request.Request(
        f"{GITHUB_API}/repos/{GITHUB_REPO}{path}",
        data=None if payload is None else json.dumps(payload).encode(),
        method=method,
        headers=github_headers(),
    )
    with urllib.request.urlopen(request, timeout=30) as answer:
        text = answer.read()
    return json.loads(text) if text else None


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def github_download(url, path):
    """An artifact into `path`. It lives elsewhere behind a redirect, and the token stays at GitHub."""
    request = urllib.request.Request(url, headers=github_headers())
    try:
        answer = urllib.request.build_opener(NoRedirect).open(request, timeout=60)
    except urllib.error.HTTPError as e:
        if e.code not in (301, 302, 303, 307, 308):
            raise
        answer = urllib.request.urlopen(e.headers["Location"], timeout=60)
    with answer, open(path, "wb") as f:
        shutil.copyfileobj(answer, f, CHUNK)


def release_commits():
    """Each version's commit in mod-builds' versions.json, which its builds are made from, read at
    most every RELEASES_SECONDS; the last one read while GitHub doesn't answer, and none without it.
    """
    if not github_open():
        return {}
    now = time.time()
    with lock:
        if now - releases["at"] < RELEASES_SECONDS:
            return releases["commits"]
        releases["at"] = now
    try:
        answer = github_api("GET", "/contents/versions.json")
        commits = json.loads(base64.b64decode(answer["content"]))
        if not isinstance(commits, dict):
            raise ValueError("versions.json is not a table")
    except (OSError, ValueError, KeyError, TypeError, urllib.error.URLError) as e:
        print(f"could not read versions.json: {e!r}", flush=True)
        with lock:
            return releases["commits"]
    with lock:
        releases["commits"] = {str(v): str(c) for v, c in commits.items()}
        return releases["commits"]


def stale(status):
    """Whether a kept build was made from another commit than the one its version names now: one
    built while a release candidate had the release's number is not the release (0.8.0's first
    builds were release-candidate-5's, without its controls and skins)."""
    build = (status or {}).get("build") or {}
    want = release_commits().get(build.get("version"))
    return bool(want and build.get("commit") and build["commit"] != want)


def forget_stale(job):
    """A kept build of an older commit goes: asking for its mods builds them again."""
    shutil.rmtree(result_folder(job), ignore_errors=True)
    with lock:
        for request in [r for r, j in kept.items() if j == job]:
            del kept[request]
        github_jobs.pop(job, None)
    print(f"{job}: built from an older commit of its version, forgotten", flush=True)
    save_index()


def github_folder():
    return os.path.join(STORE, "github")


def github_ask(address, request, version, body):
    """This request's job on GitHub, asked for before or now: (job, None), or (None, why not)."""
    if not github_open():
        return None, "Building on GitHub isn't available right now."
    if not re.fullmatch(r"\d+(\.\d+){1,3}", version):
        return None, "That is not a version of the game."
    job = request[:24]
    now = time.time()
    with lock:
        known = github_jobs.get(job)
        if known and known["state"] != "failed":
            return job, None
        github_asked[:] = [a for a in github_asked if now - a[0] < DAY]
        if sum(a[1] == address for a in github_asked) >= GITHUB_PER_ADDRESS:
            return None, YOUR_LIMIT
        if len(github_asked) >= GITHUB_PER_DAY:
            return None, EVERYONES_LIMIT
        if sum(e["state"] in ("waiting", "writing") for e in github_jobs.values()) >= GITHUB_LINE:
            return None, "The line for GitHub is full. Try again in a few minutes."
        if shutil.disk_usage(STORE).free - len(body) < MIN_FREE_BYTES:
            return None, "The build server is full. Try again later."
        github_asked.append((now, address))
        ask = secrets.token_hex(16)
        github_jobs[job] = {
            "ask": ask,
            "request": request,
            "version": version,
            "at": now,
            "state": "writing",
        }
    os.makedirs(github_folder(), exist_ok=True)
    with open(os.path.join(github_folder(), ask), "wb") as f:
        f.write(body)
    with lock:
        github_jobs[job]["state"] = "waiting"
    save_index()
    return job, None


def github_status(job):
    """What the service would say of a job on GitHub, or None when there is none."""
    now = time.time()
    with lock:
        job_now = github_jobs.get(job)
        if not job_now:
            return None
        state = job_now["state"]
        if state == "failed":
            return {"state": "failed", "error": job_now["error"], "settled": False}
        if state == "building":
            return {"state": "building", "seconds": int(now - job_now["started"])}
        ahead = 0
        if state in ("waiting", "writing"):
            ahead = sum(
                e["state"] in ("waiting", "writing") and e["at"] < job_now["at"]
                for e in github_jobs.values()
            )
        return {"state": "queued", "ahead": ahead}


def github_forget_body(ask):
    for name in (ask, f"{ask}.zip"):
        try:
            os.remove(os.path.join(github_folder(), name))
        except OSError:
            pass


def github_failed(job, error):
    print(f"github {job}: {error}", flush=True)
    with lock:
        entry = github_jobs.get(job)
        if not entry:
            return
        entry["state"] = "failed"
        entry["error"] = error
        entry["at"] = time.time()
        ask = entry["ask"]
    github_forget_body(ask)
    save_index()


def github_keep(job, entry, run):
    """The run's answer: its library here, checked, or the failure it reports."""
    archive = os.path.join(github_folder(), f"{entry['ask']}.zip")
    part = result_folder(job) + ".part"
    try:
        artifacts = github_api("GET", f"/actions/runs/{run}/artifacts")["artifacts"]
        url = next(a["archive_download_url"] for a in artifacts if a["name"] == "result")
        github_download(url, archive)
        with zipfile.ZipFile(archive) as z:
            status = json.loads(z.read("status.json"))
            if status.get("state") != "done":
                error = str(status.get("error") or "The build failed.")[:600]
                if not status.get("settled"):
                    return github_failed(job, error)
                failure = {"state": "failed", "error": error, "settled": True}
                with lock:
                    known_failures[entry["request"]] = {
                        "job": job,
                        "at": time.time(),
                        "status": failure,
                    }
                    del github_jobs[job]
                github_forget_body(entry["ask"])
                return save_index()
            if shutil.disk_usage(STORE).free - status["size"] < MIN_FREE_BYTES:
                return github_failed(job, "The build server is full. Try again later.")
            os.makedirs(os.path.join(STORE, "results"), exist_ok=True)
            digest = hashlib.sha256()
            with z.open(LIBRARY) as source, open(part, "wb") as f:
                while chunk := source.read(CHUNK):
                    digest.update(chunk)
                    f.write(chunk)
        if digest.hexdigest() != status["sha256"] or os.path.getsize(part) != status["size"]:
            return github_failed(job, "The build came back different from what it said.")
        place_result(job, part, status)
        with lock:
            kept[entry["request"]] = job
            del github_jobs[job]
        github_forget_body(entry["ask"])
        forget_oldest()
        save_index()
    except (OSError, ValueError, KeyError, StopIteration, zipfile.BadZipFile) as e:
        print(f"github {job}: could not keep it: {e!r}", flush=True)
        github_failed(job, "The build's result did not arrive. Try again.")
    finally:
        if os.path.exists(part):
            os.remove(part)


def github_round():
    """One look: failures that were told long enough go, those in line run, the rest are followed."""
    now = time.time()
    with lock:
        for job in [
            j
            for j, e in github_jobs.items()
            if e["state"] == "failed" and now - e["at"] > GITHUB_FAILURE_SECONDS
        ]:
            del github_jobs[job]
        active = {
            j: dict(e)
            for j, e in github_jobs.items()
            if e["state"] in ("waiting", "asked", "queued", "building")
        }
    if not active:
        return
    running = sum(e["state"] != "waiting" for e in active.values())
    waiting = sorted(
        (j for j, e in active.items() if e["state"] == "waiting"), key=lambda j: active[j]["at"]
    )
    for job in waiting:
        if running >= GITHUB_AT_ONCE:
            break
        entry = active[job]
        try:
            github_api(
                "POST",
                f"/actions/workflows/{GITHUB_WORKFLOW}/dispatches",
                {
                    "ref": "main",
                    "inputs": {
                        "ask": entry["ask"],
                        "version": entry["version"],
                        "gateway": PUBLIC_URL,
                    },
                },
            )
        except urllib.error.HTTPError as e:
            if e.code >= 500:
                break
            github_failed(job, "GitHub refused the build.")
            continue
        except OSError as e:
            print(f"github: could not ask for a run: {e}", flush=True)
            break
        with lock:
            github_jobs[job]["state"] = entry["state"] = "asked"
            github_jobs[job]["asked"] = entry["asked"] = time.time()
        running += 1
    followed = {j: e for j, e in active.items() if e["state"] != "waiting"}
    if not followed:
        return save_index()
    try:
        runs = github_api(
            "GET",
            f"/actions/workflows/{GITHUB_WORKFLOW}/runs?event=workflow_dispatch&per_page=100",
        )["workflow_runs"]
    except (OSError, ValueError, KeyError) as e:
        print(f"github: could not read the runs: {e}", flush=True)
        return None
    by_title = {}
    for run in runs:
        by_title.setdefault(run.get("display_title"), run)
    now = time.time()
    for job, entry in followed.items():
        run = by_title.get(f"build {entry['ask']}")
        if not run:
            if now - entry["asked"] > GITHUB_NO_RUN_SECONDS:
                github_failed(job, "GitHub did not start the build. Try again later.")
            continue
        if now - entry["asked"] > GITHUB_SECONDS:
            github_failed(job, "The build took too long. Try again later.")
        elif run["status"] == "completed":
            if run["conclusion"] == "success":
                github_keep(job, entry, run["id"])
            else:
                github_failed(job, "The build on GitHub did not finish. Try again.")
        elif run["status"] == "in_progress" and entry["state"] != "building":
            started = run.get("run_started_at") or ""
            try:
                began = calendar.timegm(time.strptime(started, "%Y-%m-%dT%H:%M:%SZ"))
            except ValueError:
                began = now
            with lock:
                github_jobs[job]["state"] = "building"
                github_jobs[job]["started"] = began
        elif entry["state"] == "asked":
            with lock:
                github_jobs[job]["state"] = "queued"
    save_index()


def github_worker():
    while True:
        time.sleep(GITHUB_LOOK_SECONDS)
        try:
            if github_open():
                github_round()
        except Exception as e:  # The next look goes on whatever this one did.
            print(f"github: {e!r}", flush=True)


def perf_refusal(address, now):
    """Why a performance report from `address` is refused (too many in the last hour), or None."""
    with lock:
        if len(perf_sent) > 10_000:
            for key in [k for k, v in perf_sent.items() if not v or now - v[-1] >= 3600]:
                del perf_sent[key]
        times = [t for t in perf_sent.get(address, []) if now - t < 3600]
        if len(times) >= PERF_PER_HOUR:
            perf_sent[address] = times
            return "Too many reports from here in the last hour."
        times.append(now)
        perf_sent[address] = times
    return None


def keep_perf(report, now):
    """Keeps one performance report; False when there is no store or no room left."""
    if not STORE:
        return False
    folder = os.path.join(STORE, "perf")
    line = json.dumps({"received": int(now), "report": report}, separators=(",", ":")) + "\n"
    with lock:
        os.makedirs(folder, exist_ok=True)
        used = sum(e.stat().st_size for e in os.scandir(folder) if e.is_file())
        if used + len(line) > PERF_BYTES:
            return False
        day = time.strftime("%Y-%m-%d", time.gmtime(now))
        with open(os.path.join(folder, f"{day}.jsonl"), "a", encoding="utf-8") as f:
            f.write(line)
    return True


class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "StarlingModGateway"
    timeout = 120

    def address(self):
        forwarded = self.headers.get("X-Forwarded-For") or self.client_address[0]
        return forwarded.split(",")[0].strip()

    def answer(self, code, value):
        data = json.dumps(value).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def seen(self):
        global last_activity
        with lock:
            last_activity = time.time()

    def perf(self):
        size = int(self.headers.get("Content-Length") or 0)
        if size <= 0 or size > PERF_MAX_BODY:
            return self.answer(413, {"error": "That report is too big."})
        try:
            report = json.loads(self.rfile.read(size))
        except ValueError:
            return self.answer(400, {"error": "That is not a report."})
        if not isinstance(report, dict) or not isinstance(report.get("schema"), int):
            return self.answer(400, {"error": "That is not a report."})
        now = time.time()
        refused = perf_refusal(self.address(), now)
        if refused:
            return self.answer(429, {"error": refused})
        if not keep_perf(report, now):
            return self.answer(507, {"error": "Reports can't be kept right now."})
        return self.answer(200, {"ok": True})

    def do_POST(self):
        url = urllib.parse.urlparse(self.path)
        if url.path == "/v1/perf":
            return self.perf()
        if url.path != "/v1/build":
            return self.answer(404, {"error": "No such thing here."})
        size = int(self.headers.get("Content-Length") or 0)
        if size <= 0 or size > MAX_BODY:
            return self.answer(413, {"error": "That is too big to compile."})
        body = self.rfile.read(size)
        query = urllib.parse.parse_qs(url.query)
        version = query.get("version", [""])[0]
        github = query.get("via", [""])[0] == "github" or version in GITHUB_VERSIONS
        # The same mods on the same game, whichever machine is asked: what one built, the other has.
        asked = f"{url.path}?version={version}" if github else self.path
        request = hashlib.sha256(asked.encode() + body).hexdigest()
        job = answered_here(request)
        if job:
            return self.answer(200, {"job": job})
        if github:
            job, refused = github_ask(self.address(), request, version, body)
            if refused:
                print(f"refused: {refused}", flush=True)
                return self.answer(429, {"error": refused})
            return self.answer(200, {"job": job})
        if not VM:
            print(f"refused: {NO_MACHINE}", flush=True)
            return self.answer(429, {"error": NO_MACHINE})
        refused = refusal(self.address(), request)
        if refused:
            print(f"refused: {refused}", flush=True)
            return self.answer(429, {"error": refused})
        # Only now is there something for the machine to do: asking alone keeps nothing awake.
        self.seen()
        if not wake():
            return self.answer(200, {"starting": True})
        try:
            c = connection(60)
            c.request(
                "POST",
                self.path,
                body,
                {"Content-Type": "application/octet-stream", "X-Real-IP": self.address()},
            )
            response = c.getresponse()
            data = response.read()
        except (OSError, http.client.HTTPException):
            return self.answer(200, {"starting": True})
        job = json.loads(data).get("job") if response.status == 200 else None
        note_job(self.address(), request, job)
        self.send_response(response.status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def serve_library(self, job):
        path = os.path.join(result_folder(job), LIBRARY)
        size = os.path.getsize(path)
        start = 0
        ranged = re.fullmatch(r"bytes=(\d+)-", self.headers.get("Range") or "")
        if ranged and int(ranged.group(1)) < size:
            start = int(ranged.group(1))
        self.send_response(206 if start else 200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(size - start))
        if start:
            self.send_header("Content-Range", f"bytes {start}-{size - 1}/{size}")
        self.end_headers()
        with open(path, "rb") as f:
            f.seek(start)
            shutil.copyfileobj(f, self.wfile, CHUNK)

    def serve_mods(self, ask):
        """A request's mods, for the runner building them. Whoever has the name, which is random."""
        path = os.path.join(github_folder(), ask)
        if not STORE or not ASK.fullmatch(ask) or not os.path.exists(path):
            return self.answer(404, {"error": "No such thing here."})
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(os.path.getsize(path)))
        self.end_headers()
        with open(path, "rb") as f:
            shutil.copyfileobj(f, self.wfile, CHUNK)

    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        parts = path.strip("/").split("/")
        if parts[:2] == ["v1", "github"] and len(parts) == 3:
            return self.serve_mods(parts[2])
        if parts[:2] != ["v1", "job"] or len(parts) not in (3, 4) or parts[3:] not in ([], ["lib"]):
            return self.answer(404, {"error": "No such thing here."})
        job = parts[2]
        if not JOB.fullmatch(job):
            return self.answer(404, {"error": "No such thing here."})
        library = len(parts) == 4
        failure = failure_of(job)
        if failure and not library:
            return self.answer(200, failure)
        with lock:
            arriving = copying.get(job)
        if library and arriving:
            arriving.wait(60)
        status = kept_status(job)
        if status and stale(status):
            # Not the game its version is any more: asking for its mods again builds it.
            forget_stale(job)
            return self.answer(404, {"error": "No such job."})
        if status:
            try:
                os.utime(os.path.join(result_folder(job), "status.json"))
            except OSError:
                pass
            return self.serve_library(job) if library else self.answer(200, status)
        # Nothing about a job on GitHub wakes the machine.
        on_github = github_status(job)
        if on_github:
            if library:
                return self.answer(404, {"error": "No such thing here."})
            return self.answer(200, on_github)
        return self.forward(path, library, job)

    def forward(self, path, library, job):
        """The service's answer, for what isn't here."""
        global proxying
        headers = {}
        if self.headers.get("Range"):
            headers["Range"] = self.headers["Range"]
        try:
            c = connection(60)
            c.request("GET", path, headers=headers)
            response = c.getresponse()
        except (OSError, http.client.HTTPException):
            self.seen()
            wake()
            if library:
                return self.answer(503, {"error": "The build server is waking up."})
            return self.answer(200, {"state": "starting"})
        self.send_response(response.status)
        for name in ("Content-Type", "Content-Length", "Content-Range"):
            value = response.getheader(name)
            if value:
                self.send_header(name, value)
        self.end_headers()
        if library:
            with lock:
                proxying += 1
            try:
                shutil.copyfileobj(response, self.wfile, CHUNK)
            finally:
                with lock:
                    proxying -= 1
                self.seen()
            return
        data = response.read()
        try:
            status = json.loads(data)
            state = status.get("state")
        except ValueError:
            status, state = None, None
        if state in ("queued", "building"):
            self.seen()
        elif state == "failed":
            settled = bool(status.get("settled"))
            job_failed(job, settled)
            if settled:
                remember_failure(job, status)
        elif state == "done":
            job_done(job)
            keep_result(job, status)
        self.wfile.write(data)


def main():
    global started_at
    load_index()
    # What was on when this restarted is looked after as if this had started it: forgotten, it would stay on.
    if backend_status() is not None:
        started_at = time.time()
    threading.Thread(target=sleeper, daemon=True).start()
    threading.Thread(target=github_worker, daemon=True).start()
    http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()


if __name__ == "__main__":
    main()
