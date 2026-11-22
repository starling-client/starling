"""The diagnostics window (DiagnosticsActivity) and the native library it loads in its own process."""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
JAVA = ROOT / "packaging/android/java/io/github/juanluci/starling/DiagnosticsActivity.java"


class NativeThread(unittest.TestCase):
    def test_one_thread_calls_the_library_for_the_whole_process(self):
        # mimalloc counts the thread that loads the library as the main thread and frees the main
        # heap when it ends; with a thread per report, a later one that got the same thread pointer
        # counted as main and crashed in mi_thread_init (SIGSEGV at 0x18).
        java = JAVA.read_text(encoding="utf-8")
        self.assertIn("Executors.newSingleThreadExecutor", java)
        self.assertIn("NATIVE.execute(", java)
        self.assertNotIn("new Thread(new Runnable", java)


if __name__ == "__main__":
    unittest.main()
