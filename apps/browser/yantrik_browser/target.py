"""Which browser answers on the DevTools port, and whether this caller may use it (#1085).

The surface drives whatever answers on the port. The desktop's Browser, started first, is the
person's own Chromium (their profile, their sign-ins) and it holds that port; Mind View's browser
holds it when it started first. Neither the page nor the address says which one is there, so this
asks the machine: the process that owns the listening socket (`/proc/net/tcp`, then the `/proc`
entry holding that socket), its account, and whether it is the process that owns Mind View's
profile.

That last fact is NOT read from its command line. Chromium rewrites its own `/proc/<pid>/cmdline`
into one space-joined string, any argument (a URL) can contain `--user-data-dir=...`, and a
process can retitle itself: text proves nothing. What it cannot fake is the files it holds: Mind
View's browser is the one whose pid is named by the profile's `SingletonLock` (written by
Chromium itself when it takes the profile), or that holds one of the profile's `LOCK` files open
(an exclusive lock on a database in it). A browser that shows neither is not Mind View's, whatever
its arguments say.

  * A Mind, or any caller that cannot be shown to be the person, may use only Mind View's browser.
    Every other browser, and any this cannot identify, is refused: nothing is read, shown or done.
  * The person may use any browser of their own account except Mind View's, which is not theirs.

The check is tied to the connection, not to the moment: a connection is opened to one listener
(`Listener`, identified before and after it is opened), and before every action the port is
identified again and must still be that listener, or the connection is dropped and made afresh,
and judged afresh.
"""

import collections
import os
import time
import urllib.parse

from yantrik_surface import agent_token, caller, mind_door

from . import who

PROC = "/proc"
LISTEN = "0A"
# A Mind reads a refusal that says "Nothing was done" as a call that did not happen. The remedy in
# every sentence below is the person's to take, never a route for the Mind.
NOTHING = "Nothing was done"


class Declined(Exception):
    """This caller may not use the browser on the port. `str()` is the sentence it reads."""


class Unidentified(Exception):
    """The browser on the port could not be established from the machine's own facts."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


class NoListener(Unidentified):
    """Nothing listens on the port: no browser is open."""


# pids/starts: every process holding the listening socket and when each started (so a reused pid
# is a different listener); uid: the socket's owner; mind_view: whether it owns Mind View's profile.
Listener = collections.namedtuple("Listener", "port inode pids starts uid mind_view")


def mind_view_profile():
    """Where Mind View's browser keeps its profile. The one definition is `mind_browser_profile`
    and `singleton_isolation` in crates/yantrik-ui/src/wire/dock.rs, which starts that browser;
    this is the same path, and tests/browser-core/test_target.py reads that file to keep the two
    equal."""
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "yantrik", "mind-view-browser", "chromium")


# ── finding the listener ──

def _endpoint(address):
    parts = urllib.parse.urlsplit(address)
    try:
        port = parts.port
    except ValueError:
        port = None
    if parts.scheme != "http" or port is None or parts.hostname not in ("127.0.0.1", "::1"):
        raise Unidentified("the browser address %r is not a loopback address and port" % address)
    return ("tcp6" if parts.hostname == "::1" else "tcp"), port


def _listening(proc, kind, port):
    """(inode, uid) of every socket listening where a connection to this loopback address and port
    lands: sockets bound to that very address (for IPv4 also `::ffff:127.0.0.1`), else the
    wildcards (0.0.0.0, and :: which answers IPv4 unless it is IPv6-only, which a table row does
    not say: then the connection fails and nothing is reached)."""
    loopback = {"tcp": "0100007F", "tcp6": "00000000000000000000000001000000"}[kind]
    mapped = "0000000000000000FFFF0000" + loopback if kind == "tcp" else None
    exact, wild = [], []
    for table in ("tcp", "tcp6") if kind == "tcp" else ("tcp6",):
        try:
            with open(os.path.join(proc, "net", table)) as f:
                rows = f.read().splitlines()[1:]
        except FileNotFoundError:
            if table == "tcp6" and kind == "tcp":
                continue                  # a kernel with no IPv6 has no IPv6 listeners
            raise Unidentified("the machine's socket table %s is missing" % table) from None
        except OSError as e:
            raise Unidentified("the machine's socket table could not be read (%s)" % e) from None
        for row in rows:
            cols = row.split()
            if len(cols) < 10 or cols[3] != LISTEN:
                continue
            addr, _, hexport = cols[1].rpartition(":")
            if int(hexport, 16) != port:
                continue
            found = (int(cols[9]), int(cols[7]))
            if set(addr) == {"0"}:
                wild.append(found)
            elif (table == kind and addr == loopback) or (table == "tcp6" and addr == mapped):
                exact.append(found)
    return exact or wild


def _holders(proc, inode):
    """The processes holding socket `inode`, as (pid, start time)."""
    want = "socket:[%d]" % inode
    found = []
    try:
        names = os.listdir(proc)
    except OSError as e:
        raise Unidentified("the process table could not be read (%s)" % e) from None
    for name in names:
        if not name.isdigit():
            continue
        try:
            for fd in os.listdir(os.path.join(proc, name, "fd")):
                if os.readlink(os.path.join(proc, name, "fd", fd)) == want:
                    found.append((int(name), _start_time(proc, name)))
                    break
        except OSError:
            continue
    return found


def _start_time(proc, pid):
    with open(os.path.join(proc, str(pid), "stat")) as f:
        return f.read().rpartition(")")[2].split()[19]


def _owns(proc, pid, profile):
    """Whether process `pid` owns the profile directory `profile` (its real path): the profile's
    `SingletonLock` names it, or it holds a `LOCK` file under the profile open."""
    try:
        if os.readlink(os.path.join(profile, "SingletonLock")).rpartition("-")[2] == str(pid):
            return True
    except OSError:
        pass
    fds = os.path.join(proc, str(pid), "fd")
    try:
        names = os.listdir(fds)
    except OSError as e:
        raise Unidentified("the files process %d holds could not be read (%s)" % (pid, e)) from None
    for name in names:
        try:
            held = os.readlink(os.path.join(fds, name))
        except OSError:
            continue
        if held.endswith(" (deleted)"):
            held = held[:-len(" (deleted)")]
        if os.path.basename(held) == "LOCK" and os.path.realpath(held).startswith(profile + os.sep):
            return True
    return False


def identify(address, proc=PROC, mind_profile=None):
    """The `Listener` a connection to `address` reaches. Raises `NoListener` when no browser is
    open there, and `Unidentified` when what is there cannot be established."""
    kind, port = _endpoint(address)
    rows = _listening(proc, kind, port)
    if not rows:
        raise NoListener("nothing is listening on port %d" % port)
    if len({inode for inode, _ in rows}) > 1:
        raise Unidentified("more than one socket is listening on port %d" % port)
    inode, uid = rows[0]
    if uid != os.getuid():
        return Listener(port, inode, (), (), uid, False)     # another account: not ours to look into
    holders = _holders(proc, inode)
    for _ in range(4):                # a browser still starting up moves its descriptors about
        if holders:
            break
        time.sleep(0.05)
        holders = _holders(proc, inode)
    if not holders:
        raise Unidentified("no process holding the listening socket could be found")
    profile = os.path.realpath(mind_profile or mind_view_profile())
    ours = any(_owns(proc, pid, profile) for pid, _ in holders)
    return Listener(port, inode, tuple(p for p, _ in holders), tuple(s for _, s in holders), uid, ours)


# ── the rule ──

def caller_is_person():
    """True only for a call the kernel says came from the person's own account with no agent
    token, whose ancestry proves no harness (`who.proves_person`: Hermes and the others run as the
    person, tokenless). A caller that cannot be attributed is not the person."""
    peer = caller()
    return (peer is not None and peer.uid == os.getuid() and not agent_token()
            and not mind_door.is_mind(peer.uid) and who.proves_person(peer.pid))


def cannot_tell(unidentified):
    return ("%s. Which browser is on the desktop's debugging port could not be established (%s), so "
            "it is not used: no page was opened, read or changed and no tab list is shown. Only "
            "the person can change this: they can close that browser, or do this part themselves. "
            "Trying again will not change it." % (NOTHING, unidentified.reason))


NOT_MIND_VIEW = (
    "%s. The browser on the desktop's debugging port is not Mind View's own browser, so it may be "
    "the person's own, with their sign-ins and tabs. A Mind does not use it: no page was opened, "
    "read or changed and no tab list is shown. Only the person can change this: they can close "
    "that browser, or do this part themselves. Trying again will not change it." % NOTHING)

ANOTHER_ACCOUNT = (
    "%s. The browser on the desktop's debugging port belongs to another account, not to the "
    "person this runs for, so it is not used. Closing it lets the Browser open." % NOTHING)

IS_MIND_VIEW = (
    "%s. The browser on the desktop's debugging port is Mind View's own browser, kept for a Mind's "
    "web tools, with none of your sign-ins; your Browser is not the one this reaches. Close "
    "Mind View's browser (or wait until the Mind is done with it), then open your Browser again."
    % NOTHING)


def decide(listener, person):
    """None when `person` (a bool) may use the browser `listener`, else the sentence refusing it."""
    if listener.uid != os.getuid():
        return ANOTHER_ACCOUNT
    if person:
        return IS_MIND_VIEW if listener.mind_view else None
    return None if listener.mind_view else NOT_MIND_VIEW


class Guard:
    """Holds every use of a `cdp.Browser` to `decide`, for whoever is calling right now."""

    def __init__(self, is_person=caller_is_person):
        self.is_person = is_person

    def admit(self, browser):
        """Before one action: raise `Declined` unless this caller may use the browser the port holds
        now. A connection to a listener that is no longer the one on the port is dropped. The next
        connection is judged before it is opened (`browser.vet`), and again once it is."""
        person = self.is_person()

        def vet(listener):
            sentence = decide(listener, person)
            if sentence:
                raise Declined(sentence)
        browser.vet = vet
        try:
            now = browser.who()
        except NoListener:
            browser.close()
            return
        if browser.alive and browser.bound != now:
            browser.close()
        vet(now)


class Open:
    """No guard: for a test that drives its own browser. Never the default."""

    def is_person(self):
        return True

    def admit(self, browser):
        browser.vet = None
