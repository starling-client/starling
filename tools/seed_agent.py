"""Randoms that repeat between vanilla and Sunrise (docs/parity-workflow.md, "Randoms that repeat").

`java_option(work, release)` builds the oracle JVM's agent (mods/sunrise/src/test/.../SeedAgent.java) into
work/seed-agent.jar and returns its -javaagent option; `ENV` is Sunrise's switch. A harness script's
`sync_randoms` step then sets the uniquifier, the entity-id counter and the level's random in both engines.
"""
import shutil
import subprocess
import zipfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

SOURCE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/SeedAgent.java'
ENV = {'SUNRISE_TEST_SEEDS': '1'}
MANIFEST = 'Manifest-Version: 1.0\r\nPremain-Class: dev.sunriselab.parity.SeedAgent\r\n\r\n'
# A fixed date: the same classes make the same jar, which tools/parity_shim.py hashes as an oracle input.
STAMP = (1980, 1, 1, 0, 0, 0)


def java_option(work, release):
    classes = Path(work) / 'seed-agent'
    shutil.rmtree(classes, ignore_errors=True)
    subprocess.run(['javac', '--release', str(release), '-proc:none', '-d', str(classes), str(SOURCE)], check=True)
    jar = Path(work) / 'seed-agent.jar'
    with zipfile.ZipFile(jar, 'w') as archive:
        archive.writestr(zipfile.ZipInfo('META-INF/MANIFEST.MF', STAMP), MANIFEST)
        for path in sorted(classes.rglob('*.class')):
            archive.writestr(zipfile.ZipInfo(path.relative_to(classes).as_posix(), STAMP), path.read_bytes())
    return f'-javaagent:{jar}'
