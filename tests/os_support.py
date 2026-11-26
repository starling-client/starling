"""What the operating system running a test allows, for tests whose behaviour depends on it.

A test checks a capability here, never a platform name: Windows lets a process create a symbolic
link only with Developer Mode on or as an administrator, and its bare `bash` may be WSL's, which
cannot see the host's paths. macOS and Linux always allow both.
"""
import functools
import os
import shutil
import tempfile
import unittest


@functools.cache
def can_symlink():
    """Whether this process may create a symbolic link."""
    with tempfile.TemporaryDirectory() as directory:
        try:
            os.symlink(directory, os.path.join(directory, "link"), target_is_directory=True)
        except OSError:
            return False
    return True


# The bash on PATH. Windows' CreateProcess looks in System32 before PATH, so a bare "bash" there is WSL's (#5383),
# which cannot see the host's paths; Git Bash and every macOS or Linux bash can.
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.dirname(os.path.normcase(BASH)).endswith(os.path.normcase(r"\system32")))

needs_runner_shell = unittest.skipUnless(POSIX_BASH, "needs a bash that sees this system's paths, not WSL's")
"""Skips a test that runs a workflow step or a `tools/ci/*.sh` script with bash where the only bash is WSL's."""


def needs_symlinks(test):
    """Skips `test` (a test method or class) where this process may not create a symbolic link."""
    return unittest.skipUnless(can_symlink(), "this system does not allow creating symbolic links")(test)

