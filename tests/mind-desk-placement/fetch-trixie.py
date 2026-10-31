#!/usr/bin/env python3
"""Unpack Debian 13 (trixie) labwc, grim and wlr-randr, with everything they need, into a prefix.

The Mind desk's bar is placed by a labwc window rule (config/labwc-mind/rc.xml), and what a rule
does depends on the labwc that reads it. The OS runs Debian 13's labwc 0.8.x, so the placement
check (run.sh) runs that one, not whatever a build machine happens to have. A build machine without
root cannot install it, and its libc is not Debian 13's, so this unpacks the packages (no install)
and run.sh starts labwc through the unpacked loader with the unpacked libraries.

    fetch-trixie.py PREFIX                 unpack exactly what trixie.lock names
    fetch-trixie.py --update-lock          rewrite trixie.lock from the mirror, for a reviewer to read

What is run is what a reviewer read. `trixie.lock`, committed beside this script, names every
package by its file and sha256; the default mode downloads exactly those files over https and
refuses any whose sha256 differs, so a mirror, a proxy or a network path cannot change what runs
without changing a file in git. Nothing is resolved from an index when it unpacks.

`--update-lock` is the only step that trusts the mirror's index. It reads Packages.xz over https,
resolves `Depends` (first alternative, virtual packages through `Provides`), and writes the lock.
Its diff is the thing to review. A package the mirror has dropped (a point release replaces the
files) is a failed download, and the answer is to update the lock and read the diff.

Packages that only matter on real hardware (the GPU drivers) are not followed: the check runs
labwc on wlroots' headless backend with the pixman renderer. Nothing outside PREFIX is touched.
"""

import hashlib
import lzma
import os
import re
import subprocess
import sys
import urllib.request

MIRROR = "https://deb.debian.org/debian"
SUITE = "trixie"
LOCK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "trixie.lock")
WANTED = ["labwc", "grim", "wlr-randr", "xkb-data", "fonts-dejavu-core"]
# Not followed: real hardware's drivers and what only they need.
SKIP = re.compile(r"^(libgl1-mesa-dri|libegl-mesa0|mesa-.*|libllvm.*|libgallium.*|libglx-mesa0|xserver-.*|libglapi-mesa)$")


def fetch(url):
    if not url.startswith("https://"):
        sys.exit(f"refusing a download that is not https: {url}")
    return urllib.request.urlopen(url, timeout=120).read()


def index():
    raw = lzma.decompress(fetch(f"{MIRROR}/dists/{SUITE}/main/binary-amd64/Packages.xz")).decode("utf-8", "replace")
    packages = {}
    for block in raw.split("\n\n"):
        fields, key = {}, None
        for line in block.split("\n"):
            if line.startswith((" ", "\t")) and key:
                fields[key] += "\n" + line
            elif ":" in line:
                key, _, value = line.partition(":")
                fields[key] = value.strip()
        if "Package" in fields:
            packages[fields["Package"]] = fields
    provides = {}
    for name, f in packages.items():
        for p in f.get("Provides", "").split(","):
            p = p.strip().split(" ")[0]
            if p:
                provides.setdefault(p, name)
    return packages, provides


def closure(packages, provides, roots):
    seen, todo = {}, list(roots)
    while todo:
        name = todo.pop()
        if name in seen or SKIP.match(name):
            continue
        if name not in packages:
            name = provides.get(name, name)
            if name not in packages or name in seen or SKIP.match(name):
                continue
        seen[name] = packages[name]
        for dep in packages[name].get("Depends", "").split(","):
            first = dep.strip().split("|")[0].strip()
            first = re.sub(r"\s*\(.*?\)", "", first).split(":")[0].strip()
            if first:
                todo.append(first)
    return seen


def update_lock():
    packages, provides = index()
    chosen = closure(packages, provides, WANTED)
    with open(LOCK, "w", encoding="utf-8", newline="\n") as out:
        out.write(f"# {SUITE} packages for tests/mind-desk-placement, by file and sha256. Written by\n")
        out.write("# `fetch-trixie.py --update-lock`; its diff is what a reviewer reads. name filename sha256\n")
        for name, f in sorted(chosen.items()):
            out.write(f"{name} {f['Filename']} {f['SHA256']}\n")
    print(f"{len(chosen)} packages written to {LOCK}; labwc {chosen['labwc']['Version']}", file=sys.stderr)


def unpack(prefix):
    entries = []
    for line in open(LOCK, encoding="utf-8"):
        if line.strip() and not line.startswith("#"):
            name, filename, sha256 = line.split()
            if not re.fullmatch(r"[0-9a-f]{64}", sha256) or ".." in filename or filename.startswith("/"):
                sys.exit(f"{LOCK}: a line that is not a package: {line!r}")
            entries.append((name, filename, sha256))
    os.makedirs(prefix + "/debs", exist_ok=True)
    for name, filename, sha256 in entries:
        deb = f"{prefix}/debs/{os.path.basename(filename)}"
        if not (os.path.exists(deb) and hashlib.sha256(open(deb, "rb").read()).hexdigest() == sha256):
            data = fetch(f"{MIRROR}/{filename}")
            if hashlib.sha256(data).hexdigest() != sha256:
                sys.exit(f"{name}: what the mirror sent is not what trixie.lock names (sha256 differs); not unpacked")
            open(deb, "wb").write(data)
        subprocess.run(["dpkg", "-x", deb, prefix + "/root"], check=True)
    print(f"{len(entries)} packages unpacked", file=sys.stderr)


def main():
    if sys.argv[1:] == ["--update-lock"]:
        update_lock()
    elif len(sys.argv) == 2 and not sys.argv[1].startswith("-"):
        unpack(os.path.abspath(sys.argv[1]))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
