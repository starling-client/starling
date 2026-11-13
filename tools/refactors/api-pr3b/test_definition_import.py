#!/usr/bin/env python3
"""Run the importer regression and production-load/allocation probe together."""
from pathlib import Path
import runpy
runpy.run_path(str(Path(__file__).with_name("test_block_key_load_boundary.py")), run_name="__main__")
