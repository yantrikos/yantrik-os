#!/usr/bin/python3
"""Stop Mind View's browsers, and only those: the update's step on both sides of a build change.

A build starts Mind View's browser on its own port (this one's: browsers.conf). A browser left
from the other side of an update or a rollback would hold Mind View's profile, and a second launch
hands over to it instead of starting one on the right port: the mind would find no browser where
it looks. So before the shell restarts, the main process of every browser that owns Mind View's
profile is told to stop. It is the mind's, and it reopens it. The person's Browser is never matched:
a process is Mind View's browser only when Mind View's profile's `SingletonLock` names it (the
profile as each process's own account and environment would give it), never by its arguments.

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
    """[pid] of the main processes of the browsers that own Mind View's profile."""
    found = []
    for name in os.listdir(proc):
        if name.isdigit() and listener.lock_pid(ports.mind_view_profile(environ_of(name, proc))) == int(name):
            found.append(int(name))
    return found


def main(argv):
    mains = mind_view_mains()
    for pid in mains:
        print("%s %d" % ("would stop" if "--list" in argv else "stopping", pid))
        if "--list" not in argv:
            try:
                os.kill(pid, signal.SIGTERM)
            except OSError:
                pass
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
