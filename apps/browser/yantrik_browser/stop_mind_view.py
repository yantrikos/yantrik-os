#!/usr/bin/python3
"""Stop Mind View's browsers, and only those: the update's step on both sides of a build change.

A build starts Mind View's browser on its own port (this one's: browsers.conf). A browser left
from the other side of an update or a rollback would hold Mind View's profile, and a second launch
hands over to it instead of starting one on the right port: the mind would find no browser where
it looks. So before the shell restarts, the main process of every browser whose profile is Mind
View's is told to stop. It is the mind's, and it reopens it. The person's Browser is never matched:
a browser is Mind View's only by its whole profile path (listener.has_profile), the one each
process's own account and environment would give it, and a browser on its default profile, or with
the person's profile spelled out, is not.

    python3 stop_mind_view.py            # run as root by yantrik-update; prints what it stopped
    python3 stop_mind_view.py --list     # print what it would stop, stop nothing
"""

import os
import signal
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from yantrik_browser import listener, ports  # noqa: E402


def environ_of(pid, proc):
    """A process's environment as `name -> value`: what decides where its profile lives."""
    try:
        with open(os.path.join(proc, str(pid), "environ"), "rb") as f:
            pairs = f.read().split(b"\0")
    except OSError:
        return {}
    out = {}
    for pair in pairs:
        name, _, value = pair.decode("utf-8", "replace").partition("=")
        if name in ("HOME", "XDG_DATA_HOME"):
            out[name] = value
    return out


def mind_view_mains(proc="/proc"):
    """[(pid, cmdline)] of the main processes of the browsers on Mind View's profile."""
    found = []
    for name in os.listdir(proc):
        if not name.isdigit():
            continue
        try:
            with open(os.path.join(proc, name, "cmdline"), "rb") as f:
                raw = f.read()
        except OSError:
            continue
        if not listener.is_main(raw):
            continue
        profile = ports.mind_view_profile(environ_of(name, proc))
        if listener.has_profile(raw, profile):
            found.append((int(name), raw))
    return found


def main(argv):
    mains = mind_view_mains()
    for pid, _ in mains:
        print("%s %d" % ("would stop" if "--list" in argv else "stopping", pid))
        if "--list" not in argv:
            try:
                os.kill(pid, signal.SIGTERM)
            except OSError:
                pass
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
