import struct
import unittest

from tools import check_macho

ARM64 = 0x0100000C
LC_LOAD_DYLIB = 0xC


def dylib(name: str) -> bytes:
    text = name.encode() + b"\0"
    text += bytes(-(24 + len(text)) % 8)
    return struct.pack("<IIIIII", LC_LOAD_DYLIB, 24 + len(text), 24, 0, 0, 0) + text


def build_version(minos: int, sdk: int) -> bytes:
    return struct.pack("<IIIIIII", check_macho.LC_BUILD_VERSION, 24, 1, minos, sdk, 0, 0)


def macho(commands: list[bytes], cpu: int = ARM64) -> bytes:
    body = b"".join(commands)
    return struct.pack("<IiiIIIII", check_macho.MH_MAGIC_64, cpu, 0, 2, len(commands), len(body), 0, 0) + body


class CheckMacho(unittest.TestCase):
    def test_versions_and_libraries_are_read(self):
        found = check_macho.inspect(macho([dylib("/usr/lib/libSystem.B.dylib"), dylib("/opt/other.dylib"), build_version(0xB0000, 0xF0500)]))
        self.assertEqual(found["cpu"], ARM64)
        self.assertEqual(found["libraries"], ["/usr/lib/libSystem.B.dylib", "/opt/other.dylib"])
        self.assertEqual([check_macho.version(v) for v in found["build_version"]], ["11.0", "15.5"])


if __name__ == "__main__":
    unittest.main()
