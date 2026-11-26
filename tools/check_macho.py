#!/usr/bin/env python3
"""Checks Apple-chip Mach-O programs without macOS tools (a cross-built leg runs on Linux): the architecture
(`lipo -archs`), the system-only libraries (`otool -L`), the embedded Info.plist (`otool -P`) and the build version
(`LC_BUILD_VERSION`: the oldest macOS the program runs on and the SDK it was linked against).

    check_macho.py [--minos 11.0] [--sdk 15.5] [--plist KEY] PROGRAM...
"""
import argparse
import struct
import sys

MH_MAGIC_64 = 0xFEEDFACF
CPU_TYPE_ARM64 = 0x0100000C
LC_SEGMENT_64 = 0x19
LC_BUILD_VERSION = 0x32
DYLIB_COMMANDS = {0xC, 0x80000018, 0x8000001F, 0x80000023, 0x20}
SYSTEM_PREFIXES = ("/usr/lib/", "/System/Library/")


def version(packed: int) -> str:
    return f"{packed >> 16}.{packed >> 8 & 0xFF}" + (f".{packed & 0xFF}" if packed & 0xFF else "")


def inspect(data: bytes) -> dict:
    magic, cpu, _, _, count, _, _, _ = struct.unpack_from("<IiiIIIII", data, 0)
    if magic != MH_MAGIC_64:
        sys.exit("not a thin 64-bit Mach-O program")
    found = {"cpu": cpu, "libraries": [], "build_version": None, "plist": b""}
    offset = 32
    for _ in range(count):
        command, size = struct.unpack_from("<II", data, offset)
        if command in DYLIB_COMMANDS:
            name_at = struct.unpack_from("<I", data, offset + 8)[0]
            found["libraries"].append(data[offset + name_at:offset + size].split(b"\0")[0].decode())
        elif command == LC_BUILD_VERSION:
            found["build_version"] = struct.unpack_from("<II", data, offset + 12)
        elif command == LC_SEGMENT_64:
            for index in range(struct.unpack_from("<I", data, offset + 64)[0]):
                section = offset + 72 + 80 * index
                if data[section:section + 16].split(b"\0")[0] == b"__info_plist":
                    size_at, file_at = struct.unpack_from("<QI", data, section + 40)
                    found["plist"] = data[file_at:file_at + size_at]
        offset += size
    return found


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--minos")
    parser.add_argument("--sdk")
    parser.add_argument("--plist", help="a key the embedded Info.plist of the first program must hold")
    parser.add_argument("programs", nargs="+")
    args = parser.parse_args()
    failed = False
    for number, path in enumerate(args.programs):
        with open(path, "rb") as file:
            found = inspect(file.read())
        problems = []
        if found["cpu"] != CPU_TYPE_ARM64:
            problems.append(f"cpu type {found['cpu']:#x}, expected arm64")
        problems += [f"links {name}" for name in found["libraries"] if not name.startswith(SYSTEM_PREFIXES)]
        minos, sdk = (version(value) for value in found["build_version"] or (0, 0))
        if args.minos and minos != args.minos:
            problems.append(f"minos {minos}, expected {args.minos}")
        if args.sdk and sdk != args.sdk:
            problems.append(f"sdk {sdk}, expected {args.sdk}")
        if args.plist and number == 0 and args.plist.encode() not in found["plist"]:
            problems.append(f"no {args.plist} in the embedded Info.plist")
        print(f"{path}: arm64, {len(found['libraries'])} libraries, minos {minos}, sdk {sdk}, Info.plist {len(found['plist'])} bytes")
        for problem in problems:
            print(f"::error::{path}: {problem}")
        failed |= bool(problems)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
