"""Where Hermes runs, for the two things that need it: hermes_runtime.py prints the Python Hermes runs on

The install (install/hermes.sh) and "Can it remember?" (proofs.py) both run code in Hermes's own
Python, and both ask here, every time, rather than writing the answer down: a newer Hermes moves it.

- An older Hermes (0.14, VM 520) links ~/.local/bin/hermes into its venv, so its Python sits beside
  the real file.
- A newer one (0.21, a fresh install on VM 541, 13 Oct) puts a shell launcher there and runs a
  managed Python with a venv on its sys.path. Its own package manager rebuilds that venv as a new
  generation from its lock on an update, so the path changes. The generation it committed is read
  from its own record (`installs/<id>/facts.json`), which is what Hermes's bootstrap reads too.
  Asking Hermes instead (`hermes --run-module site`) goes through that bootstrap, which first runs
  any sync Hermes owes (90 s on VM 541), so it is only the last resort.

The memory provider is not installed into that venv for the same reason: a rebuild keeps only what
Hermes's lock and its plugins' declared pip_dependencies name, and dropped the provider on VM 541. It
lives in VENDOR, beside the shim Hermes loads, which Hermes never rebuilds; the shim and the proof put
it at the end of sys.path, so Hermes's own packages always come first.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Callable, Optional

# Hermes's user plugin for the YantrikDB provider, and the provider's packages beside it.
PLUGIN = Path.home() / ".hermes" / "plugins" / "yantrikdb"
VENDOR = PLUGIN / "vendor"

_SITE_LINE = re.compile(r"^\s*'(/[^']*)/lib/python[0-9.]*/site-packages',$")


def venv_of(site_output: str) -> Optional[str]:
    """The venv's Python from `hermes --run-module site`'s printout: the first sys.path entry
    whose venv has a pyvenv.cfg and an executable bin/python."""
    for line in site_output.splitlines():
        found = _SITE_LINE.match(line)
        if not found:
            continue
        venv = Path(found.group(1))
        python = venv / "bin" / "python"
        if (venv / "pyvenv.cfg").is_file() and os.access(python, os.X_OK):
            return str(python)
    return None


def committed_venv(hermes_home: Path) -> Optional[str]:
    """The venv Hermes's package manager last committed, from its own record
    (`installs/<id>/facts.json`, `packages.venv.environment`; hermes_bootstrap reads the same before
    every launch). Read, not asked: asking Hermes runs any sync it owes first, 90 s on VM 541."""
    records = sorted((hermes_home / "installs").glob("*/facts.json"),
                     key=lambda p: p.stat().st_mtime, reverse=True)
    for record in records:
        try:
            with open(record, encoding="utf-8") as f:
                venv = json.load(f)["packages"]["venv"]["environment"]
        except (OSError, ValueError, KeyError, TypeError):
            continue
        python = Path(str(venv)) / "bin" / "python"
        if os.access(python, os.X_OK):
            return str(python)
    return None


def python(run: Callable[..., subprocess.CompletedProcess] = subprocess.run) -> Optional[str]:
    """The Python Hermes runs on now, or None when there is no Hermes or it cannot be told."""
    launcher = Path.home() / ".local" / "bin" / "hermes"
    try:
        real = launcher.resolve(strict=True)
    except OSError:
        return None
    beside = real.parent / "python"
    if os.access(beside, os.X_OK):
        return str(beside)
    # ~/.hermes, as PLUGIN and hermes.sh have it: the desktop never sets HERMES_HOME.
    committed = committed_venv(Path.home() / ".hermes")
    if committed:
        return committed
    # A layout with neither: Hermes's own sys.path, which may first wait on a sync it owes.
    try:
        done = run([str(launcher), "--run-module", "site"], capture_output=True, text=True, timeout=180,
                   stdin=subprocess.DEVNULL)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return venv_of(done.stdout or "")


if __name__ == "__main__":
    found = python()
    if not found:
        print("cannot find the Python Hermes runs on: none beside hermes, and no venv on Hermes's own sys.path",
              file=sys.stderr)
        sys.exit(1)
    print(found)
