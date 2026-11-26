#!/usr/bin/env python3
"""Export the modding book's generated chapters and SDK tables for the site sync."""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools import modding_hook_notes as notes, modding_sdk_tables as tables


def export(out):
    """Render from this checkout before writing the files the site sync consumes."""
    context = {"root": str(ROOT / "docs/modding")}
    sdk_tables = {name: make() for name, make in tables.generators(ROOT).items()}
    files = {
        notes.PAGE: notes.generated(context, "notes"),
        "reference/neoforge-counterparts.md": notes.generated(context, "counterparts"),
        "reference/api-reference.md": notes.generated(context, "reference"),
        "sdk-tables.json": json.dumps(sdk_tables, indent=2) + "\n",
    }
    for name, text in files.items():
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    export(parser.parse_args().out)


if __name__ == "__main__":
    main()
