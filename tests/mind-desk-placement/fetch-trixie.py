#!/usr/bin/env python3
"""Unpack Debian 13 (trixie) labwc, grim and wlr-randr, with everything they need, into a prefix.

The Mind desk's two bars are placed by labwc window rules (config/labwc-mind/rc.xml), and what a
rule does depends on the labwc that reads it. The OS runs Debian 13's labwc 0.8.x, so the placement
check (run.sh) runs that one, not whatever a build machine happens to have. A build machine without
root cannot install it, and its libc is not Debian 13's, so this unpacks the packages (no install)
and run.sh starts labwc through the unpacked loader with the unpacked libraries.

    fetch-trixie.py PREFIX [package ...]

It resolves `Depends` (first alternative, virtual packages through `Provides`) from the Debian
mirror's package list, downloads each .deb, checks its sha256 against that list, and unpacks it
with `dpkg -x`. Nothing outside PREFIX is touched. Packages that only matter on real hardware (the
GPU drivers) are not followed: the check runs labwc on wlroots' headless backend with the pixman
renderer.
"""

import hashlib
import lzma
import os
import re
import subprocess
import sys
import urllib.request

MIRROR = "http://deb.debian.org/debian"
SUITE = "trixie"
WANTED = ["labwc", "grim", "wlr-randr", "xkb-data", "fonts-dejavu-core"]
# Not followed: real hardware's drivers and what only they need.
SKIP = re.compile(r"^(libgl1-mesa-dri|libegl-mesa0|mesa-.*|libllvm.*|libgallium.*|libglx-mesa0|xserver-.*|libglapi-mesa)$")


def index():
    url = f"{MIRROR}/dists/{SUITE}/main/binary-amd64/Packages.xz"
    raw = lzma.decompress(urllib.request.urlopen(url).read()).decode("utf-8", "replace")
    packages = {}
    for block in raw.split("\n\n"):
        fields = {}
        key = None
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
            dep = dep.strip()
            if not dep:
                continue
            first = dep.split("|")[0].strip()
            first = re.sub(r"\s*\(.*?\)", "", first).split(":")[0].strip()
            todo.append(first)
    return seen


def main():
    prefix = os.path.abspath(sys.argv[1])
    extra = sys.argv[2:]
    os.makedirs(prefix + "/debs", exist_ok=True)
    packages, provides = index()
    chosen = closure(packages, provides, WANTED + extra)
    print(f"{len(chosen)} packages", file=sys.stderr)
    for name, f in sorted(chosen.items()):
        deb = f"{prefix}/debs/{os.path.basename(f['Filename'])}"
        if not os.path.exists(deb):
            data = urllib.request.urlopen(f"{MIRROR}/{f['Filename']}").read()
            if hashlib.sha256(data).hexdigest() != f["SHA256"]:
                sys.exit(f"{name}: sha256 does not match the package list")
            open(deb, "wb").write(data)
        subprocess.run(["dpkg", "-x", deb, prefix + "/root"], check=True)
    print(f"labwc {chosen['labwc']['Version']}", file=sys.stderr)


if __name__ == "__main__":
    main()
