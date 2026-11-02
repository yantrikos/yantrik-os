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


def candidate_profiles(proc, homes):
    """Where Mind View's profile may be, found without trusting any process's environment (a
    Chromium overwrites its own, and a browser started with a small one has no HOME in it):
    under each account's home, and wherever a process holds a LOCK file inside a directory laid
    out as Mind View's profile is."""
    tail = "/" + ports.MIND_VIEW_PROFILE + "/"
    found = {os.path.join(home, ".local", "share", *ports.MIND_VIEW_PROFILE.split("/")) for home in homes}
    for name in os.listdir(proc):
        fds = os.path.join(proc, name, "fd")
        try:
            held = [os.readlink(os.path.join(fds, fd)) for fd in os.listdir(fds)]
        except OSError:
            continue
        for path in held:
            at = path.find(tail)
            if at != -1:
                found.add(path[:at + len(tail) - 1])
    return sorted(found)


def all_homes():
    try:
        import pwd
        return [e.pw_dir for e in pwd.getpwall() if os.path.isabs(e.pw_dir)]
    except ImportError:
        return []


def mind_view_mains(proc="/proc", homes=None):
    """[pid] of the main processes of the browsers that own Mind View's profile: the pid each
    candidate profile's SingletonLock names, if it is running. Never by a command line or an
    environment, which a browser rewrites and which prove nothing."""
    pids = set()
    for directory in candidate_profiles(proc, all_homes() if homes is None else homes):
        pid = listener.lock_pid(directory)
        if pid and os.path.isdir(os.path.join(proc, str(pid))):
            pids.add(pid)
    return sorted(pids)


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
