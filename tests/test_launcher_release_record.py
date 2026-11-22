"""check_launcher_release_record.py signs off a launcher draft only when it is exactly the release release.yml attested."""
import copy
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("record", Path(__file__).resolve().parents[1] / "tools/ci/check_launcher_release_record.py")
record = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(record)

COMMIT = "a" * 40
FILES = {"sunrise-launcher-linux-x86_64.tar.gz": "1" * 64, "sunrise-launcher-setup.exe": "2" * 64}


def attested(tag="v1.1.0", files=FILES, commit=COMMIT, trigger="workflow_dispatch"):
    return {"verificationResult": {
        "statement": {"predicateType": record.PREDICATE, "predicate": {"tag": tag, "version": tag[1:], "commit": commit},
                      "subject": [{"name": name, "digest": {"sha256": digest}} for name, digest in files.items()]},
        "signature": {"certificate": {"buildTrigger": trigger, "buildSignerURI": record.SIGNER, "sourceRepositoryRef": "refs/heads/main",
                                      "sourceRepositoryDigest": commit, "runnerEnvironment": "github-hosted"}}}}


class LauncherReleaseRecord(unittest.TestCase):
    def passes(self, results, files=FILES, sums=FILES, tag="v1.1.0"):
        try:
            record.check(results, tag, COMMIT, files, sums)
            return True
        except SystemExit:
            return False

    def test_the_attested_release_passes(self):
        self.assertTrue(self.passes([attested()]))

    def test_a_missing_renamed_or_swapped_file_is_refused(self):
        missing = dict(list(FILES.items())[:1])
        self.assertFalse(self.passes([attested()], files=missing, sums=missing))
        renamed = {"sunrise-launcher-windows-x64.zip" if n.endswith(".exe") else n: d for n, d in FILES.items()}
        self.assertFalse(self.passes([attested()], files=renamed, sums=renamed))
        swapped = dict(zip(FILES, reversed(list(FILES.values()))))
        self.assertFalse(self.passes([attested()], files=swapped, sums=swapped))

    def test_another_version_commit_or_trigger_is_refused(self):
        self.assertFalse(self.passes([attested(tag="v1.0.9")]))
        self.assertFalse(self.passes([attested(commit="b" * 40)]))
        self.assertFalse(self.passes([attested(trigger="push")]))
        foreign = copy.deepcopy(attested())
        foreign["verificationResult"]["signature"]["certificate"]["buildSignerURI"] = record.SIGNER.replace("main", "other")
        self.assertFalse(self.passes([foreign]))

    def test_sha256sums_must_list_the_same_files(self):
        self.assertFalse(self.passes([attested()], sums={**FILES, "sunrise-launcher-extra.zip": "3" * 64}))

    def test_a_reruns_identical_records_pass_and_disagreeing_ones_are_refused(self):
        self.assertTrue(self.passes([attested(), attested()]))
        other = {**FILES, "sunrise-launcher-setup.exe": "3" * 64}
        self.assertFalse(self.passes([attested(), attested(files=other)]))
        self.assertFalse(self.passes([attested(), attested(commit="b" * 40)]))
        self.assertFalse(self.passes([]))


if __name__ == "__main__":
    unittest.main()
