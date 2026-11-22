#!/usr/bin/env python3
"""Check the machine and, optionally, the main-thread stack reserve of Windows PE executables."""

import argparse
import struct
import sys
from pathlib import Path

MACHINES = {"x64": 0x8664, "arm64": 0xAA64}


def pe_fields(data, *, read_stack_reserve=False):
    """Read the COFF machine and, when requested, PE32+ SizeOfStackReserve."""
    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError("not a PE executable (missing MZ header)")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if pe + 24 > len(data) or data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("not a PE executable (missing PE signature or incomplete COFF header)")
    machine = struct.unpack_from("<H", data, pe + 4)[0]
    if not read_stack_reserve:
        return machine, None
    optional = pe + 24
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    # PE32+ stores the 64-bit reserve at byte 72 of the optional header.
    if optional_size < 80 or optional + optional_size > len(data):
        raise ValueError("missing or truncated PE optional header")
    if struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise ValueError("expected a PE32+ optional header")
    return machine, struct.unpack_from("<Q", data, optional + 72)[0]


def check(path, architecture, stack_reserve=None):
    expected = MACHINES[architecture]
    try:
        actual, reserve = pe_fields(Path(path).read_bytes(), read_stack_reserve=stack_reserve is not None)
    except (OSError, ValueError) as error:
        print(f"{path}: {error}", file=sys.stderr)
        return False
    if actual != expected:
        print(f"{path}: PE machine 0x{actual:04x}, expected {architecture} (0x{expected:04x})", file=sys.stderr)
        return False
    if stack_reserve is not None and reserve != stack_reserve:
        print(f"{path}: PE SizeOfStackReserve is {reserve}, expected {stack_reserve} bytes", file=sys.stderr)
        return False
    stack = "" if reserve is None else f", {reserve} stack bytes"
    print(f"{path}: {architecture} (PE machine 0x{actual:04x}{stack})")
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--stack-reserve", type=int, metavar="BYTES")
    parser.add_argument("architecture", choices=MACHINES)
    parser.add_argument("binaries", nargs="+")
    args = parser.parse_args()
    sys.exit(0 if all(check(path, args.architecture, args.stack_reserve) for path in args.binaries) else 1)


if __name__ == "__main__":
    main()
