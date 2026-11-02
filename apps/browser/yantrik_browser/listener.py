"""Which browser holds a loopback port, from facts a page cannot influence: the kernel's table of
listening sockets, the processes holding that socket, their account, and which profile each
process OWNS: the pid its `SingletonLock` names, or a `LOCK` file it holds open. The port is never
asked who it is.

Nothing here reads a command line. A real Chromium's /proc/<pid>/cmdline has come in several
shapes (NUL separated; one space-joined string; that with a trailing NUL, the one that got Mind
View's own browser refused on 520), a process can retitle itself, and a URL can carry
`--user-data-dir=`: text proves nothing either way. The files a process holds are what it cannot
fake (the 520 gate, on #1088, whose target.py this follows).

The Rust side (yantrik-browser-ports) does the same for the companion's tools, Arcade and the shell.
"""

import os
import time
from collections import namedtuple

from . import ports

PERSON, MIND_VIEW, IDENTITY = "person", "mind-view", "identity"


class Listener(namedtuple("Listener", "port uid inode pids place")):
    @property
    def key(self):
        """What makes it the same listener later: not the pids, which a fork changes for a moment."""
        return (self.port, self.uid, self.inode, self.place)


class Unidentified(Exception):
    """Which browser holds the port could not be established; the port is not used. `transient`:
    /proc was momentarily unreadable (a process exec'ing), worth one more look before refusing."""

    def __init__(self, reason, nobody=False, transient=False):
        super().__init__(reason)
        self.nobody, self.transient = nobody, transient


def lock_pid(profile):
    """The pid a profile's SingletonLock names (Chromium writes a link to `host-pid`), or None."""
    try:
        return int(os.readlink(os.path.join(profile, "SingletonLock")).rpartition("-")[2])
    except (OSError, ValueError):
        return None


def owns(proc, pid, profile):
    """Whether process `pid` owns `profile`: its SingletonLock names it, or it holds one of the
    profile's LOCK files open. Raises Unidentified when the files it holds cannot be read."""
    if lock_pid(profile) == pid:
        return True
    real = os.path.realpath(profile)
    fds = os.path.join(proc, str(pid), "fd")
    try:
        names = os.listdir(fds)
    except OSError as e:
        raise Unidentified("the files process %d holds could not be read (%s)" % (pid, e), transient=True) from None
    for name in names:
        try:
            held = os.readlink(os.path.join(fds, name))
        except OSError:
            continue
        if held.endswith(" (deleted)"):
            held = held[:-len(" (deleted)")]
        if os.path.basename(held) == "LOCK" and os.path.realpath(held).startswith(real + os.sep):
            return True
    return False


def mind_profiles(environ=None):
    """[(directory, place)] of the profiles that make a browser a mind's."""
    out = [(ports.mind_view_profile(environ), MIND_VIEW)]
    root = ports.identities_root(environ)
    try:
        out += [(os.path.join(root, n), IDENTITY) for n in sorted(os.listdir(root))
                if os.path.isdir(os.path.join(root, n))]
    except OSError:
        pass
    return out


def place_of_pid(proc, pid, environ=None):
    """MIND_VIEW, IDENTITY, or PERSON (it owns no mind's profile)."""
    for directory, place in mind_profiles(environ):
        if owns(proc, pid, directory):
            return place
    return PERSON


def is_mind_side(place):
    return place in (MIND_VIEW, IDENTITY)


def _rows(table, port):
    suffix = ":%04X" % port
    for line in table.splitlines()[1:]:
        f = line.split()
        if len(f) > 9 and f[3] == "0A" and f[1].endswith(suffix):
            try:
                yield f[1], int(f[7]), int(f[9])
            except ValueError:
                continue


def identify(port, proc="/proc", environ=None, tries=3, pause=0.1):
    """Who holds 127.0.0.1:`port`. Raises Unidentified, `nobody` set when nothing listens. A
    reading that failed only because /proc was momentarily unreadable is taken again, briefly."""
    for attempt in range(tries):
        try:
            return _identify(port, proc, environ)
        except Unidentified as e:
            if not e.transient or attempt == tries - 1:
                raise
            time.sleep(pause)


def _identify(port, proc, environ):
    try:
        with open(os.path.join(proc, "net", "tcp"), encoding="ascii", errors="replace") as f:
            table = f.read()
    except OSError as e:
        raise Unidentified("the socket table is unreadable (%s)" % e) from None
    rows = list(_rows(table, port))
    # The kernel hands a connection to the exact address before the wildcard one.
    chosen = ([r for r in rows if r[0].startswith("0100007F:")]
              or [r for r in rows if r[0].startswith("00000000:")])
    if not chosen:
        raise Unidentified("nothing is listening there", nobody=True)
    if len({r[2] for r in chosen}) != 1:
        raise Unidentified("two different sockets listen there")
    _, uid, inode = chosen[0]
    want = "socket:[%d]" % inode
    pids = []
    try:
        names = os.listdir(proc)
    except OSError as e:
        raise Unidentified("/proc is unreadable (%s)" % e) from None
    for name in names:
        if not name.isdigit():
            continue
        fd_dir = os.path.join(proc, name, "fd")
        try:
            held = any(os.readlink(os.path.join(fd_dir, fd)) == want for fd in os.listdir(fd_dir))
        except OSError:
            continue
        if held:
            pids.append(int(name))
    if not pids:
        raise Unidentified("the process holding it cannot be seen", transient=True)
    pids.sort()
    place = PERSON
    for pid in pids:
        here = place_of_pid(proc, pid, environ)
        if is_mind_side(here):
            place = here
    return Listener(port, uid, inode, tuple(pids), place)
