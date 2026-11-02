"""Which browser answers on the DevTools port, and whether this caller may use it (#1085).

The surface drives whatever answers on the port. The desktop's Browser, started first, is the
person's own Chromium (their profile, their sign-ins) and it holds that port; Mind View's browser
holds it when it started first. Neither the page nor the address says which one is there, so this
asks the machine: the process that owns the listening socket (`/proc/net/tcp`, then the `/proc`
entry holding that socket), its account, and the `--user-data-dir` it was started with. Nothing a
page can say or do changes any of that.

  * A Mind, or any caller that cannot be shown to be the person, may use only the browser whose
    profile is Mind View's (`mind_view_profile`). Every other browser, and any this cannot
    identify, is refused: nothing is read, shown or done.
  * The person may use any browser of their own account except Mind View's, which is not theirs.

The check is tied to the connection, not to the moment: a connection is opened to one listener
(`Listener`, identified before and after it is opened), and before every action the port is
identified again and must still be that listener, or the connection is dropped and made afresh,
and judged afresh.
"""

import collections
import os
import re
import urllib.parse

from yantrik_surface import agent_token, caller, mind_door

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
# is a different listener); uid: the socket's owner; profile: the real path of the
# `--user-data-dir` it was started with, or None when it was given none (its default profile).
Listener = collections.namedtuple("Listener", "port inode pids starts uid profile")


def mind_view_profile():
    """Where Mind View's browser keeps its profile. The one definition is `mind_browser_profile`
    and `singleton_isolation` in crates/yantrik-ui/src/wire/dock.rs, which starts that browser;
    this is the same path, and tests/browser-core/test_target.py reads that file to keep the two
    equal."""
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "yantrik", "mind-view-browser", "chromium")


def same_dir(a, b):
    try:
        if os.path.samefile(a, b):
            return True
    except OSError:
        pass
    return os.path.realpath(a) == os.path.realpath(b)


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
    lands: sockets bound to that very address, else the wildcards (0.0.0.0, and :: which also
    answers IPv4)."""
    loopback = {"tcp": "0100007F", "tcp6": "00000000000000000000000001000000"}[kind]
    exact, wild = [], []
    for table in ("tcp", "tcp6") if kind == "tcp" else ("tcp6",):
        try:
            with open(os.path.join(proc, "net", table)) as f:
                rows = f.read().splitlines()[1:]
        except OSError as e:
            raise Unidentified("the machine's socket table could not be read (%s)" % e) from None
        for row in rows:
            cols = row.split()
            if len(cols) < 10 or cols[3] != LISTEN:
                continue
            addr, _, hexport = cols[1].rpartition(":")
            if int(hexport, 16) != port:
                continue
            if set(addr) == {"0"}:
                wild.append((int(cols[9]), int(cols[7])))
            elif table == kind and addr == loopback:
                exact.append((int(cols[9]), int(cols[7])))
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


def _profile_of(proc, pid):
    """The real path of the `--user-data-dir` process `pid` was started with, None if it has none.
    Chromium reads `--switch=value` and `-switch=value`, stops at a bare `--`, and takes a relative
    value from the directory it started in. A flag given more than once with different values, or
    without one, is not something this will guess at."""
    try:
        with open(os.path.join(proc, str(pid), "cmdline"), "rb") as f:
            args = f.read().split(b"\0")
        values = set()
        for raw in args[1:]:
            arg = raw.decode("utf-8")
            if arg == "--":
                break
            m = re.match(r"^--?user-data-dir(=(.*))?$", arg, re.S)
            if m:
                if m.group(2) in (None, ""):
                    raise Unidentified("the browser was started with --user-data-dir but no path")
                values.add(m.group(2))
        if not values:
            return None
        if len(values) > 1:
            raise Unidentified("the browser was started with --user-data-dir more than once, differently")
        path = values.pop()
        if not os.path.isabs(path):
            path = os.path.join(os.readlink(os.path.join(proc, str(pid), "cwd")), path)
        return os.path.realpath(path)
    except (OSError, UnicodeDecodeError, IndexError) as e:
        raise Unidentified("the browser's start-up flags could not be read (%s)" % e) from None


def identify(address, proc=PROC):
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
        return Listener(port, inode, (), (), uid, None)     # another account: not ours to look into
    holders = _holders(proc, inode)
    if not holders:
        raise Unidentified("no process holding the listening socket could be found")
    profiles = {_profile_of(proc, pid) for pid, _ in holders}
    if len(profiles) > 1:
        raise Unidentified("the processes sharing the listening socket disagree about their profile")
    return Listener(port, inode, tuple(p for p, _ in holders), tuple(s for _, s in holders), uid,
                    profiles.pop())


# ── the rule ──

def caller_is_person():
    """True only for a call the kernel says came from the person's own account with no agent
    token. A caller that cannot be attributed (no peer, a token, the mind account) is not."""
    peer = caller()
    return (peer is not None and peer.uid == os.getuid() and not agent_token()
            and not mind_door.is_mind(peer.uid))


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


def decide(listener, person, mind_profile):
    """None when `person` (a bool) may use the browser `listener`, else the sentence refusing it."""
    if listener.uid != os.getuid():
        return ANOTHER_ACCOUNT
    ours = listener.profile is not None and same_dir(listener.profile, mind_profile)
    if person:
        return IS_MIND_VIEW if ours else None
    return None if ours else NOT_MIND_VIEW


class Guard:
    """Holds every use of a `cdp.Browser` to `decide`, for whoever is calling right now."""

    def __init__(self, is_person=caller_is_person, profile=mind_view_profile):
        self.is_person = is_person
        self.profile = profile

    def admit(self, browser):
        """Before one action: raise `Declined` unless this caller may use the browser the port holds
        now. A connection to a listener that is no longer the one on the port is dropped. The next
        connection is judged before it is opened (`browser.vet`), and again once it is."""
        person = self.is_person()
        mind_profile = self.profile()

        def vet(listener):
            sentence = decide(listener, person, mind_profile)
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
