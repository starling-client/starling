"""The build gateway's day limits (packaging/android/mod_build_gateway.py)."""
import base64
import hashlib
import http.client
import http.server
import importlib.util
import io
import json
import os
import tempfile
import threading
import time
import unittest
import zipfile
from pathlib import Path
from unittest import mock

GATEWAY = Path(__file__).resolve().parents[1] / "packaging" / "android" / "mod_build_gateway.py"


def load(**env):
    """A fresh copy of the gateway, with nothing counted yet."""
    spec = importlib.util.spec_from_file_location("mod_build_gateway", GATEWAY)
    module = importlib.util.module_from_spec(spec)
    settings = {"BUILD_BACKEND": "http://127.0.0.1:8787", "BUILD_VM": "/vm", **env}
    with mock.patch.dict(os.environ, settings):
        spec.loader.exec_module(module)
    return module


class DayLimits(unittest.TestCase):
    def test_one_address_is_told_its_own_limit(self):
        gateway = load()
        for i in range(gateway.PER_ADDRESS):
            self.assertIsNone(gateway.refusal("a", f"request {i}"))
        self.assertEqual(gateway.refusal("a", "one more"), gateway.YOUR_LIMIT)
        self.assertIsNone(gateway.refusal("b", "another player's"))

    def test_everyones_limit_is_told_as_everyones(self):
        gateway = load(BUILDS_PER_DAY="3")
        for i in range(3):
            self.assertIsNone(gateway.refusal(f"address {i}", f"request {i}"))
        self.assertEqual(gateway.refusal("new player", "theirs"), gateway.EVERYONES_LIMIT)

    def test_the_day_limit_comes_from_the_environment(self):
        self.assertEqual(load().PER_DAY, 80)
        self.assertEqual(load(BUILDS_PER_DAY="200").PER_DAY, 200)

    def test_a_failed_build_gives_its_place_back(self):
        gateway = load(BUILDS_PER_DAY="2")
        self.assertIsNone(gateway.refusal("a", "broken mod"))
        gateway.note_job("a", "broken mod", "job")
        gateway.job_failed("job")
        self.assertIsNone(gateway.refusal("b", "working mod"))
        self.assertIsNone(gateway.refusal("c", "another working mod"))
        self.assertEqual(gateway.refusal("d", "one too many"), gateway.EVERYONES_LIMIT)

    def test_failures_have_their_own_day_limit(self):
        gateway = load(BUILDS_PER_DAY="2")
        for i in range(2):
            self.assertIsNone(gateway.refusal(f"address {i}", f"broken {i}"))
            gateway.note_job(f"address {i}", f"broken {i}", f"job {i}")
            gateway.job_failed(f"job {i}")
        self.assertEqual(gateway.refusal("new player", "theirs"), gateway.EVERYONES_LIMIT)

    def test_a_failure_of_the_mods_own_passes_at_the_limit_and_counts_nothing(self):
        gateway = load(BUILDS_PER_DAY="1")
        self.assertIsNone(gateway.refusal("a", "broken mod"))
        gateway.note_job("a", "broken mod", "job")
        gateway.job_failed("job", settled=True)
        self.assertIsNone(gateway.refusal("b", "broken mod"))
        self.assertIsNone(gateway.refusal("a", "broken mod"))
        self.assertEqual(len(gateway.accepted), 1)

    def test_a_failure_the_machine_may_have_caused_counts_when_asked_again(self):
        gateway = load()
        self.assertIsNone(gateway.refusal("a", "mod"))
        gateway.note_job("a", "mod", "job")
        gateway.job_failed("job")
        self.assertIsNone(gateway.refusal("a", "mod"))
        self.assertEqual(len(gateway.accepted), 2)

    def test_one_job_failing_for_many_counts_once_for_everyone(self):
        gateway = load(BUILDS_PER_DAY="2")
        for i in range(2):
            self.assertIsNone(gateway.refusal(f"address {i}", f"same mods, order {i}"))
            gateway.note_job(f"address {i}", f"same mods, order {i}", "job")
            gateway.job_failed("job", settled=True)
        self.assertIsNone(gateway.refusal("new player", "theirs"))

    def test_a_request_the_service_refused_is_no_failure_for_everyone(self):
        gateway = load(BUILDS_PER_DAY="2")
        for i in range(2):
            self.assertIsNone(gateway.refusal(f"address {i}", f"request {i}"))
            gateway.note_job(f"address {i}", f"request {i}", None)
        self.assertIsNone(gateway.refusal("new player", "theirs"))

    def test_the_same_request_again_costs_nothing(self):
        gateway = load(BUILDS_PER_DAY="1")
        self.assertIsNone(gateway.refusal("a", "mine"))
        self.assertIsNone(gateway.refusal("a", "mine"))
        self.assertEqual(len(gateway.accepted), 1)

    def test_what_is_built_passes_at_the_limit_and_counts_nothing(self):
        gateway = load(BUILDS_PER_DAY="1")
        self.assertIsNone(gateway.refusal("a", "popular mod"))
        gateway.note_job("a", "popular mod", "job")
        gateway.job_done("job")
        self.assertIsNone(gateway.refusal("b", "popular mod"))
        self.assertEqual(len(gateway.accepted), 1)
        self.assertEqual(gateway.refusal("b", "something new"), gateway.EVERYONES_LIMIT)

    def test_only_as_many_builds_as_the_service_keeps_are_remembered(self):
        gateway = load()
        for i in range(gateway.BUILT_REMEMBERED + 1):
            address = f"address {i}"
            self.assertIsNone(gateway.refusal(address, f"request {i}"))
            gateway.note_job(address, f"request {i}", f"job {i}")
            gateway.job_done(f"job {i}")
        self.assertEqual(len(gateway.built), gateway.BUILT_REMEMBERED)
        self.assertNotIn("request 0", gateway.built)
        self.assertIn(f"request {gateway.BUILT_REMEMBERED}", gateway.built)

    def test_the_gateway_remembers_as_many_as_the_service_keeps(self):
        server = (GATEWAY.parent / "mod_build_server.py").read_text(encoding="utf-8")
        self.assertIn(f"KEEP_RESULTS = {load().BUILT_REMEMBERED}\n", server)


class FakeService(http.server.BaseHTTPRequestHandler):
    """The build service as the gateway sees it, counting what reaches it."""

    library = b"a built game" * 100
    state = "done"
    asked = 0

    def log_message(self, *args):
        pass

    def send(self, value):
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        type(self).asked += 1
        self.send({"job": JOB})

    def do_GET(self):
        if self.path == "/v1/busy":
            return self.send({"busy": False})
        type(self).asked += 1
        cls = type(self)
        if self.path.endswith("/lib"):
            self.send_response(200)
            self.send_header("Content-Length", str(len(cls.library)))
            self.end_headers()
            self.wfile.write(cls.library)
        elif cls.state == "failed":
            self.send({"state": "failed", "error": "does not fit (E_X): no", "settled": True})
        elif cls.state == "building":
            self.send({"state": "building", "seconds": 5})
        else:
            self.send(
                {
                    "state": "done",
                    "sha256": getattr(cls, "says_sha", hashlib.sha256(cls.library).hexdigest()),
                    "size": len(cls.library),
                    "build": {},
                }
            )


JOB = "0123456789abcdef01234567"


class FakeGitHub(http.server.BaseHTTPRequestHandler):
    """The mod-builds repository as the gateway sees it. The runs are set by the test."""

    dispatched = []
    runs = []
    artifact = b""
    token_at_the_blob = None
    # versions.json: each version's commit.
    versions = {}

    def log_message(self, *args):
        pass

    def send(self, value, code=200):
        data = json.dumps(value).encode() if value is not None else b""
        self.send_response(code)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        size = int(self.headers.get("Content-Length") or 0)
        type(self).dispatched.append(json.loads(self.rfile.read(size))["inputs"])
        self.send(None, 204)

    def do_GET(self):
        cls = type(self)
        port = self.server.server_address[1]
        if "/runs?" in self.path:
            self.send({"workflow_runs": cls.runs})
        elif self.path.endswith("/contents/versions.json"):
            content = base64.b64encode(json.dumps(cls.versions).encode()).decode()
            self.send({"encoding": "base64", "content": content})
        elif self.path.endswith("/artifacts"):
            url = f"http://127.0.0.1:{port}/artifact"
            self.send({"artifacts": [{"name": "result", "archive_download_url": url}]})
        elif self.path == "/artifact":
            self.send_response(302)
            self.send_header("Location", f"http://127.0.0.1:{port}/blob")
            self.send_header("Content-Length", "0")
            self.end_headers()
        else:
            cls.token_at_the_blob = self.headers.get("Authorization")
            self.send_response(200)
            self.send_header("Content-Length", str(len(cls.artifact)))
            self.end_headers()
            self.wfile.write(cls.artifact)


def result_zip(status, library=None):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as z:
        z.writestr("status.json", json.dumps(status))
        if library is not None:
            z.writestr("libhorizon_android.so", library)
    return buffer.getvalue()


class Stored(unittest.TestCase):
    """Through real sockets: the gateway in front of a fake service."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.fake = type("Fake", (FakeService,), {"asked": 0, "state": "done"})
        self.service = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.fake)
        threading.Thread(target=self.service.serve_forever, daemon=True).start()
        self.addCleanup(self.service.server_close)
        self.service_up = True
        self.addCleanup(self.stop_service)
        self.gateway = load(
            BUILD_BACKEND=f"http://127.0.0.1:{self.service.server_address[1]}",
            STATE_DIRECTORY=self.folder.name,
        )
        self.gateway.MIN_FREE_BYTES = 0
        self.gateway.power = mock.Mock(return_value=True)
        self.door = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.gateway.Handler)
        threading.Thread(target=self.door.serve_forever, daemon=True).start()
        self.addCleanup(self.door.server_close)
        self.addCleanup(self.door.shutdown)

    def stop_service(self):
        if self.service_up:
            self.service.shutdown()
            self.service.server_close()
            self.service_up = False

    def call(self, method, path, body=None, headers=None):
        c = http.client.HTTPConnection("127.0.0.1", self.door.server_address[1], timeout=20)
        c.request(method, path, body, headers or {})
        response = c.getresponse()
        return response.status, response.read()

    def post(self, body=b"mods"):
        status, data = self.call("POST", "/v1/build?version=0.8.0", body)
        return status, json.loads(data)

    def poll(self):
        return json.loads(self.call("GET", f"/v1/job/{JOB}")[1])

    def settle_copy(self):
        for _ in range(200):
            if not self.gateway.copying:
                return
            time.sleep(0.02)
        self.fail("the copy did not end")

    def built_and_kept(self, body=b"mods"):
        self.assertEqual(self.post(body), (200, {"job": JOB}))
        self.assertEqual(self.poll()["state"], "done")
        self.settle_copy()

    def test_a_kept_build_is_answered_with_the_machine_asleep(self):
        self.built_and_kept()
        self.stop_service()
        self.assertEqual(self.post(), (200, {"job": JOB}))
        self.assertEqual(self.poll()["sha256"], hashlib.sha256(self.fake.library).hexdigest())
        status, data = self.call("GET", f"/v1/job/{JOB}/lib")
        self.assertEqual((status, data), (200, self.fake.library))
        status, data = self.call("GET", f"/v1/job/{JOB}/lib", headers={"Range": "bytes=5-"})
        self.assertEqual((status, data), (206, self.fake.library[5:]))
        self.gateway.power.assert_not_called()

    def test_a_restart_remembers_what_is_kept(self):
        self.built_and_kept()
        again = load(STATE_DIRECTORY=self.folder.name)
        again.load_index()
        request = next(iter(self.gateway.kept))
        self.assertEqual(again.answered_here(request), JOB)

    def test_a_library_that_came_different_is_not_kept(self):
        self.fake.says_sha = "0" * 64
        self.built_and_kept()
        self.assertEqual(self.gateway.kept, {})
        self.assertIsNone(self.gateway.kept_status(JOB))

    def test_a_known_failure_is_answered_with_the_machine_asleep(self):
        self.fake.state = "failed"
        self.assertEqual(self.post(), (200, {"job": JOB}))
        self.assertTrue(self.poll()["settled"])
        self.stop_service()
        self.assertEqual(self.post(), (200, {"job": JOB}))
        self.assertEqual(self.poll()["error"], "does not fit (E_X): no")
        self.gateway.power.assert_not_called()

    def test_a_known_failure_is_asked_again_after_a_day(self):
        self.fake.state = "failed"
        self.post()
        self.poll()
        for failure in self.gateway.known_failures.values():
            failure["at"] -= self.gateway.DAY + 1
        asked = self.fake.asked
        self.post()
        self.assertGreater(self.fake.asked, asked)

    def test_what_is_refused_or_answered_here_keeps_nothing_awake(self):
        self.built_and_kept()
        self.gateway.last_activity = 0.0
        self.post()
        self.poll()
        self.call("GET", f"/v1/job/{JOB}/lib")
        with mock.patch.object(self.gateway, "refusal", return_value="no"):
            self.assertEqual(self.post(b"other mods")[0], 429)
        self.assertEqual(self.call("POST", "/v1/build", b"")[0], 413)
        self.assertEqual(self.gateway.last_activity, 0.0)

    def test_a_build_on_its_way_keeps_the_machine_awake(self):
        self.gateway.last_activity = 0.0
        self.post()
        self.assertGreater(self.gateway.last_activity, 0.0)
        self.gateway.last_activity = 0.0
        self.fake.state = "building"
        self.assertEqual(self.poll()["state"], "building")
        self.assertGreater(self.gateway.last_activity, 0.0)

    def test_the_oldest_goes_when_the_store_is_full(self):
        self.built_and_kept()
        other = "fedcba9876543210fedcba98"
        folder = self.gateway.result_folder(other)
        os.makedirs(folder)
        with open(os.path.join(folder, self.gateway.LIBRARY), "wb") as f:
            f.write(b"x" * 100)
        with open(os.path.join(folder, "status.json"), "w", encoding="utf-8") as f:
            json.dump({"state": "done"}, f)
        os.utime(os.path.join(folder, "status.json"), (1, 1))
        self.gateway.kept["old request"] = other
        self.gateway.STORE_BYTES = len(self.fake.library)
        self.gateway.forget_oldest()
        self.assertIsNone(self.gateway.kept_status(other))
        self.assertNotIn("old request", self.gateway.kept)
        self.assertIsNotNone(self.gateway.kept_status(JOB))

    def test_a_job_that_is_not_a_job_never_reaches_a_path(self):
        for path in ("/v1/job/..%2f..%2fetc", "/v1/job/../../etc/passwd", f"/v1/job/{JOB}/lib/x", "/v1/other"):
            self.assertEqual(self.call("GET", path)[0], 404, path)

    def test_without_a_store_it_works_as_before(self):
        gateway = load()
        self.assertIsNone(gateway.kept_status(JOB))
        gateway.keep_result(JOB, {"sha256": "", "size": 0})
        self.assertEqual(gateway.copying, {})


class PerfReports(unittest.TestCase):
    """The test builds' performance reports (`/v1/perf`), through a real socket."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.gateway = load(STATE_DIRECTORY=self.folder.name)
        self.door = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.gateway.Handler)
        threading.Thread(target=self.door.serve_forever, daemon=True).start()
        self.addCleanup(self.door.server_close)
        self.addCleanup(self.door.shutdown)

    def send(self, body, address="203.0.113.7"):
        c = http.client.HTTPConnection("127.0.0.1", self.door.server_address[1], timeout=20)
        c.request("POST", "/v1/perf", body, {"X-Forwarded-For": address})
        response = c.getresponse()
        return response.status, json.loads(response.read())

    def kept(self):
        folder = Path(self.folder.name) / "perf"
        return [json.loads(line) for f in sorted(folder.glob("*.jsonl")) for line in f.read_text().splitlines()]

    def test_a_report_is_kept_without_who_sent_it(self):
        report = {"schema": 1, "device": {"gpu": "Mali-G57"}, "setups": []}
        self.assertEqual(self.send(json.dumps(report).encode()), (200, {"ok": True}))
        [line] = self.kept()
        self.assertEqual(line["report"], report)
        self.assertIsInstance(line["received"], int)
        self.assertNotIn("203.0.113.7", json.dumps(line))

    def test_what_is_not_a_report_is_refused(self):
        for body in (b"not json", b"[1, 2]", json.dumps({"setups": []}).encode()):
            self.assertEqual(self.send(body)[0], 400)
        self.assertEqual(self.send(b"x" * (self.gateway.PERF_MAX_BODY + 1))[0], 413)
        self.assertEqual(self.kept(), [])

    def test_an_address_sends_a_few_an_hour(self):
        body = json.dumps({"schema": 1}).encode()
        for _ in range(self.gateway.PERF_PER_HOUR):
            self.assertEqual(self.send(body)[0], 200)
        self.assertEqual(self.send(body)[0], 429)
        self.assertEqual(self.send(body, "198.51.100.9")[0], 200)

    def test_a_full_store_keeps_no_more(self):
        self.gateway.PERF_BYTES = 200
        body = json.dumps({"schema": 1, "pad": "x" * 100}).encode()
        self.assertEqual(self.send(body)[0], 200)
        self.assertEqual(self.send(body)[0], 507)
        self.assertEqual(len(self.kept()), 1)

    def test_without_a_store_nothing_is_kept(self):
        self.assertFalse(load().keep_perf({"schema": 1}, 1000.0))


class KeptAwake(unittest.TestCase):
    def test_the_machine_stays_on_until_the_time_asked(self):
        with tempfile.TemporaryDirectory() as store:
            gateway = load(STATE_DIRECTORY=store)
            self.assertFalse(gateway.kept_awake(1000.0))
            path = Path(store) / "keep-awake"
            path.write_text("2000\n")
            self.assertTrue(gateway.kept_awake(1000.0))
            self.assertFalse(gateway.kept_awake(2000.0))
            path.write_text("soon")
            self.assertFalse(gateway.kept_awake(1000.0))
        self.assertFalse(load().kept_awake(1000.0))


class OnGitHub(unittest.TestCase):
    """Through real sockets: the gateway between a fake service, which must stay out of it, and a
    fake GitHub."""

    LIBRARY = b"a game built on GitHub" * 100

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        secrets_folder = Path(self.folder.name) / "credentials"
        secrets_folder.mkdir()
        (secrets_folder / "github-token").write_text("a-token\n")
        store = Path(self.folder.name) / "store"
        store.mkdir()
        self.fake = type("Fake", (FakeService,), {"asked": 0})
        self.service = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.fake)
        threading.Thread(target=self.service.serve_forever, daemon=True).start()
        self.addCleanup(self.service.server_close)
        self.addCleanup(self.service.shutdown)
        self.github = type("GitHub", (FakeGitHub,), {"dispatched": [], "runs": [], "artifact": b""})
        self.github_server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.github)
        threading.Thread(target=self.github_server.serve_forever, daemon=True).start()
        self.addCleanup(self.github_server.server_close)
        self.addCleanup(self.github_server.shutdown)
        self.env = dict(
            BUILD_BACKEND=f"http://127.0.0.1:{self.service.server_address[1]}",
            STATE_DIRECTORY=str(store),
            GATEWAY_PUBLIC_URL="https://gateway.example",
            GITHUB_API=f"http://127.0.0.1:{self.github_server.server_address[1]}",
            CREDENTIALS_DIRECTORY=str(secrets_folder),
        )
        patcher = mock.patch.dict(os.environ, {"CREDENTIALS_DIRECTORY": str(secrets_folder)})
        patcher.start()
        self.addCleanup(patcher.stop)
        self.start_gateway()

    def start_gateway(self):
        self.gateway = load(**self.env)
        self.gateway.MIN_FREE_BYTES = 0
        self.gateway.power = mock.Mock(return_value=True)
        self.door = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.gateway.Handler)
        threading.Thread(target=self.door.serve_forever, daemon=True).start()
        self.addCleanup(self.door.server_close)
        self.addCleanup(self.door.shutdown)

    def call(self, method, path, body=None):
        c = http.client.HTTPConnection("127.0.0.1", self.door.server_address[1], timeout=20)
        c.request(method, path, body)
        response = c.getresponse()
        return response.status, response.read()

    def ask(self, body=b"mods", via="&via=github"):
        status, data = self.call("POST", f"/v1/build?version=0.8.0{via}", body)
        return status, json.loads(data)

    def poll(self, job):
        return json.loads(self.call("GET", f"/v1/job/{job}")[1])

    def run_is(self, status, conclusion=None):
        ask = self.github.dispatched[-1]["ask"]
        self.github.runs = [
            {
                "id": 7,
                "display_title": f"build {ask}",
                "status": status,
                "conclusion": conclusion,
                "run_started_at": "2026-11-09T12:00:00Z",
            }
        ]

    def result_on_github(self, commit=None):
        """The run's artifact: the library, built from `commit` when it is given."""
        library = self.LIBRARY
        build = {"version": "0.8.0", **({"commit": commit} if commit else {})}
        self.github.artifact = result_zip(
            {
                "state": "done",
                "sha256": hashlib.sha256(library).hexdigest(),
                "size": len(library),
                "build": build,
            },
            library,
        )

    def finished_run(self, job, commit=None):
        """Asked for, run and done: the library is here, built from `commit` when it is given."""
        self.result_on_github(commit)
        self.gateway.github_round()
        self.run_is("completed", "success")
        self.gateway.github_round()
        return self.poll(job)

    def test_without_the_token_the_door_is_shut_and_the_machine_stays_out_of_it(self):
        (Path(self.env["CREDENTIALS_DIRECTORY"]) / "github-token").unlink()
        status, data = self.ask()
        self.assertEqual(status, 429)
        self.assertIn("GitHub", data["error"])
        self.assertEqual(self.fake.asked, 0)
        self.gateway.power.assert_not_called()

    def test_a_build_from_the_asking_to_the_library(self):
        status, data = self.ask()
        self.assertEqual(status, 200)
        job = data["job"]
        self.assertEqual(self.poll(job), {"state": "queued", "ahead": 0})
        self.gateway.github_round()
        inputs = self.github.dispatched[0]
        self.assertEqual(inputs["version"], "0.8.0")
        self.assertEqual(inputs["gateway"], "https://gateway.example")
        # The runner fetches the mods by a name it was given.
        self.assertEqual(self.call("GET", f"/v1/github/{inputs['ask']}")[1], b"mods")
        self.assertEqual(self.call("GET", "/v1/github/" + "0" * 32)[0], 404)
        self.run_is("in_progress")
        self.gateway.github_round()
        self.assertEqual(self.poll(job)["state"], "building")
        self.assertEqual(self.call("GET", f"/v1/job/{job}/lib")[0], 404)
        done = self.finished_run(job)
        self.assertEqual(done["state"], "done")
        self.assertEqual(self.call("GET", f"/v1/job/{job}/lib")[1], self.LIBRARY)
        # The token stayed at GitHub, the mods are gone, and the machine never woke.
        self.assertIsNone(self.github.token_at_the_blob)
        self.assertEqual(self.call("GET", f"/v1/github/{inputs['ask']}")[0], 404)
        self.assertEqual(self.fake.asked, 0)
        self.gateway.power.assert_not_called()

    def test_a_build_of_a_commit_its_version_no_longer_names_is_built_again(self):
        # 0.8.0's first builds were release-candidate-5's, and they were still served once
        # versions.json named the release's commit: a game without the release's controls.
        self.github.versions = {"0.8.0": "c" * 40}
        job = self.ask()[1]["job"]
        self.assertEqual(self.finished_run(job, commit="c" * 40)["state"], "done")
        self.assertEqual(self.ask()[1]["job"], job)
        self.assertEqual(len(self.github.dispatched), 1)
        self.github.versions = {"0.8.0": "d" * 40}
        self.gateway.releases["at"] = 0.0
        self.assertEqual(self.ask()[1]["job"], job)
        self.assertIsNone(self.gateway.kept_status(job))
        self.assertEqual(self.poll(job)["state"], "queued")
        self.gateway.github_round()
        self.assertEqual(len(self.github.dispatched), 2)
        done = self.finished_run(job, commit="d" * 40)
        self.assertEqual(done["build"]["commit"], "d" * 40)
        # The release's own build is kept and answered as before.
        self.assertEqual(self.ask()[1]["job"], job)
        self.assertEqual(len(self.github.dispatched), 2)

    def test_a_stale_build_asked_for_by_its_job_is_gone(self):
        self.github.versions = {"0.8.0": "c" * 40}
        job = self.ask()[1]["job"]
        self.finished_run(job, commit="c" * 40)
        self.github.versions = {"0.8.0": "d" * 40}
        self.gateway.releases["at"] = 0.0
        self.assertEqual(self.call("GET", f"/v1/job/{job}/lib")[0], 404)
        self.assertIsNone(self.gateway.kept_status(job))
        self.assertEqual(self.gateway.kept, {})
        self.assertEqual(self.fake.asked, 0)

    def test_versions_json_unread_keeps_what_is_kept(self):
        # A build with no commit, a version versions.json doesn't name, and GitHub not answering:
        # nothing is forgotten.
        job = self.ask()[1]["job"]
        self.finished_run(job)
        self.github.versions = {"0.2.3": "e" * 40}
        self.gateway.releases["at"] = 0.0
        self.assertEqual(self.ask()[1]["job"], job)
        with mock.patch.object(self.gateway, "github_api", side_effect=OSError("down")):
            self.gateway.releases["at"] = 0.0
            self.assertEqual(self.gateway.release_commits(), {"0.2.3": "e" * 40})
        self.assertEqual(len(self.github.dispatched), 1)

    def test_what_github_built_answers_both_ways_of_asking_with_no_build(self):
        job = self.ask()[1]["job"]
        self.finished_run(job)
        self.assertEqual(self.ask()[1]["job"], job)
        self.assertEqual(self.ask(via="")[1]["job"], job)
        self.assertEqual(len(self.github.dispatched), 1)
        self.assertEqual(self.fake.asked, 0)

    def test_a_version_github_builds_goes_there_from_the_plain_compile(self):
        self.env["GITHUB_VERSIONS"] = "0.8.0, 0.8.1"
        self.start_gateway()
        status, data = self.ask(via="")
        self.assertEqual(status, 200)
        self.gateway.github_round()
        self.assertEqual(len(self.github.dispatched), 1)
        self.assertEqual(self.finished_run(data["job"])["state"], "done")
        # Use GitHub asks for the same build, and the machine never woke.
        self.assertEqual(self.ask()[1]["job"], data["job"])
        self.assertEqual(len(self.github.dispatched), 1)
        self.assertEqual(self.fake.asked, 0)
        self.gateway.power.assert_not_called()

    def test_with_no_build_machine_another_version_is_told_to_update(self):
        self.env.update(GITHUB_VERSIONS="0.8.0", BUILD_VM="")
        self.start_gateway()
        status, data = self.call("POST", "/v1/build?version=0.2.3", b"mods")
        self.assertEqual(status, 429)
        self.assertIn("update Starling", json.loads(data)["error"])
        self.assertEqual(self.fake.asked, 0)
        self.gateway.power.assert_not_called()
        # The version GitHub builds still compiles.
        self.assertEqual(self.ask(via="")[0], 200)

    def test_asking_twice_while_it_builds_is_one_build(self):
        job = self.ask()[1]["job"]
        self.gateway.github_round()
        self.assertEqual(self.ask()[1]["job"], job)
        self.gateway.github_round()
        self.assertEqual(len(self.github.dispatched), 1)

    def test_the_mods_own_fault_is_remembered_without_building_again(self):
        job = self.ask()[1]["job"]
        self.github.artifact = result_zip(
            {"state": "failed", "error": "does not fit (E_X): no", "settled": True}
        )
        self.gateway.github_round()
        self.run_is("completed", "success")
        self.gateway.github_round()
        self.assertEqual(self.poll(job)["error"], "does not fit (E_X): no")
        self.assertEqual(self.ask()[1]["job"], job)
        self.gateway.github_round()
        self.assertEqual(len(self.github.dispatched), 1)

    def test_a_run_that_breaks_is_told_and_asking_again_builds_again(self):
        job = self.ask()[1]["job"]
        self.gateway.github_round()
        self.run_is("completed", "failure")
        self.gateway.github_round()
        failed = self.poll(job)
        self.assertEqual(failed["state"], "failed")
        self.assertFalse(failed["settled"])
        self.assertEqual(self.ask()[1]["job"], job)
        self.gateway.github_round()
        self.assertEqual(len(self.github.dispatched), 2)
        self.assertNotEqual(self.github.dispatched[0]["ask"], self.github.dispatched[1]["ask"])

    def test_a_library_that_differs_from_what_the_run_said_is_not_kept(self):
        job = self.ask()[1]["job"]
        self.github.artifact = result_zip(
            {"state": "done", "sha256": "0" * 64, "size": len(self.LIBRARY), "build": {}},
            self.LIBRARY,
        )
        self.gateway.github_round()
        self.run_is("completed", "success")
        self.gateway.github_round()
        self.assertEqual(self.poll(job)["state"], "failed")
        self.assertEqual(self.call("GET", f"/v1/job/{job}/lib")[0], 404)

    def test_a_few_run_at_once_and_the_rest_know_their_place(self):
        self.gateway.GITHUB_AT_ONCE = 1
        first = self.ask(b"first")[1]["job"]
        second = self.ask(b"second")[1]["job"]
        third = self.ask(b"third")[1]["job"]
        self.gateway.github_round()
        self.assertEqual(len(self.github.dispatched), 1)
        self.assertEqual(self.poll(first), {"state": "queued", "ahead": 0})
        self.assertEqual(self.poll(second), {"state": "queued", "ahead": 0})
        self.assertEqual(self.poll(third), {"state": "queued", "ahead": 1})

    def test_an_address_has_a_limit_of_its_own_here(self):
        self.gateway.GITHUB_PER_ADDRESS = 2
        self.assertEqual(self.ask(b"one")[0], 200)
        self.assertEqual(self.ask(b"two")[0], 200)
        status, data = self.ask(b"three")
        self.assertEqual(status, 429)
        self.assertEqual(data["error"], self.gateway.YOUR_LIMIT)

    def test_a_build_under_way_is_still_followed_after_a_restart(self):
        job = self.ask()[1]["job"]
        self.gateway.github_round()
        self.start_gateway()
        self.gateway.load_index()
        self.assertEqual(self.poll(job)["state"], "queued")
        self.assertEqual(self.finished_run(job)["state"], "done")

    def test_a_run_that_ended_gives_its_place_to_the_next_in_line_in_the_same_look(self):
        # A look used to start runs before reading which had ended, and to bring each result here
        # before the next look: an ended run held its place, and with the line full GitHub ran
        # fewer builds than it may while players were turned away.
        self.gateway.GITHUB_AT_ONCE = 1
        first = self.ask(b"first")[1]["job"]
        second = self.ask(b"second")[1]["job"]
        self.gateway.github_round()
        self.assertEqual(len(self.github.dispatched), 1)
        self.result_on_github()
        self.run_is("completed", "success")
        on_its_way = []
        self.gateway.github_round(collect=lambda *job: on_its_way.append(job))
        # The second runs while the first's library is still to bring here, and the first is
        # still the player's build.
        self.assertEqual(len(self.github.dispatched), 2)
        self.assertEqual([job for job, _, _ in on_its_way], [first])
        self.assertEqual(self.poll(first)["state"], "building")
        self.assertEqual(self.poll(second)["state"], "queued")
        self.gateway.github_keep(*on_its_way[0])
        self.assertEqual(self.poll(first)["state"], "done")
        self.assertEqual(self.call("GET", f"/v1/job/{first}/lib")[1], self.LIBRARY)

    def test_a_result_on_its_way_when_the_service_stopped_is_brought_again(self):
        job = self.ask()[1]["job"]
        self.gateway.github_round()
        self.result_on_github()
        self.run_is("completed", "success")
        # The run ended and its download never finished: the service stopped first.
        self.gateway.github_round(collect=lambda *job: None)
        self.start_gateway()
        self.gateway.load_index()
        self.gateway.github_round()
        self.assertEqual(self.poll(job)["state"], "done")
        self.assertEqual(self.call("GET", f"/v1/job/{job}/lib")[1], self.LIBRARY)


if __name__ == "__main__":
    unittest.main()
