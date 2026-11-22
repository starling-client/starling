"""The scheduled watch cancels runs stranded behind a dead pool, and only those."""
from datetime import datetime, timezone
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools/ci"))
import dyna_ci_pool_watch as watch

NOW = 1_800_000_000
STALL = 20 * 60


def stamp(minutes_ago):
    return datetime.fromtimestamp(NOW - minutes_ago * 60, timezone.utc).isoformat()


def run(ident, status, minutes_ago, updated=None):
    return {"id": ident, "status": status, "created_at": stamp(minutes_ago), "name": f"run {ident}",
            "updated_at": stamp(minutes_ago if updated is None else updated),
            "html_url": f"https://example.invalid/{ident}"}


def job(status, label="dyna-large", started=None, completed=None, runner=None):
    return {"status": status, "labels": [label], "started_at": started and stamp(started),
            "completed_at": completed and stamp(completed), "runner_name": runner}


class Watch(unittest.TestCase):
    def sweep(self, runs, jobs, me=999, later=None):
        """`later` changes `runs` and `jobs` during the pause between the two looks."""
        requests, cancelled, sleeps = [], [], []

        def sleep(seconds):
            sleeps.append(seconds)
            if later:
                later(runs, jobs)

        def api(path):
            requests.append(path)
            if "/jobs?" in path:
                return [{"jobs": jobs[int(path.split("/runs/")[1].split("/")[0])]}]
            status = path.split("status=")[1].split("&")[0]
            listed = [entry for entry in runs if entry["status"] == status]
            if "created=%3E%3D" in path:
                since = datetime.fromisoformat(path.split("created=%3E%3D")[1].split("&")[0])
                listed = [entry for entry in listed if datetime.fromisoformat(entry["created_at"]) >= since]
            return [{"workflow_runs": listed}]

        code = watch.watch(api, cancelled.append, lambda: NOW, sleep, "o/r", me, STALL)
        return code, cancelled, sleeps, requests

    def test_a_dead_pool_has_its_stranded_runs_cancelled_after_a_second_quiet_look(self):
        runs = [run(1, "queued", 45), run(2, "in_progress", 30), run(3, "queued", 5)]
        jobs = {1: [job("queued")],
                # Its routing job finished on GitHub's runner; its gate job has waited for the pool since.
                2: [job("completed", "ubuntu-latest", 29, 28, "GitHub Actions 1"), job("queued")],
                3: [job("queued")]}
        code, cancelled, sleeps, _ = self.sweep(runs, jobs)
        self.assertEqual((code, cancelled, sleeps), (1, [1, 2], [watch.QUIET_RECHECK]))

    def test_a_pool_that_just_finished_a_whole_run_is_alive(self):
        # Run 2 ended on a Dyna runner a minute ago: only the finished runs show that.
        runs = [run(1, "queued", 45), run(2, "completed", 30, updated=1), run(4, "completed", 40, updated=30)]
        jobs = {1: [job("queued")], 2: [job("completed", started=15, completed=1, runner="dyna-ci-1")],
                4: [job("completed", started=35, completed=30, runner="dyna-ci-3")]}
        code, cancelled, _, requests = self.sweep(runs, jobs)
        self.assertEqual((code, cancelled), (0, []))
        # A finished run not updated since the stall is not read.
        self.assertFalse([path for path in requests if "/runs/4/" in path])

    def test_a_rerun_of_an_old_run_that_finishes_on_the_pool_between_the_looks_is_alive(self):
        # Run 3 was created two days ago and rerun: its pool job waits behind a runner starting up at the first look,
        # and has run and finished on the pool by the second. It is the only sign of life.
        runs = [run(1, "queued", 45), run(3, "in_progress", 2880, updated=2)]
        jobs = {1: [job("queued")],
                3: [job("completed", "ubuntu-latest", 3, 2, "GitHub Actions 3"), job("queued")]}

        def finish(runs, jobs):
            runs[1] = run(3, "completed", 2880, updated=1)
            jobs[3][1] = job("completed", started=2, completed=1, runner="dyna-ci-2")

        self.assertEqual(self.sweep(runs, jobs, later=finish)[:2], (0, []))

    def test_a_busy_pool_cancels_nothing(self):
        runs = [run(1, "queued", 45), run(2, "in_progress", 30)]
        jobs = {1: [job("queued")], 2: [job("in_progress", started=10, runner="dyna-ci-1")]}
        self.assertEqual(self.sweep(runs, jobs)[:3], (0, [], []))

    def test_a_pool_that_finished_a_job_within_the_stall_is_alive(self):
        runs = [run(1, "queued", 45), run(2, "in_progress", 40)]
        jobs = {1: [job("queued")], 2: [job("completed", started=30, completed=10, runner="dyna-ci-1"), job("queued")]}
        self.assertEqual(self.sweep(runs, jobs)[:3], (0, [], []))

    def test_a_job_that_never_held_a_runner_is_no_sign_of_life(self):
        # A cancelled queued job has start and finish times but no runner.
        runs = [run(1, "in_progress", 45)]
        jobs = {1: [job("completed", started=25, completed=25), job("queued")]}
        self.assertEqual(self.sweep(runs, jobs)[:2], (1, [1]))

    def test_a_run_with_recent_progress_or_no_pool_job_is_left_alone(self):
        runs = [run(1, "in_progress", 45), run(2, "queued", 45)]
        jobs = {1: [job("completed", "ubuntu-latest", 12, 5, "GitHub Actions 2"), job("queued")],
                2: [job("queued", "ubuntu-latest")]}
        self.assertEqual(self.sweep(runs, jobs)[:3], (0, [], []))

    def test_the_watch_skips_its_own_run_and_reads_no_young_queued_run(self):
        runs = [run(999, "in_progress", 45), run(3, "queued", 5)]
        jobs = {999: [job("queued")]}
        code, cancelled, _, requests = self.sweep(runs, jobs)
        self.assertEqual((code, cancelled), (0, []))
        self.assertFalse([path for path in requests if "/jobs?" in path])


    def test_paginated_pages_are_read_without_slurp(self):
        self.assertEqual(watch.pages('{"total_count": 2}\n{"total_count": 0}'), [{"total_count": 2}, {"total_count": 0}])


if __name__ == "__main__":
    unittest.main()
