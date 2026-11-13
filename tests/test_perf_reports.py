"""The performance reports' summary (packaging/android/perf_reports.py)."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "packaging" / "android" / "perf_reports.py"
spec = importlib.util.spec_from_file_location("perf_reports", SCRIPT)
perf_reports = importlib.util.module_from_spec(spec)
spec.loader.exec_module(perf_reports)


def report(gpu, steps, schema=1):
    return {
        "schema": schema,
        "kind": "guided",
        "device": {"gpu": gpu},
        "guided": [
            {"choices": choices, "summary": {"fps": fps, "low_1pct_fps": low}}
            for choices, fps, low in steps
        ],
    }


class Summary(unittest.TestCase):
    def test_ways_are_grouped_by_gpu_fastest_first(self):
        with tempfile.TemporaryDirectory() as folder:
            lines = [
                report("Mali-G57", [({"draws": "auto"}, 40, 30), ({"draws": "direct"}, 50, 35)]),
                report("Mali-G57", [({"draws": "auto"}, 44, 31), ({"draws": "direct"}, 52, 36)]),
                report("Adreno 740", [({"late_sky": "off"}, 90, 60)]),
                report("Mali-G57", [({"draws": "direct"}, 999, 999)], schema=0),
                {"schema": 1, "kind": "setups", "device": {"gpu": "Mali-G57"}, "setups": []},
            ]
            text = "\n".join(json.dumps({"received": 1, "report": r}) for r in lines)
            (Path(folder) / "2026-11-13.jsonl").write_text(text + "\nnot json\n", encoding="utf-8")
            out = perf_reports.lines(perf_reports.guided(folder))
        self.assertEqual(out[0], "Adreno 740")
        self.assertIn("late sky off", out[1])
        self.assertEqual(out[2], "Mali-G57")
        self.assertTrue(out[3].lstrip().startswith("direct"))
        self.assertIn("2 runs", out[3])
        self.assertIn("51.0 fps", out[3])
        self.assertTrue(out[4].lstrip().startswith("auto"))


if __name__ == "__main__":
    unittest.main()
