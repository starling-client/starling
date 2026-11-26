import json
import sys
import unittest

from tools.perf_bench import HISTOGRAM_EDGES_MS, OPTIONS, frames, rnd, summary


class ComparisonConditionsTests(unittest.TestCase):
    def test_rejects_incompatible_or_unknown_conditions_before_printing(self):
        import contextlib
        import io
        import tempfile
        from pathlib import Path
        from unittest import mock
        import tools.perf_bench as pb

        conditions = {"scenario": "flight", "seed": "1234", "render_distance": 8, "simulation_distance": 12,
                      "window": [1920, 1080], "vsync": False, "offscreen": False}
        differences = {"scenario": "other", "seed": "5678", "simulation_distance": 8, "window": [1280, 720],
                       "vsync": True, "offscreen": True, "fps_cap": 60}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / "benchmarks/perf"
            reports.mkdir(parents=True)
            def write(label, values):
                (reports / f"{label}-rd8.json").write_text(
                    json.dumps({"conditions": values, "metrics": {}}), encoding="utf-8")
            write("first", conditions)
            with mock.patch.object(pb, "ROOT", root):
                for key, value in differences.items():
                    with self.subTest(key=key):
                        write("second", dict(conditions, **{key: value}))
                        out = io.StringIO()
                        with contextlib.redirect_stdout(out), self.assertRaisesRegex(ValueError, key):
                            pb.table(["first", "second"])
                        self.assertEqual(out.getvalue(), "")
                incomplete = dict(conditions)
                del incomplete["window"]
                write("second", incomplete)
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(ValueError, "window"):
                    pb.table(["first", "second"])
                stderr, stdout = io.StringIO(), io.StringIO()
                with mock.patch("sys.argv", ["perf_bench.py", "--table", "first", "second"]), \
                        contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr), \
                        self.assertRaises(SystemExit) as exit_error:
                    pb.main()
                self.assertEqual(exit_error.exception.code, 2)
                self.assertEqual(stdout.getvalue(), "")
                self.assertIn("window missing in second", stderr.getvalue())
                self.assertNotIn("Traceback", stderr.getvalue())
                write("second", dict(conditions, fps_cap=None))
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(ValueError, "fps_cap"):
                    pb.table(["first", "second"])

    def test_matching_reports_and_single_incomplete_report_remain_readable(self):
        import contextlib
        import io
        import tempfile
        from pathlib import Path
        from unittest import mock
        import tools.perf_bench as pb

        conditions = {"scenario": "flight", "seed": "1234", "render_distance": 8, "simulation_distance": 12,
                      "window": [1920, 1080], "vsync": False, "offscreen": False}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / "benchmarks/perf"
            reports.mkdir(parents=True)
            for label, values in (("first", conditions), ("second", conditions),
                                  ("single", {"render_distance": 8})):
                (reports / f"{label}-rd8.json").write_text(
                    json.dumps({"conditions": values, "metrics": {}}), encoding="utf-8")
            with mock.patch.object(pb, "ROOT", root), contextlib.redirect_stdout(io.StringIO()) as out:
                pb.table(["first", "second"])
                pb.table(["single"])
            self.assertIn("| 8 |", out.getvalue())



class PerfBenchStatsTests(unittest.TestCase):
    def test_lows_average_the_slowest_frames(self):
        times = [10.0] * 990 + [100.0] * 9 + [1000.0]
        s = frames("village", 0, times)
        self.assertEqual(s["frames"], 1000)
        self.assertEqual(s["avg_fps"], rnd(1000 * 1000 / (9900 + 900 + 1000)))
        # Slowest 1%: 9 frames of 100 ms and one of 1000 ms.
        self.assertEqual(s["low_1pct_fps"], rnd(1000 * 10 / 1900))
        # Slowest 0.1%: the single 1000 ms frame.
        self.assertEqual(s["low_01pct_fps"], 1.0)
        self.assertEqual((s["p50_ms"], s["p99_ms"], s["p999_ms"], s["max_ms"]), (10.0, 10.0, 100.0, 1000.0))
        self.assertEqual(sum(s["histogram"]), 1000)
        self.assertEqual(len(s["histogram"]), len(HISTOGRAM_EDGES_MS))
        self.assertEqual(s["histogram"][HISTOGRAM_EDGES_MS.index(10)], 990)
        self.assertEqual(s["histogram"][HISTOGRAM_EDGES_MS.index(100)], 9)
        self.assertEqual(s["histogram"][-1], 1, "1000 ms and over")

    def test_same_figures_as_sunrise_bench(self):
        # crates/sunrise-bench's frame_figures_and_histogram test, value for value.
        s = frames("flight", 0, [2.0] * 998 + [20.0, 1500.0])
        self.assertEqual((s["seconds"], s["low_1pct_fps"], s["low_01pct_fps"]), (3.516, 6.51, 0.667))
        self.assertEqual((s["histogram"][2], s["histogram"][13], s["histogram"][23]), (998, 1, 1))
        self.assertNotIn("cpu_p50_ms", s)
        r = summary([2.0, 4.0, 6.0])
        self.assertEqual(r, {"n": 3, "mean": 4.0, "stdev": 2.0, "min": 2.0, "max": 6.0, "ci95": 4.969})
        self.assertEqual(summary([5.0])["ci95"], None)
        self.assertIsNone(summary([None]))

    def test_a_short_flight_still_has_lows(self):
        s = frames("x", 0, [5.0, 20.0])
        self.assertEqual((s["low_1pct_fps"], s["low_01pct_fps"]), (50.0, 50.0))
        self.assertIsNone(frames("x", 0, []))
        json.dumps(s)

    def test_window_sizes_name_their_reports(self):
        from tools.perf_bench import SIZES, parse_sizes, size_label
        self.assertEqual(parse_sizes("1280x720,1920X1080"), [(1280, 720), (1920, 1080)])
        self.assertEqual(SIZES, [(1280, 720), (1920, 1080)])
        self.assertEqual(size_label("sunrise-m4", 1280, 720), "sunrise-m4")
        self.assertEqual(size_label("sunrise-m4", 1920, 1080), "sunrise-m4-1080p")
        self.assertEqual(size_label("sunrise-m4", 2560, 1080), "sunrise-m4-2560x1080")

    def test_one_options_file_sets_the_render_distance(self):
        self.assertEqual(OPTIONS["renderDistance"].format(distance=16), "16")
        self.assertEqual(OPTIONS["graphicsPreset"], '"custom"')

    def test_table_distinguishes_missing_scene_load_from_zero(self):
        import contextlib
        import io
        import tempfile
        from pathlib import Path
        from unittest import mock
        import tools.perf_bench as pb

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / "benchmarks" / "perf"
            reports.mkdir(parents=True)
            # Comparable runs: table() refuses reports whose conditions differ or are missing.
            conditions = {"scenario": "flight", "seed": "1234", "render_distance": 8, "simulation_distance": 12,
                          "window": [1920, 1080], "vsync": False, "offscreen": False}
            for label, metrics in (
                ("missing", {}),
                ("zero", {"chunks.village.load_ms": {"mean": 0}}),
                ("measured", {"chunks.village.load_ms": {"mean": 1000},
                              "chunks.forest.load_ms": {"mean": 3000}}),
            ):
                (reports / f"{label}-rd8.json").write_text(
                    json.dumps({"conditions": conditions, "metrics": metrics}), encoding="utf-8")
            output = io.StringIO()
            with mock.patch.object(pb, "ROOT", root), contextlib.redirect_stdout(output):
                pb.table(["missing", "zero", "measured"])
        row = next(line for line in output.getvalue().splitlines() if line.startswith("| 8 |"))
        self.assertEqual([cell.strip().split(", ")[1] for cell in row.split("|")[2:-1]], ["–", "0.0", "2.0"])




if __name__ == "__main__":
    unittest.main()


class WaitLoadedTests(unittest.TestCase):
    """wait_loaded on a scripted game with a fake clock: one status per 50 ms poll."""

    def run_script(self, statuses, min_chunks=0):
        import tools.perf_bench as pb
        clock = [0.0]
        it = iter(statuses)

        def control(_command, timeout=None):
            return next(it)

        real_time, real_sleep = pb.time.time, pb.time.sleep
        pb.time.time = lambda: clock[0]
        pb.time.sleep = lambda s: clock.__setitem__(0, clock[0] + s)
        try:
            return pb.wait_loaded(control, lambda p: True, 60, min_chunks)
        finally:
            pb.time.time, pb.time.sleep = real_time, real_sleep

    @staticmethod
    def status(chunks, meshed, stages=None):
        s = {"player": {"position": [0, 0, 0]}, "pending_chunks": 0, "screen": "game",
             "chunks": chunks, "generated_chunks": chunks, "meshed_sections": meshed}
        if stages is not None:
            s["streaming"] = {"queued": 0, "generating": 0, "protos": 296, "decorating": 0,
                              "finishing": 0, "lighting": 0, "meshing": stages}
        return s

    def test_stages_end_the_load_before_the_remesh_tail(self):
        # 20 polls (1 s) of meshing work, then idle stages while blocks keep remeshing
        # at 400 meshes/s, with a short burst of work every 2 s.
        loading = [self.status(900, 1000 * i, stages=500) for i in range(20)]
        tail = [self.status(901, 20000 + 20 * i, stages=40 if i % 40 == 0 else 0) for i in range(400)]
        self.assertAlmostEqual(self.run_script(loading + tail), 1.0, places=6)

    def test_old_builds_wait_for_meshing_to_go_quiet(self):
        busy = [self.status(901, 1000 * i) for i in range(20)]
        quiet = [self.status(901, 20000 + i) for i in range(40)]
        self.assertAlmostEqual(self.run_script(busy + quiet), 1.0, places=6)

    def test_a_quiet_start_is_not_a_load(self):
        # Right after a teleport Sunrise reports steady counts and nothing queued for 1.5 s before it loads:
        # with a target the clock runs on until the chunks are there and the meshing has gone quiet.
        idle = [self.status(3, 0, stages=0) for _ in range(30)]
        loading = [self.status(30 * (i + 1), 500 * i, stages=500) for i in range(20)]
        tail = [self.status(600, 10000, stages=0) for _ in range(60)]
        took = self.run_script(idle + loading + tail, min_chunks=500)
        self.assertGreater(took, 2.4)
        self.assertLess(took, 4.0)
        # Without one (the old call), the quiet start ends the wait at once.
        self.assertLess(self.run_script(idle + loading + tail), 1.6)


class PortabilityTests(unittest.TestCase):
    """What the driver needs from the OS also works where it differs (Windows: no ps, no load average,
    no Unix sockets in Python, nvidia-smi without per-process memory)."""

    def test_resident_memory_of_this_process(self):
        import os
        from tools.perf_bench import rss_bytes
        self.assertGreater(rss_bytes(os.getpid()), 1 << 20)

    def test_load_average_is_a_number_or_absent(self):
        from tools.perf_bench import load_average
        self.assertTrue(load_average() is None or load_average() >= 0)

    def test_report_without_a_load_average(self):
        import argparse
        import tools.perf_bench as pb
        run = {"startup": {"title_s": 1.0, "world_s": 5.0}, "memory": {}, "load_before": None,
               "scenes": {"village": {"load_s": 2.0, "frame_ms": [5.0] * 100, "meshes_load": 10}}}
        args = argparse.Namespace(offscreen=False, settle=2.0, label="x")
        r = pb.report(args, 8, [run], {"name": "sunrise-client"})
        self.assertNotIn("load_avg_1m", r["conditions"])
        self.assertEqual(r["metrics"]["fps.village.avg_fps"]["mean"], 200.0)
        self.assertEqual(pb.report(args, 8, [dict(run, load_before=1.25)], {})["conditions"]["load_avg_1m"], 1.25)
        self.assertTrue(r["hardware"]["os"] and r["hardware"]["cpu_threads"])
        self.assertNotIn("server", r["conditions"])
        # A run on a dedicated server only compares with runs on the same kind of server.
        on_server = argparse.Namespace(offscreen=False, settle=2.0, label="x", server="vanilla")
        self.assertEqual(pb.report(on_server, 8, [run], {})["conditions"]["server"], "vanilla")

    def test_nvidia_memory_not_available(self):
        from unittest import mock
        import subprocess
        import tools.perf_bench as pb
        out = subprocess.CompletedProcess([], 0, stdout="1234, [N/A]\n4321, 812\n")
        with mock.patch.object(pb.shutil, "which", return_value="nvidia-smi"), \
                mock.patch.object(pb.subprocess, "run", return_value=out):
            self.assertIsNone(pb.nvidia_memory_mb(1234))
            self.assertEqual(pb.nvidia_memory_mb(4321), 812.0)
            self.assertIsNone(pb.nvidia_memory_mb(99))

    @unittest.skipUnless(sys.platform == "win32", "the console's OEM code page is Windows'")
    def test_windows_version_reads_the_console_code_page(self):
        # pr_ready runs the tests with PYTHONUTF8=1, and `cmd /C ver` answers in the OEM code page:
        # decoding it as UTF-8 left `stdout` None (issue #5201).
        import os
        import subprocess
        from pathlib import Path
        root = Path(__file__).resolve().parents[1]
        out = subprocess.run(
            [sys.executable, "-c",
             "from tools.perf_bench import windows_version; print(windows_version())"],
            capture_output=True, text=True, encoding="utf-8", errors="replace", check=True,
            env={**os.environ, "PYTHONUTF8": "1"}, cwd=root)
        self.assertIn("Windows", out.stdout)

    def test_control_over_the_tcp_endpoint(self):
        # The game's TCP transport (Windows, or an endpoint path ending in .json): the file names a port and
        # a token, and the first line must be `auth TOKEN`.
        import socket
        import tempfile
        import threading
        from pathlib import Path
        from tools.perf_bench import Control, control_path
        self.assertIn(control_path().suffix, (".sock", ".json"))
        server = socket.create_server(("127.0.0.1", 0))
        lines = []

        def serve():
            for _ in range(2):
                conn, _ = server.accept()
                with conn, conn.makefile("rwb") as f:
                    auth = f.readline().decode().strip()
                    lines.append(auth)
                    if auth != "auth secret":
                        f.write(b'{"error":"bad or missing token"}\n')
                        f.flush()
                        continue
                    f.write(b'{"ok":true}\n')
                    f.flush()
                    lines.append(f.readline().decode().strip())
                    f.write(b'{"screen":"title"}\n')
                    f.flush()

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        with tempfile.TemporaryDirectory() as tmp, server:
            path = Path(tmp) / "control.json"
            port = server.getsockname()[1]
            path.write_text(json.dumps({"transport": "tcp", "port": port, "token": "secret"}))
            self.assertEqual(Control(path)("status", timeout=10), {"screen": "title"})
            path.write_text(json.dumps({"transport": "tcp", "port": port, "token": "wrong"}))
            with self.assertRaises(OSError):
                Control(path)("status", timeout=10)
            thread.join(10)
        self.assertEqual(lines, ["auth secret", "status", "auth wrong"])

    def test_a_release_build_reports_its_own_version(self):
        import tempfile
        from pathlib import Path
        from tools.perf_bench import release_version
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "sunrise-rd8.log"
            log.write_text("Sunrise v0.2.97\nload assets: 1.29s\n")
            self.assertEqual(release_version(log), "v0.2.97")
            log.write_text("Sunrise nightly-2026-10-28-0c02067\n")
            self.assertEqual(release_version(log), "nightly-2026-10-28-0c02067")
            for dev in ("Sunrise 0.1.0\n", "Sunrise v0.2.97 (client 0.1.0)\n"):
                log.write_text(dev)
                self.assertIsNone(release_version(log))
            self.assertIsNone(release_version(Path(tmp) / "missing.log"))

    def test_vanilla_gets_the_manifest_jvm_flags(self):
        from tools.perf_bench import manifest_jvm_flags
        manifest = {"arguments": {"jvm": [
            {"rules": [{"action": "allow", "os": {"name": "osx"}}], "value": ["-XstartOnFirstThread"]},
            {"rules": [{"action": "allow", "os": {"arch": "x86"}}], "value": "-Xss1M"},
            "-XX:StackShadowPages=32", "--enable-native-access=ALL-UNNAMED",
            "-Djava.library.path=${natives_directory}/java", "-cp", "${classpath}"]}}
        self.assertEqual(manifest_jvm_flags(manifest), ["-XX:StackShadowPages=32"])
        self.assertEqual(manifest_jvm_flags({}), [])


class StressSceneTests(unittest.TestCase):
    def test_stress_selects_every_stress_scene_after_the_flights(self):
        import argparse
        from tools import stress_scenes
        from tools.perf_bench import scene_list
        names = [s[0] for s in scene_list(argparse.Namespace(scenes="ocean,stress"))]
        self.assertEqual(names, ["ocean"] + stress_scenes.NAMES)
        self.assertEqual([s[0] for s in scene_list(argparse.Namespace(scenes="stress_mobs"))], ["stress_mobs"])

    def test_scenes_have_their_entity_counts(self):
        from tools import stress_scenes
        summons = lambda name, kind: sum(c.startswith(f"summon {kind} ") for c in stress_scenes.setup(name))
        self.assertEqual(sum(summons("stress_mobs", k) for k in stress_scenes.MOBS), 1000)
        self.assertEqual(sum(summons("stress_mob_ai", k) for k in stress_scenes.MOBS), 500)
        self.assertEqual(summons("stress_items", "item"), 2000)
        self.assertEqual(sum(summons("stress_held", k) for k in stress_scenes.HOLDERS), 1000)
        self.assertEqual(sum(summons("stress_eyes", k) for k in stress_scenes.EYED), 1000)
        self.assertEqual(sum(summons("stress_glowing", k) for k in stress_scenes.MOBS), 1000)
        self.assertEqual(summons("stress_markings", "horse") + summons("stress_markings", "slime"), 1000)
        self.assertEqual(summons("stress_frames", "item_frame"), 512)
        self.assertEqual(summons("stress_armor_stands", "armor_stand"), 500)
        self.assertEqual(summons("stress_avatars", "mannequin"), 300)
        self.assertEqual(stress_scenes.setup("stress_empty"), stress_scenes.SETTINGS + stress_scenes.floor(
            *stress_scenes.origin("stress_empty")))
        # Only the run's first stress scene sets the game rules.
        self.assertEqual(stress_scenes.setup("stress_empty", first=False),
                         stress_scenes.floor(*stress_scenes.origin("stress_empty")))

    def test_every_entity_is_near_enough_to_be_drawn_at_rd16(self):
        # Entity.shouldRenderAtSqrDistance with viewScale 2: a dropped item (0.25) within 32 blocks.
        from tools import stress_scenes
        for name in ("stress_mobs", "stress_mob_ai", "stress_items", "stress_frames", "stress_armor_stands", "stress_avatars"):
            _, at, _, _ = stress_scenes.scene(name)
            eye = (at[0], at[1] + 1.62, at[2])
            for line in stress_scenes.setup(name):
                if line.startswith("summon "):
                    x, y, z = map(float, line.split()[2:5])
                    distance = sum((a - b) ** 2 for a, b in zip((x, y, z), eye)) ** 0.5
                    self.assertLess(distance, 32.0, (name, line))
                    self.assertGreater(z, eye[2], (name, line))

    def test_commands_are_single_lines_inside_the_world(self):
        from tools import stress_scenes
        for name in stress_scenes.NAMES:
            for line in stress_scenes.setup(name):
                self.assertNotIn("\n", line)
                self.assertLess(len(line), 1024)
            self.assertTrue(all(len(c) < 1024 for c in stress_scenes.cleanup(name)))

    def test_cleanup_covers_every_block_a_scene_places(self):
        from tools import stress_scenes
        slabs = [list(map(int, c.split()[1:7])) for c in stress_scenes.cleanup("stress_mobs") if c.startswith("fill")]
        for slab in slabs:
            dx, dy, dz = (abs(slab[i + 3] - slab[i]) + 1 for i in range(3))
            self.assertLessEqual(dx * dy * dz, 32768)  # vanilla's fill limit
        covered = lambda x, y, z: any(s[0] <= x <= s[3] and s[1] <= y <= s[4] and s[2] <= z <= s[5] for s in slabs)
        for name in stress_scenes.NAMES:
            for line in stress_scenes.setup(name):
                words = line.split()
                if words[0] == "fill":
                    a, b = list(map(int, words[1:4])), list(map(int, words[4:7]))
                    for x in (a[0], b[0]):
                        for y in (a[1], b[1]):
                            for z in (a[2], b[2]):
                                self.assertTrue(covered(x, y, z), (name, line))
                elif words[0] == "setblock":
                    self.assertTrue(covered(*map(int, words[1:4])), (name, line))


class CompareTests(unittest.TestCase):
    def test_paired_matches_sunrise_bench(self):
        from tools.perf_bench import paired
        # crates/sunrise-bench's the_decided_vectors, value for value.
        z = [0.0] * 8
        up = [float(v) for v in range(1, 9)]
        cases = [
            (z, up, 4.5, [2.0, 7.0], 0.9609, "slower"),
            (z, [-0.3, 0.2, -0.1, 0.4, -0.2, 0.1, 0.3, -0.4], 0.0, [-0.3, 0.3], 0.9609, "within_noise"),
            ([0.0] * 5, up[:5], 3.0, None, None, "too_few_pairs"),
            ([10.0] * 8, [10.0 + v / 100 for v in range(1, 9)], 0.045, [0.02, 0.07], 0.9609, "negligible"),
            (up, z, -4.5, [-7.0, -2.0], 0.9609, "faster"),
            ([0.0] * 10, [0.3, -0.1, 0.8, 0.2, -0.5, 0.6, 0.1, 0.4, 0.7, 0.05], 0.25, [-0.025, 0.55], 0.9512,
             "within_noise"),
        ]
        for a, b, diff, ci, confidence, verdict in cases:
            p = paired(list(zip(a, b)))
            self.assertEqual((p["diff"], p["ci95"], p["confidence"], p["verdict"]), (diff, ci, confidence, verdict), b)

    def test_judge_pairs_neighbouring_games_and_gates_on_the_budget(self):
        from tools.perf_bench import ARMS, judge

        def games(before, after, noise, figure="p50_ms"):
            out = []
            for i, n in enumerate(noise):
                v = before if ARMS[i % 4] == "A" else after
                out.append({"village": {figure: v + n, "p99_ms": 1.0, "server_tick_p50_ms": 5.0,
                                        "server_tick_p99_ms": 9.0, "cpu_p50_ms": v / 2}})
            return out

        noise = [0.02, -0.01, 0.03, -0.02, 0.01, 0.0, -0.03, 0.02, 0.01, -0.01, 0.02, -0.02]
        same = {r["figure"]: r for r in judge(games(5.0, 5.0, noise), 3, ["village"])}
        self.assertEqual((same["p50_ms"]["verdict"], same["p50_ms"]["passed"]), ("within_noise", True))
        wide = {r["figure"]: r for r in judge(games(5.0, 5.0, [n * 20 for n in noise]), 3, ["village"])}["p50_ms"]
        self.assertEqual((wide["verdict"], wide["interval_within_budget"], wide["passed"]), ("within_noise", False, True),
                         "a wide interval around no change is reported, not failed")
        self.assertEqual((same["cpu_p50_ms"]["gated"], same["cpu_p50_ms"]["passed"]), (False, None))
        slower = {r["figure"]: r for r in judge(games(5.0, 5.25, noise), 3, ["village"])}["p50_ms"]
        self.assertEqual((slower["verdict"], slower["passed"]), ("slower", False), "5% over a 3% budget")
        self.assertTrue({r["figure"]: r for r in judge(games(5.0, 5.25, noise), 8, ["village"])}["p50_ms"]["passed"])
        self.assertFalse(judge(games(5.0, 5.0, noise[:10]), 3, ["village"])[0]["passed"], "five pairs give no interval")

    def test_judge_gates_unrounded_values_and_fails_what_is_missing(self):
        from tools.perf_bench import ARMS, judge
        pick = lambda before, after: [before if ARMS[i % 4] == "A" else after for i in range(12)]
        frame = [{"village": {"p50_ms": v, "p99_ms": 1.0, "server_tick_p50_ms": 5.0, "server_tick_p99_ms": 9.0}}
                 for v in pick(0.24951, 0.25749)]
        row = {r["figure"]: r for r in judge(frame, 3, ["village"])}["p50_ms"]
        self.assertFalse(row["passed"], "3.2% slower, though 0.250 and 0.257 round to under 3%")
        tick = [{"village": {"p50_ms": 1.0, "p99_ms": 1.0, "server_tick_p50_ms": v, "server_tick_p99_ms": 2.0}}
                for v in pick(2.0, 2.06)]
        row = {r["figure"]: r for r in judge(tick, 3, ["village"])}["server_tick_p50_ms"]
        self.assertFalse(row["passed"], "+0.06 ms on 2.0, plus half a 0.1 ms step, is over 3%")
        tick = [{"village": {"p50_ms": 1.0, "p99_ms": 1.0, "server_tick_p50_ms": v, "server_tick_p99_ms": 9.0}}
                for v in pick(5.0, 5.0)]
        row = {r["figure"]: r for r in judge(tick, 3, ["village"])}["server_tick_p50_ms"]
        self.assertTrue(row["passed"], "at 5 ms, half a step is within 3%")

    def test_a_tick_time_below_resolution_is_shown_but_not_gated(self):
        from tools.perf_bench import ARMS, judge
        pick = lambda before, after: [before if ARMS[i % 4] == "A" else after for i in range(12)]
        fast = [{"village": {"p50_ms": f, "p99_ms": 1.0, "server_tick_p50_ms": t, "server_tick_p99_ms": 1.2}}
                for f, t in zip(pick(0.25, 0.30), pick(1.0, 1.4))]
        rows = {r["figure"]: r for r in judge(fast, 3, ["village"])}
        tick = rows["server_tick_p50_ms"]
        self.assertEqual((tick["below_resolution"], tick["gated"], tick["passed"], tick["diff"]), (True, False, None, 0.4),
                         "1.0 ms is below what 0.1 ms steps can judge at 3%: shown, neither pass nor fail")
        self.assertEqual((rows["p50_ms"]["gated"], rows["p50_ms"]["passed"]), (True, False),
                         "the client frame figures are still gated")
        lacking = [{"village": {"p50_ms": 0.25, "p99_ms": 1.0}} for _ in range(12)]
        missing = {r["figure"]: r for r in judge(lacking, 3, ["village"])}["server_tick_p50_ms"]
        self.assertEqual((missing["verdict"], missing["passed"]), ("missing", False), "a missing tick figure still fails")
        lacking = [{"village": {"p50_ms": 1.0, "p99_ms": 1.0}} for _ in range(12)]
        rows = {r["figure"]: r for r in judge(lacking, 3, ["village", "menu_hold"])}
        self.assertEqual((rows["server_tick_p50_ms"]["verdict"], rows["server_tick_p50_ms"]["passed"]), ("missing", False))
        menu = [r for r in judge(lacking, 3, ["menu_hold"])]
        self.assertEqual({(r["figure"], r["verdict"]) for r in menu}, {("p50_ms", "missing"), ("p99_ms", "missing")},
                         "menu_hold gates frame time only")

    def test_tick_query_percentiles(self):
        from tools.perf_bench import parse_tick_query
        lines = ["The game is running normally", "Target tick rate: 20.0 per second.\nAverage time per tick: 3.1ms (Target: 50.0ms)",
                 "Percentiles: P50: 2.9ms P95: 4.2ms P99: 6.0ms. Sample: 100"]
        self.assertEqual(parse_tick_query(lines), {"p50_ms": 2.9, "p95_ms": 4.2, "p99_ms": 6.0, "samples": 100})
        self.assertIsNone(parse_tick_query(["Unknown command"]))

    def test_a_misspelled_or_empty_scene_is_refused(self):
        from tools.perf_bench import unknown_scenes
        self.assertEqual(unknown_scenes("village,forrest"), ["forrest"])
        self.assertEqual(unknown_scenes(""), [""])
        self.assertEqual(unknown_scenes("village,,ocean"), [""])
        self.assertEqual(unknown_scenes("stress,menu_hold,mountain,stress_mobs"), [])

    def test_server_ticks_count_only_the_flights_own_ticks(self):
        from tools.perf_bench import game_time, server_ticks
        replies = {"command time query gametime": {"ok": True, "chat": [], "success": True, "result": 48210},
                   "command tick query": {"chat": ["Percentiles: P50: 2.9ms P95: 4.2ms P99: 6.0ms. Sample: 100"]}}

        def control(verb):
            return replies[verb]

        self.assertEqual(game_time(control), 48210)
        with self.assertRaisesRegex(RuntimeError, "needs a longer --seconds"):
            server_ticks(control, 99)
        self.assertEqual(server_ticks(control, 100)["p50_ms"], 2.9)

    def test_game_time_is_the_commands_result_not_its_chat_text(self):
        from tools.perf_bench import parse_game_time
        # The game answers "The game time is 48210 tick(s)" and the control reply carries no such line.
        self.assertEqual(parse_game_time({"ok": True, "chat": [], "success": True, "result": 48210}), 48210)

    def test_an_unparseable_game_time_or_tick_query_raises(self):
        from tools.perf_bench import game_time, parse_game_time, server_ticks
        for reply in ({"ok": True, "chat": ["The game time is 48210 tick(s)"], "success": False, "result": None},
                      {"ok": True, "chat": [], "success": False, "result": 0},
                      {"ok": True, "chat": [], "success": True, "result": None},
                      {"ok": True, "chat": [], "success": True, "result": "48210"},
                      {"ok": True, "chat": [], "success": True, "result": True}, {}):
            with self.assertRaisesRegex(RuntimeError, "no game time"):
                parse_game_time(reply)
        with self.assertRaisesRegex(RuntimeError, "no game time"):
            game_time(lambda verb: {"chat": ["The time is 48210"]})
        with self.assertRaisesRegex(RuntimeError, "no percentiles"):
            server_ticks(lambda verb: {"chat": ["Unknown command"]}, 240)

    def test_ungated_tick_figures_are_listed_for_the_summary(self):
        from tools.perf_bench import ungated_ticks
        rows = [{"scene": "village", "figure": "server_tick_p50_ms", "below_resolution": True},
                {"scene": "village", "figure": "p50_ms", "below_resolution": False},
                {"scene": "ocean", "figure": "server_tick_p99_ms", "below_resolution": False}]
        self.assertEqual(ungated_ticks(rows), ["village server_tick_p50_ms"])

    def test_game_folder_gets_graphics_and_packs(self):
        import argparse
        import tempfile
        from pathlib import Path
        from tools.perf_bench import game_folder_extras
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / "Relief").mkdir()
            (tmp / "Relief" / "pack.mcmeta").write_text("{}")
            (tmp / "golden").mkdir()
            (tmp / "dusk.zip").write_bytes(b"PK")
            game = tmp / "game"
            game.mkdir()
            (game / "options.txt").write_text("renderDistance:8\n")
            args = argparse.Namespace(graphics=["fog:false", "gpuCulling:true"], resource_pack=[str(tmp / "Relief")],
                                      shader_pack=str(tmp / "golden"))
            lines, env = game_folder_extras(args, game)
            self.assertEqual((game / "sunrise-graphics.txt").read_text(), "fog:false\ngpuCulling:true\n")
            self.assertEqual((game / "options.txt").read_text(), 'renderDistance:8\nresourcePacks:["vanilla","file/Relief"]\n')
            self.assertEqual(lines, 'resourcePacks:["vanilla","file/Relief"]\n')
            self.assertTrue((game / "resourcepacks/Relief/pack.mcmeta").exists())
            self.assertTrue((game / "shaderpacks/golden").is_dir())
            self.assertEqual(env, {"SUNRISE_SHADER_PACK": "golden"})
            zipped = tmp / "fresh"
            zipped.mkdir()
            (zipped / "options.txt").write_text("")
            args.shader_pack, args.resource_pack, args.graphics = str(tmp / "dusk.zip"), None, None
            self.assertEqual(game_folder_extras(args, zipped), ("", {"SUNRISE_SHADER_PACK": "dusk.zip"}))
            self.assertTrue((zipped / "shaderpacks/dusk.zip").is_file())
