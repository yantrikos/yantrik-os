"""Fail the image build if the Hermes tree it is about to ship holds a restrictive licence.

    python3 -I hermes_licences.py HERMES_HOME

Hermes is MIT, and so are nearly all of its packages, but its installer pulls in a few hundred
more (Python wheels in the venv, npm packages for the terminal and web UI) and a pin moved by hand
can bring in a new one. The rule is to ship nothing with a restrictive licence, so what is in the
tree is read here, from the licence each package declares, and a match stops the build. A package
that declares none is not a match: the build prints it, so a person can read it.

Restrictive here means copyleft (GPL, AGPL, LGPL), source-available or non-commercial terms (SSPL,
BUSL, Commons Clause, Elastic, "no charge" or "non-commercial" licences) and proprietary ones.
MPL-2.0 is file-level copyleft that asks nothing of the rest of the image, and is allowed.
"""

import glob
import json
import os
import re
import sys

RESTRICTIVE = re.compile(
    r"GPL|SSPL|BUSL|Business Source|Commons Clause|Elastic License|no charge|non-?commercial|proprietary",
    re.IGNORECASE,
)


def python_licences(home):
    """(package, licence) for every wheel installed in a Hermes venv generation."""
    for metadata in glob.glob(os.path.join(home, "installs", "*", "**", "*.dist-info", "METADATA"), recursive=True):
        declared = []
        with open(metadata, encoding="utf-8", errors="replace") as lines:
            for line in lines:
                if not line.strip():
                    break
                if line.startswith(("License-Expression:", "License:", "Classifier: License")):
                    declared.append(line.split(":", 1)[1].strip())
        yield os.path.basename(os.path.dirname(metadata)), " / ".join(declared)


def npm_licences(home):
    """(package, licence) for every npm package under Hermes's checkout."""
    pattern = os.path.join(home, "hermes-agent", "**", "node_modules", "**", "package.json")
    for manifest in glob.glob(pattern, recursive=True):
        try:
            with open(manifest, encoding="utf-8") as handle:
                package = json.load(handle)
        except (OSError, ValueError):
            continue
        if isinstance(package, dict) and package.get("name"):
            yield package["name"], str(package.get("license") or package.get("licenses") or "")


def restrictive(home):
    """Every (package, licence) under `home` whose declared licence is restrictive."""
    found = set()
    for source in (python_licences, npm_licences):
        found.update((name, text) for name, text in source(home) if RESTRICTIVE.search(text))
    return sorted(found)


def main(argv):
    if len(argv) != 2:
        print("usage: hermes_licences.py HERMES_HOME", file=sys.stderr)
        return 2
    found = restrictive(argv[1])
    for name, text in found:
        print("restrictive licence: %s (%s)" % (name, text), file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
