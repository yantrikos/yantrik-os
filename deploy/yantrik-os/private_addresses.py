#!/usr/bin/env python3
"""Which private network addresses an image ships, and whether any of them is a real one.

    private_addresses.py <config.yaml> <update.conf> <yantrik-update>

The boot test asks this of the booted machine: `grep -HnoE GREP_ERE <files>` runs in the guest,
where the files are, and only its few lines come back over the serial console to `problems()`.
The selftest beside this file runs the same function over the files in the tree.

What it holds:
  - config.yaml and update.conf name no private address at all: a shipped image is pointed at
    the public host, and nothing of the developer's network is configuration.
  - yantrik-update names private addresses for two honest reasons — the ranges it treats as the
    local network (`10.0.0.0/8`), and its selftest's made-up fixtures — so there a private
    address may stand only as a network in CIDR, or between the YANTRIK-SELFTEST-BEGIN and
    YANTRIK-SELFTEST-END markers. Both markers must be there, once each and in that order: a
    check that silently allows the whole file once a marker is lost is no check.
  - 192.168.4.x is the developer's real LAN (the release host, the person's SearXNG). It fails
    anywhere, in any of the three files, test data included.

Exit 0 when nothing is wrong, 1 with one line per problem otherwise.
"""
import ipaddress
import re
import sys

BEGIN, END = "YANTRIK-SELFTEST-BEGIN", "YANTRIK-SELFTEST-END"
# An address not run on from a longer number (`110.0.0.1` is no 10/8 address), with its prefix
# length if it is written as CIDR. ERE, so the guest's grep and this file read it alike.
_ADDR = r"(192\.168|10\.[0-9]+|172\.(1[6-9]|2[0-9]|3[01]))\.[0-9]+\.[0-9]+(/[0-9]+)?"
GREP_ERE = "%s|%s|(^|[^0-9.])%s" % (BEGIN, END, _ADDR)
_TOKEN = re.compile(GREP_ERE)
_LAN = re.compile(r"^192\.0*168\.0*4\.")
UPDATER = "yantrik-update"


def scan(text):
    """[(line number, match)] as `grep -noE GREP_ERE` prints them for this text."""
    found = []
    for n, line in enumerate(text.split("\n"), 1):
        found.extend((n, m.group(0)) for m in _TOKEN.finditer(line))
    return found


def parse_grep(output):
    """{path: [(line number, match)]} from `grep -HnoE GREP_ERE <files>`."""
    found = {}
    for line in output.splitlines():
        m = re.match(r"^(.*?):([0-9]+):(.*)$", line.strip("\r"))
        if m:
            found.setdefault(m.group(1), []).append((int(m.group(2)), m.group(3)))
    return found


def _address(match):
    m = re.search(_ADDR, match)
    return m.group(0) if m else None


def _is_network(addr):
    """A network written in CIDR, host bits clear: `10.0.0.0/8`, never `10.0.0.7/8`."""
    if "/" not in addr:
        return False
    try:
        ipaddress.ip_network(addr, strict=True)
        return True
    except ValueError:
        return False


def problems(found):
    """What is wrong in {path: [(line number, match)]}; an empty list when nothing is.

    A path is the updater when its basename is `yantrik-update`; every other path is
    configuration. Pass every file looked at, the updater with its matches even if there are
    none, so a missing marker is noticed.
    """
    out = []
    for path in sorted(found):
        matches = found[path]
        updater = path.rsplit("/", 1)[-1] == UPDATER
        begins = [n for n, m in matches if m == BEGIN]
        ends = [n for n, m in matches if m == END]
        inside = None
        if updater:
            if len(begins) != 1 or len(ends) != 1 or begins[0] >= ends[0]:
                out.append("%s: wants one %s line before one %s line, has %d and %d"
                           % (path, BEGIN, END, len(begins), len(ends)))
            else:
                inside = (begins[0], ends[0])
        for n, m in matches:
            addr = _address(m)
            if addr is None:
                continue
            where = "%s:%d: %s" % (path, n, addr)
            if _LAN.match(addr):
                out.append(where + " is on the developer's own LAN (192.168.4.0/24)")
            elif not updater:
                out.append(where + " is a private address in shipped configuration")
            elif _is_network(addr):
                continue
            elif inside is None or not inside[0] < n < inside[1]:
                out.append(where + " is a private host address outside the selftest")
    return out


def main(paths):
    found = {}
    for p in paths:
        try:
            with open(p, encoding="utf-8", errors="replace") as f:
                found[p] = scan(f.read())
        except FileNotFoundError:
            if p.rsplit("/", 1)[-1] == UPDATER:
                found[p] = []
    bad = problems(found)
    for line in bad:
        print(line)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
