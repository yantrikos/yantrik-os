"""Fail the image build unless every package in the Hermes tree it is about to ship declares a
licence this image may carry.

    python3 -I hermes_licences.py HERMES_HOME

Hermes is MIT, and so are nearly all of its packages, but its installer pulls in a few hundred
more (Python wheels in the venv, npm packages for the terminal and web UI, the tools under
`tools/`) and a pin moved by hand can bring in a new one. The rule is an allow-list, and it fails
closed: a package passes only when what it declares is wholly on the list (an `OR` needs one side
on it, an `AND` needs both), or is a reviewed exception below. One that declares nothing, or
something the list does not know, stops the build and is named, so a person reads it and either
adds it here with its upstream licence or takes it out of the tree. A tool in `tools/` or in
`facts.json` that is not one of the five the image ships stops the build the same way: ffmpeg is
taken out by name, and a pin that brings another is not shipped unseen.

Allowed here means permissive: MIT and its variants, BSD, Apache-2.0, ISC, PSF, Unlicense, 0BSD,
Blue Oak, CC0 and CC-BY (not NC, not SA). MPL-2.0 is file-level copyleft that asks nothing of the
rest of the image, and is allowed. Everything else is not, GPL and its family, source-available and
non-commercial terms, "no charge" licences and proprietary ones included.
"""

import glob
import json
import os
import re
import sys

# SPDX identifiers and the free-form spellings of the same licences that wheels in the tree use.
# Compared without case, with the words "license" and "the" dropped (`_term`).
ALLOWED = frozenset(
    name.lower()
    for name in (
        "MIT", "MIT-0", "MIT-CMU", "X11",
        "BSD", "BSD-2-Clause", "BSD-3-Clause", "0BSD", "3-Clause BSD", "BSD 3-Clause", "BSD 2-Clause",
        "Apache-2.0", "Apache 2.0", "Apache 2", "Apache Version 2.0", "Apache Software",
        "ISC", "ISCL", "ISC (ISCL)",
        "PSF", "PSFL", "PSF-2.0", "Python-2.0", "Python Software Foundation",
        "MPL-2.0", "MPL 2.0", "Mozilla Public 2.0", "Mozilla Public 2.0 (MPL 2.0)",
        "Unlicense", "BlueOak-1.0.0", "CC0-1.0", "CC-BY-4.0", "Zlib", "WTFPL", "Artistic-2.0",
    )
)

# Packages that declare no licence in their metadata, with the licence their upstream states.
# Reviewed by hand when added; a name here passes only while it still declares nothing.
EXCEPTIONS = {
    "agent-client-protocol": "Apache-2.0",   # github.com/zed-industries/agent-client-protocol
    "resvg-py": "MIT",                       # github.com/baseplate-admin/resvg-py
    "socksio": "MIT",                        # github.com/sethmlarson/socksio
    "ptyprocess": "ISC",                     # github.com/pexpect/ptyprocess
    # Hermes's own workspace packages: part of hermes-agent, MIT (LICENSE at the pinned commit).
    "web": "MIT",
    "hermes-tui": "MIT",
    "@hermes/ink": "MIT",
    "@hermes/shared": "MIT",
}

# The tools the bake leaves under `tools/`: the installer's own runtime, each permissive.
TOOLS = frozenset(("uv", "python", "node", "npm", "ripgrep"))

# Words that mean the build stops whatever else a package says beside them.
RESTRICTIVE = re.compile(
    r"GPL|SSPL|BUSL|Business Source|Commons Clause|Elastic License|no charge|non-?commercial|"
    r"proprietary|UNLICENSED|SEE LICENSE|CC-BY-NC|CC-BY-SA|EPL|Eclipse Public",
    re.IGNORECASE,
)

# What a wheel's `License:` field carries when the classifier is the real declaration.
PLACEHOLDERS = frozenset(("unknown", "dual", ""))

CLASSIFIER = "License :: OSI Approved :: "


def _term(text):
    """A licence name as the allow-list spells it: lower case, without commas, "license" and "the"."""
    words = re.sub(r"\b(the|license|licence)\b", " ", text.replace(",", " "), flags=re.IGNORECASE)
    return re.sub(r"\s+", " ", words).strip().lower()


def _allowed(expression):
    """Whether a licence expression is wholly on the list: `OR` needs one side, `AND` both."""
    text = expression.strip()
    if text.startswith("(") and text.endswith(")"):
        text = text[1:-1]
    if " OR " in text:
        return any(_allowed(side) for side in text.split(" OR "))
    if " AND " in text:
        return all(_allowed(side) for side in text.split(" AND "))
    return _term(text) in ALLOWED


def python_declarations(home):
    """(package, declared) for every wheel installed in a Hermes venv generation: `declared` holds
    the `License-Expression:`, `License:` and `Classifier: License` values, in that order."""
    for metadata in glob.glob(os.path.join(home, "installs", "*", "**", "*.dist-info", "METADATA"), recursive=True):
        declared = []
        name = os.path.basename(os.path.dirname(metadata))
        with open(metadata, encoding="utf-8", errors="replace") as lines:
            for line in lines:
                if not line.strip():
                    break
                if line.startswith("Name:"):
                    name = line.split(":", 1)[1].strip()
                elif line.startswith(("License-Expression:", "License:", "Classifier: License")):
                    declared.append(line.split(":", 1)[1].strip())
        yield name, declared


def npm_declarations(home):
    """(package, declared) for every npm package under Hermes's checkout."""
    pattern = os.path.join(home, "hermes-agent", "**", "node_modules", "**", "package.json")
    for manifest in glob.glob(pattern, recursive=True):
        try:
            with open(manifest, encoding="utf-8") as handle:
                package = json.load(handle)
        except (OSError, ValueError):
            continue
        if not isinstance(package, dict) or not package.get("name"):
            continue
        licence = package.get("license") or package.get("licenses") or ""
        if isinstance(licence, dict):
            licence = licence.get("type", "")
        if isinstance(licence, list):
            licence = " AND ".join(str(l.get("type", "") if isinstance(l, dict) else l) for l in licence)
        yield package["name"], [str(licence)] if str(licence) else []


def verdict(name, declared):
    """`None` when the package may ship, else why not."""
    for text in declared:
        if RESTRICTIVE.search(text):
            return "restrictive licence (%s)" % text
    terms = []
    for text in declared:
        if text.startswith(CLASSIFIER):
            terms.append(text[len(CLASSIFIER):])
        elif _term(text) not in PLACEHOLDERS:
            terms.append(text)
    if not terms:
        if name.lower().replace("_", "-") in EXCEPTIONS:
            return None
        return "declares no licence"
    unknown = [t for t in terms if not _allowed(t)]
    if unknown:
        return "licence not on the allow-list (%s)" % " / ".join(unknown)
    return None


def tool_problems(home):
    """Every folder under `tools/` (its files are PM's own record and lock), and every package
    `facts.json` records, that is not one of the tools the image ships."""
    tools = os.path.join(home, "tools")
    problems = []
    try:
        entries = sorted(os.listdir(tools))
    except OSError:
        return problems
    for entry in entries:
        if os.path.isdir(os.path.join(tools, entry)) and entry.split("-", 1)[0] not in TOOLS:
            problems.append("tools/%s: not a tool the image ships" % entry)
    try:
        with open(os.path.join(tools, "facts.json"), encoding="utf-8") as handle:
            recorded = json.load(handle).get("packages", {})
    except (OSError, ValueError, AttributeError):
        recorded = {}
    for name in sorted(recorded):
        if name not in TOOLS:
            problems.append("facts.json records %s: not a tool the image ships" % name)
    return problems


def problems(home):
    """Every reason this tree may not ship: `package: why` lines, sorted and unique."""
    found = set()
    for source in (python_declarations, npm_declarations):
        for name, declared in source(home):
            why = verdict(name, declared)
            if why:
                found.add("%s: %s" % (name, why))
    found.update(tool_problems(home))
    return sorted(found)


def main(argv):
    if len(argv) != 2:
        print("usage: hermes_licences.py HERMES_HOME", file=sys.stderr)
        return 2
    found = problems(argv[1])
    for line in found:
        print("not shipped: %s" % line, file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
