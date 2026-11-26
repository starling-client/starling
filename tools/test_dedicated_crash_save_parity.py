#!/usr/bin/env python3
"""Expose dedicated player-save and crash-save checks to the parity runner."""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.test_dedicated_crash_save import main


if __name__ == '__main__':
    raise SystemExit(main())
