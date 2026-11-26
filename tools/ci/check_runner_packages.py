#!/usr/bin/env python3
"""Fail when the Dyna CI runner image lacks an apt package listed in tools/ci/runner-packages.txt.

The image also serves the room's other repositories, so packages beyond the list are fine. It is built outside this
repository, so whoever changes it runs this against its Containerfile:
    python3 tools/ci/check_runner_packages.py path/to/Containerfile
"""
import re
import sys
from pathlib import Path

LIST = Path(__file__).with_name("runner-packages.txt")


def image_packages(containerfile):
    text = re.sub(r"\\\n", " ", containerfile)
    packages = []
    for command in re.split(r"&&|;|\n", text):
        words = command.split()
        if "apt-get" in words and "install" in words:
            packages += [word for word in words[words.index("install") + 1:] if not word.startswith("-")]
    return packages


def main(listing=LIST):
    image = image_packages(Path(sys.argv[1]).read_text())
    if not image:
        sys.exit("no apt-get install found in the Containerfile")
    listed = [line for line in listing.read_text().splitlines() if line and not line.startswith("#")]
    missing = [package for package in listed if package not in image]
    if missing:
        sys.exit(f"the image lacks runner-packages.txt's: {' '.join(missing)}")
    print(f"the image installs all {len(listed)} packages of runner-packages.txt")


if __name__ == "__main__":
    main()
