import contextlib
import io
from pathlib import Path
import struct
import tempfile
import unittest

from tools import check_imports


def elf(interpreter, libraries, runpath=None):
    """A minimal ELF64; `runpath` adds a DT_RUNPATH."""
    data = bytearray(4096)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<HH", data, 54, 56, 3 if interpreter else 2)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 4, 0, 0x400000, 0, len(data), len(data), 4096)
    strings = bytearray(b"\0")
    entries = [(5, 0x400400)]
    for tag, value in [(1, lib) for lib in libraries] + ([(29, runpath)] if runpath else []):
        entries.append((tag, len(strings)))
        strings.extend(value.encode() + b"\0")
    entries.append((0, 0))
    struct.pack_into("<IIQQQQQQ", data, 120, 2, 4, 512, 0x400200, 0,
                     len(entries) * 16, len(entries) * 16, 8)
    if interpreter:
        raw = interpreter.encode() + b"\0"
        data[256:256 + len(raw)] = raw
        struct.pack_into("<IIQQQQQQ", data, 176, 3, 4, 256, 0x400100, 0, len(raw), len(raw), 1)
    for i, entry in enumerate(entries):
        struct.pack_into("<qQ", data, 512 + i * 16, *entry)
    data[1024:1024 + len(strings)] = strings
    return data


class LinuxImportsTests(unittest.TestCase):
    def check_binary(self, interpreter, libraries, *, runpath=None, **options):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "program"
            path.write_bytes(elf(interpreter, libraries, runpath))
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                return check_imports.check(path, **options)

    def test_the_standard_loader_and_libraries_only(self):
        self.assertTrue(self.check_binary("/lib64/ld-linux-x86-64.so.2", ["libc.so.6", "libasound.so.2"]))
        self.assertFalse(self.check_binary("/lib/ld-musl-x86_64.so.1", ["libc.musl-x86_64.so.1"]))
        self.assertFalse(self.check_binary("/lib64/ld-linux-x86-64.so.2", ["libcustom.so"]))

    def test_a_server_needs_neither_sound_nor_controllers_and_no_build_path_runs(self):
        loader = "/lib64/ld-linux-x86-64.so.2"
        self.assertTrue(self.check_binary(loader, ["libgcc_s.so.1"], server=True))
        for lib in ("libasound.so.2", "libudev.so.1"):
            with self.subTest(lib=lib):
                self.assertFalse(self.check_binary(loader, [lib], server=True))
        self.assertFalse(self.check_binary(loader, [], runpath="/nix/store/libs"))
