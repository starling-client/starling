"""The one loader of the vendored PyYAML (tools/ci/vendor/pyyaml-6.0.3.zip).

Every tool and test that parses YAML takes the `yaml` module from `load()`, which checks the archive's sha256 before
Python imports it. Never from site packages. The sha is pinned here and in vendor/README.md (update both with the
archive; see tools/ci/vendor_pyyaml.py).
"""
import functools
import hashlib
import sys
from pathlib import Path

ARCHIVE = Path(__file__).resolve().parent / "vendor/pyyaml-6.0.3.zip"
SHA256 = "94ac21b4343e15647b3b5b5bd594aab5b7d3cd8d2ce17db15acb7260d489b6fc"


@functools.cache
def load():
    """The `yaml` module imported from ARCHIVE. A missing archive raises OSError, a changed one ImportError."""
    if hashlib.sha256(ARCHIVE.read_bytes()).hexdigest() != SHA256:
        raise ImportError(f"vendored YAML parser checksum mismatch: {ARCHIVE} is not the pinned PyYAML 6.0.3")
    sys.path.insert(0, str(ARCHIVE))
    import yaml
    return yaml
