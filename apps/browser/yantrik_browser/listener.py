"""Which browser holds a loopback port, from facts a page cannot influence: the kernel's table of
listening sockets, the processes holding that socket, their account, and the profile each was
started with. The port is never asked who it is.

The Rust side (yantrik-browser-ports, `listener` and `cmdline`) does the same for the companion's
tools, Arcade and the shell; this is the browser service's.

A Chromium main process rewrites its own /proc/<pid>/cmdline into ONE space-joined string, with
no NUL between the words (found on 520, #1088). Splitting on NUL and looking for an argument that
is exactly `--user-data-dir=...` finds nothing, and takes every Chromium for the one on the
default profile. A command line is read both ways here; a profile path with a space in it is not
read back whole from the joined form, so `has_profile` compares the whole expected path instead.
"""

import os
import time
from collections import namedtuple

from . import ports

PROFILE = "--user-data-dir="
DEBUGGING_PORT = "--remote-debugging-port="

DEFAULT, MIND_VIEW, IDENTITY, OTHER = "default", "mind-view", "identity", "other"

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


def words(raw):
    """(the words up to a bare `--`, whether the line was NUL separated)."""
    text = raw.decode("utf-8", "replace")
    nul = b"\0" in raw
    parts = [w for w in (text.split("\0") if nul else text.split(" ")) if w]
    out = []
    for w in parts:
        if w == "--":
            break
        out.append(w)
    return out, nul


def values(raw, flag):
    """Every value `flag` was given, in order; `-flag=` counts too."""
    single = flag[1:]
    out = []
    for w in words(raw)[0]:
        for spelled in (flag, single):
            if w.startswith(spelled):
                out.append(w[len(spelled):])
                break
    return out


def has_profile(raw, directory):
    """Whether the line names `directory` as its one profile: the whole path, however the line
    was written, and no other --user-data-dir beside it."""
    if len(values(raw, PROFILE)) != 1:
        return False
    want = PROFILE + directory
    ws, nul = words(raw)
    if nul:
        return any(w in (want, want[1:]) for w in ws)
    text = raw.decode("utf-8", "replace")
    at = text.find(want)
    while at != -1:
        end = at + len(want)
        if (at == 0 or text[at - 1] == " ") and (end == len(text) or text[end] == " "):
            return True
        at = text.find(want, at + 1)
    return False


def serves_port(raw, port):
    return str(port) in values(raw, DEBUGGING_PORT)


def is_main(raw):
    return not any(t.startswith("--type=") for t in raw.decode("utf-8", "replace").replace("\0", " ").split(" "))


def place_of(raw, environ=None):
    """Whose profile a command line names, or None when it names it twice or with nothing."""
    found = values(raw, PROFILE)
    if not found:
        return DEFAULT
    if len(found) != 1 or not found[0]:
        return None
    if has_profile(raw, ports.mind_view_profile(environ)):
        return MIND_VIEW
    if found[0].startswith(ports.identities_root(environ) + "/"):
        return IDENTITY
    return OTHER


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
    place = None
    for pid in pids:
        try:
            with open(os.path.join(proc, str(pid), "cmdline"), "rb") as f:
                raw = f.read()
        except OSError as e:
            raise Unidentified("process %d's command line is unreadable (%s)" % (pid, e), transient=True) from None
        here = place_of(raw, environ)
        if here is None:
            raise Unidentified("process %d names its profile more than once" % pid)
        if place is not None and place != here:
            raise Unidentified("the processes holding it disagree about whose it is")
        place = here
    return Listener(port, uid, inode, tuple(pids), place)
