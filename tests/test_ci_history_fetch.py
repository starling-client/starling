"""Execute both gate history-fetch steps with controlled Git/network results, and the slot mirror with real Git."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def steps():
    import yaml
    workflow = yaml.load((ROOT / ".github/workflows/dyna-ci.yml").read_text(), Loader=yaml.BaseLoader)["jobs"]
    for job, name in [("changes", "Fetch this commit's history for selection"),
                      ("verify-static", "Fetch this commit's history")]:
        yield job, name, next(s for s in workflow[job]["steps"] if s.get("name") == name)


@unittest.skipIf(os.name == "nt", "executes Bash and executable test doubles")
@unittest.skipIf(importlib.util.find_spec("yaml") is None, "requires PyYAML")
class HistoryFetchTests(unittest.TestCase):
    def test_bounded_fetch_retries_and_fails_closed(self):
        for job, name, step in steps():
            for mode, expected, count in [("success", 0, 1), ("retry", 0, 2),
                                           ("failure", 1, 2), ("complete", 0, 0)]:
                with self.subTest(job=job, mode=mode), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    git = root / "git"
                    git.write_text(f"#!{sys.executable}\n"
                                   "import os, pathlib, sys\n"
                                   "mode = os.environ['MODE']\n"
                                   "if sys.argv[1] == 'rev-parse':\n"
                                   "    print(('false' if mode == 'complete' else 'true') if sys.argv[2] == '--is-shallow-repository' else 'abc123')\n"
                                   "    sys.exit(0)\n"
                                   "p = pathlib.Path(os.environ['CALLS'])\n"
                                   "n = int(p.read_text()) + 1 if p.exists() else 1\n"
                                   "p.write_text(str(n))\n"
                                   "sys.exit(1 if mode == 'failure' or (mode == 'retry' and n == 1) else 0)\n")
                    timeout = root / "timeout"
                    timeout.write_text(f"#!{sys.executable}\n"
                                       "import json, os, sys\n"
                                       "with open(os.environ['LIMITS'], 'a') as f: f.write(json.dumps(sys.argv[1:3]) + '\\n')\n"
                                       "os.execvp(sys.argv[3], sys.argv[3:])\n")
                    git.chmod(0o755)
                    timeout.chmod(0o755)
                    # Off our runners: no slot, so no mirror.
                    env = dict(os.environ, **step["env"], MODE=mode, DYNA_CI_BUILD="", DYNA_CI_KEEP="",
                               PATH=directory + os.pathsep + os.environ["PATH"],
                               CALLS=str(root / "calls"), LIMITS=str(root / "limits"))
                    result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"]], cwd=ROOT,
                                            env=env, capture_output=True, text=True, timeout=5)
                    self.assertEqual(result.returncode, expected, result.stderr)
                    actual = int((root / "calls").read_text()) if (root / "calls").exists() else 0
                    self.assertEqual(actual, count)
                    limits = (root / "limits").read_text().splitlines() if count else []
                    self.assertEqual([json.loads(line) for line in limits], [["--kill-after=10s", "300s"]] * count)
                    if expected:
                        self.assertIn(f"::error::{name}:", result.stdout)
                        self.assertIn("GitHub failed or timed out twice", result.stdout)
                    else:
                        self.assertNotIn("::error::", result.stdout)

    @unittest.skipUnless(shutil.which("git") and shutil.which("timeout"), "needs git and timeout")
    def test_selection_takes_the_commits_and_fetches_only_the_trees_its_diff_reads(self):
        _, name, step = next(steps())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            quiet = dict(capture_output=True, text=True, check=True)
            git = lambda *args, cwd=root: subprocess.run(["git", *args], cwd=cwd, **quiet).stdout.strip()
            server = root / "server"
            git("init", "-q", "-b", "main", str(server))
            git("config", "uploadpack.allowFilter", "true", cwd=server)
            for n in range(6):
                (server / f"d{n}").mkdir()
                (server / f"d{n}" / "f").write_text(f"{n}\n")
                git("add", ".", cwd=server)
                git("-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-qm", f"c{n}",
                    cwd=server)
            heads = git("rev-list", "--reverse", "main", cwd=server).split()
            build = root / "build"
            checkout = root / "checkout"
            git("init", "-q", str(checkout))
            git("remote", "add", "origin", f"file://{server}", cwd=checkout)
            git("fetch", "-q", "--depth=1", "origin", heads[5], cwd=checkout)
            git("checkout", "-q", "FETCH_HEAD", cwd=checkout)
            env = dict(os.environ, **step["env"], DYNA_CI_BUILD=str(build), DYNA_CI_KEEP="",
                       GITHUB_REPOSITORY="org/repo")
            result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"].replace(
                "tools/ci/", f"{ROOT}/tools/ci/")], cwd=checkout, env=env, capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(git("rev-list", "--count", "HEAD", cwd=checkout), "6")
            # The older commits came without their trees, and the slot's mirror was neither borrowed nor filled.
            missing = git("rev-list", "--objects", "--missing=print", heads[2], cwd=checkout).split()
            self.assertIn(f"?{git('rev-parse', heads[2] + '^{tree}', cwd=server)}", missing)
            self.assertFalse(build.exists())
            # Selection's diff still reads both trees, fetching what it lacks.
            self.assertEqual(git("diff", "--name-only", f"{heads[2]}...{heads[5]}", cwd=checkout).split(),
                             ["d3/f", "d4/f", "d5/f"])

    @unittest.skipUnless(shutil.which("git") and shutil.which("timeout"), "needs git and timeout")
    def test_the_slot_mirror_fills_then_lends_its_history_to_the_next_checkout(self):
        _, name, step = list(steps())[1]
        # Our pool and main's pushes take the whole history: no filter.
        step = dict(step, env=dict(step["env"], HISTORY_FILTER=""))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            quiet = dict(capture_output=True, text=True, check=True)
            git = lambda *args, cwd=root: subprocess.run(["git", *args], cwd=cwd, **quiet).stdout.strip()
            server = root / "server"
            git("init", "-q", "-b", "main", str(server))
            for n in range(6):
                (server / "f").write_text(f"{n}\n")
                git("add", "f", cwd=server)
                git("-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-qm", f"c{n}",
                    cwd=server)
            heads = git("rev-list", "--reverse", "main", cwd=server).split()
            build = root / "build"
            mirror = build / "git-mirror" / "org" / "repo.git"
            # With only the slot's folder given (no $DYNA_CI_KEEP), the slot keeps a mirror of its own.
            env = dict(os.environ, **step["env"], DYNA_CI_BUILD=str(build), DYNA_CI_KEEP="",
                       GITHUB_REPOSITORY="org/repo")

            def job(commit, label):
                checkout = root / label
                git("init", "-q", str(checkout))
                git("remote", "add", "origin", f"file://{server}", cwd=checkout)
                git("fetch", "-q", "--depth=1", "origin", commit, cwd=checkout)
                git("checkout", "-q", "FETCH_HEAD", cwd=checkout)
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"].replace(
                    "tools/ci/", f"{ROOT}/tools/ci/")], cwd=checkout, env=env, capture_output=True, text=True,
                    timeout=60)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn("::error::", result.stdout)
                self.assertEqual(git("rev-parse", "--is-shallow-repository", cwd=checkout), "false")
                self.assertEqual(git("rev-list", "--count", "HEAD", cwd=checkout), str(heads.index(commit) + 1))
                job.output = result.stdout
                return checkout

            # The slot's first job has no mirror: it fetches everything, then leaves its head in the mirror.
            first = job(heads[3], "first")
            self.assertFalse((first / ".git/objects/info/alternates").exists())
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), heads[3])
            # The next job borrows the mirror's objects and takes only what is new; the mirror moves to its head.
            second = job(heads[5], "second")
            self.assertEqual((second / ".git/objects/info/alternates").read_text(), f"{mirror}/objects\n")
            subprocess.run(["git", "fsck", "--connectivity-only"], cwd=second, **quiet)
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), heads[5])
            # With no time left in the step, the job still passes and the mirror stays where it was.
            env["UPKEEP_BUDGET"] = "0"
            job(heads[4], "late")
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), heads[5])
            del env["UPKEEP_BUDGET"]
            # A mirror that no longer reads as a repository is not borrowed, and takes this job's head again.
            shutil.rmtree(mirror / "refs")
            third = job(heads[4], "third")
            self.assertFalse((third / ".git/objects/info/alternates").exists())
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), heads[4])

            # A stand-in `timeout` answers the calls a rule names with an exit status instead of running them: the
            # first rule a set number of times, the second every time.
            stand_in = root / "bin"
            stand_in.mkdir()
            (stand_in / "timeout").write_text(
                f"#!{sys.executable}\n"
                "import os, pathlib, sys\n"
                "args = sys.argv[1:]\n"
                "while args[0].startswith('--'):\n"
                "    args.pop(0)\n"
                "args = args[1:]\n"
                "call = ' '.join(args)\n"
                "part, status, times = os.environ['ANSWER'].split('|')\n"
                "count = pathlib.Path(os.environ['ANSWERED'])\n"
                "n = int(count.read_text()) if count.exists() else 0\n"
                "if part in call and n < int(times):\n"
                "    count.write_text(str(n + 1))\n"
                "    sys.exit(int(status))\n"
                "always = os.environ.get('ALWAYS')\n"
                "if always and always.split('|')[0] in call:\n"
                "    sys.exit(int(always.split('|')[1]))\n"
                "os.execvp(args[0], args)\n")
            (stand_in / "timeout").chmod(0o755)
            env["PATH"] = f"{stand_in}{os.pathsep}{env['PATH']}"

            def answered(part, status, always=None):
                (root / "answered").unlink(missing_ok=True)
                env.update(ANSWER=f"{part}|{status}|1", ANSWERED=str(root / "answered"), ALWAYS=always or "")

            # The mirror's update runs out of time (124 from timeout, 137 once it has to kill): the job passes and
            # the mirror stays, at its old head, for the next job.
            for status, label in ((124, "fetch-timeout"), (137, "fetch-killed")):
                answered("refs/heads/ci-head", status)
                job(heads[5], label)
                self.assertIn("out of time to update the history mirror", job.output)
                self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), heads[4])
            # A repack cut short keeps the update it follows.
            answered(" gc --auto", 124)
            job(heads[5], "gc-timeout")
            self.assertIn("was not repacked this time", job.output)
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), heads[5])
            # The first remote fetch fails while borrowing: the retry runs without the mirror, and a mirror that then
            # can't take the head (a real error, not a timeout) is dropped, as no checkout borrows from it any more.
            answered("--unshallow", 1, always="refs/heads/ci-head|1")
            retried = job(heads[4], "retried")
            self.assertIn("retrying without the history mirror", job.output)
            self.assertNotIn(f"{mirror}/objects", (retried / ".git/objects/info/alternates").read_text())
            self.assertIn("could not take this head", job.output)
            self.assertFalse(mirror.exists())

    @unittest.skipUnless(shutil.which("git") and shutil.which("timeout") and shutil.which("flock"),
                         "needs git, timeout and flock")
    def test_the_hosts_shared_mirror_serves_every_slot_and_keeps_what_each_checkout_borrowed(self):
        _, name, step = list(steps())[1]
        step = dict(step, env=dict(step["env"], HISTORY_FILTER=""))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            quiet = dict(capture_output=True, text=True, check=True)
            git = lambda *args, cwd=root: subprocess.run(["git", *args], cwd=cwd, **quiet).stdout.strip()
            server = root / "server"
            git("init", "-q", "-b", "main", str(server))

            def commit(n):
                (server / "f").write_text(f"{n}\n")
                git("add", "f", cwd=server)
                git("-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-qm", n,
                    cwd=server)
                return git("rev-parse", "HEAD", cwd=server)

            trunk = [commit(f"c{n}") for n in range(3)]
            # A side branch whose commits are on no later head: a pull request's merge that main never takes.
            git("checkout", "-q", "-b", "side", trunk[1], cwd=server)
            side = [commit(f"s{n}") for n in range(3)]
            git("checkout", "-q", "main", cwd=server)
            later = [commit(f"c{n}") for n in range(3, 5)]
            keep = root / "keep"
            mirror = keep / "git-mirror" / "org" / "repo.git"

            def job(head, label, **extra):
                # Each job on a slot of its own: a fresh, empty slot folder.
                checkout, slot = root / label, root / f"slot-{label}"
                slot.mkdir()
                git("init", "-q", str(checkout))
                git("remote", "add", "origin", f"file://{server}", cwd=checkout)
                git("fetch", "-q", "--depth=1", "origin", head, cwd=checkout)
                git("checkout", "-q", "FETCH_HEAD", cwd=checkout)
                env = dict(os.environ, **step["env"], DYNA_CI_BUILD=str(slot), DYNA_CI_KEEP=str(keep),
                           GITHUB_REPOSITORY="org/repo", **extra)
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"].replace(
                    "tools/ci/", f"{ROOT}/tools/ci/")], cwd=checkout, env=env, capture_output=True, text=True,
                    timeout=60)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(git("rev-parse", "--is-shallow-repository", cwd=checkout), "false")
                self.assertFalse((slot / "git-mirror").exists())
                return checkout

            def whole(checkout):
                subprocess.run(["git", "fsck", "--connectivity-only"], cwd=checkout, **quiet)

            job(trunk[2], "first")
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), trunk[2])
            # A job on the side branch, on another freshly emptied slot, borrows the mirror and moves it there.
            on_side = job(side[2], "side")
            self.assertEqual((on_side / ".git/objects/info/alternates").read_text(), f"{mirror}/objects\n")
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), side[2])
            # A job borrowing at the side head, still reading history, while a later main head replaces it and
            # the mirror is repacked with everything unreachable pruned: what it borrowed is pinned and stays.
            reader = job(side[2], "reader")
            job(later[1], "later")
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), later[1])
            git("-C", str(mirror), "-c", "gc.reflogExpire=now", "-c", "gc.reflogExpireUnreachable=now", "gc",
                "--prune=now", "-q")
            whole(reader)
            self.assertEqual(git("rev-list", "--count", "HEAD", cwd=reader), "5")
            # A pin older than PIN_DAYS goes with the next update; the recent ones, the reader's among them, stay.
            pins = lambda: set(git("for-each-ref", "--format=%(refname)", "refs/borrowed/", cwd=mirror).split())
            git("update-ref", "refs/borrowed/1000-1-1-1", trunk[0], cwd=mirror)
            recent = pins() - {"refs/borrowed/1000-1-1-1"}
            self.assertEqual(len(recent), 3)
            job(later[1], "expiry")
            self.assertEqual(len(pins()), 4)
            self.assertTrue(recent < pins())
            # While another job holds the update lock, this one fetches as usual and leaves the mirror alone.
            holder = subprocess.Popen(["flock", "-x", f"{mirror}.lock", "-c", "echo held; exec sleep 30"],
                                      stdout=subprocess.PIPE, text=True, start_new_session=True)
            try:
                self.assertEqual(holder.stdout.readline(), "held\n")
                whole(job(trunk[2], "locked"))
            finally:
                # The lock is held by flock's child too: end the whole group, then wait until the lock is free.
                os.killpg(holder.pid, 9)
                holder.wait()
                subprocess.run(["flock", "-x", f"{mirror}.lock", "true"], check=True, timeout=30)
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), later[1])
            # A shared mirror that can't take a head stays: other jobs' checkouts may be borrowing from it.
            stand_in = root / "bin"
            stand_in.mkdir()
            (stand_in / "timeout").write_text(
                f"#!{sys.executable}\n"
                "import os, sys\n"
                "args = sys.argv[1:]\n"
                "while args[0].startswith('--'):\n"
                "    args.pop(0)\n"
                "args = args[1:]\n"
                "if 'refs/heads/ci-head' in ' '.join(args):\n"
                "    sys.exit(1)\n"
                "os.execvp(args[0], args)\n")
            (stand_in / "timeout").chmod(0o755)
            whole(job(trunk[0], "refused", PATH=f"{stand_in}{os.pathsep}{os.environ['PATH']}"))
            self.assertEqual(git("rev-parse", "refs/heads/ci-head", cwd=mirror), later[1])

if __name__ == "__main__":
    unittest.main()
