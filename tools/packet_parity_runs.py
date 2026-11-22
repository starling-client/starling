"""Where tools/client_packet_parity.py keeps a run: one folder per client and run number under the runs folder,
`<client>-<n>`, or `<label>-<client>-<n>` for a labelled set, so the two clients of one label never share one.
Standard library only, so the tests import it without the harness."""
import re
import shutil
from pathlib import Path

# A label becomes part of a folder's name: one plain component.
LABEL = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,47}")


def prefix(client, label=None):
    """The folder prefix of a client's runs: what `client_packet_diff.py --a/--b` takes."""
    if label is not None and not LABEL.fullmatch(label):
        raise ValueError(f"label {label!r}: letters, digits, '.', '_' and '-', starting with a letter or digit")
    return f"{label}-{client}" if label else client


def run_folder(runs, client, index, label=None, replace=False):
    """The run's own folder under `runs`, made empty; an existing one is replaced only with `replace`."""
    if not isinstance(index, int) or index < 1:
        raise ValueError(f"run number {index!r}: from 1")
    run = Path(runs) / f"{prefix(client, label)}-{index}"
    if run.exists():
        if not replace:
            raise FileExistsError(f"{run} exists: pass --replace to run it again, or another --label or --first")
        shutil.rmtree(run)
    run.mkdir(parents=True)
    return run
