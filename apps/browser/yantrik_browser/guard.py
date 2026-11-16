"""Which browser a call may use: decided by who is calling and by what holds its pipe (#1085, #1200).

Who is calling picks the browser, deterministically and never by who answers first:

  a mind (an agent token, the mind account, a caller the shell says descends from an attached
  mind, or anyone not shown to be the person)  ->  Mind View's browser and its profile
  the person (their own `yos web`, `yos act browser`)  ->  their Browser, over its pipe (#1137)

Then it is checked by what holds it: each side's pipe must be held for a live browser run by this
account, Mind View's on Mind View's profile and the person's on none of a mind's (listener.py).
Neither browser has a port (#1200). A call is never sent to a browser it was not meant for, and a
refusal says what is true.
"""

import os
import select
import socket
import time

from yantrik_surface import PeerRefused, agent_token, call_once, caller
from yantrik_surface import gate, mind_door, reach, wire

from . import listener, ports

SHELL_ASKED = 3.0
SHELL_LOOKED_AT = 0.5    # seconds to connect to the shell's socket and read who holds it
START_WAIT = 30.0        # seconds Mind View's browser is given to be handed over once asked for


class Denied(Exception):
    """The call may not use what holds the pipe. The sentence is the caller's to read."""


class Starting(Exception):
    """The shell could not start Mind View's browser; the message is why."""


def shell_open_mind_browser():
    """Ask the shell to start Mind View's browser (its own action: `open_app` would be routed as
    the person's, since the caller here is this service). Raises Starting with the shell's reason."""
    try:
        reply = call_once(wire.default_socket_path(gate.SHELL), "app.act",
                          {"action": "open_mind_browser", "args": {}},
                          timeout=START_WAIT, peer_rule=gate.must_be_the_shell)
        reach._act_result(reply)  # noqa: SLF001
    except reach.Unanswered as e:
        raise Starting(str(e)) from None
    except (PeerRefused, OSError, ConnectionError, ValueError) as e:
        raise Starting("the desktop did not answer (%s)" % e) from None


def _start_of(pid, proc="/proc"):
    """A process's start time in ticks since boot: with its pid, who it is for as long as it lives."""
    try:
        with open(os.path.join(proc, str(pid), "stat"), encoding="utf-8", errors="replace") as f:
            return f.read().rpartition(")")[2].split()[19]
    except (OSError, IndexError):
        return None


class Watch:
    """A caller, held by something that survives its pid being reused: a pidfd, or failing that its
    start time. `gone()` says it has exited (or was never there). Taken as early as the call is
    handled, so a call that waited behind another is still known to be its caller's."""

    def __init__(self, pid, proc="/proc"):
        self.pid, self.proc, self.fd = pid, proc, None
        self.start = _start_of(pid, proc) if pid else None
        try:
            self.fd = os.pidfd_open(pid) if pid else None
        except (OSError, AttributeError):
            self.fd = None

    def gone(self):
        if self.fd is not None and select.select([self.fd], [], [], 0)[0]:
            return True
        return self.start is None or _start_of(self.pid, self.proc) != self.start

    def __del__(self):
        if self.fd is not None:
            try:
                os.close(self.fd)
            except OSError:
                pass


def is_bridge_ancestor(pid, proc="/proc"):
    """Whether the process `pid` started below a `yos-mcp`, the program every mind's tools run
    through. The shell's own ancestry reading (which minds are attached) decides the rest."""
    seen = set()
    while isinstance(pid, int) and pid > 1 and pid not in seen:
        seen.add(pid)
        try:
            with open(os.path.join(proc, str(pid), "cmdline"), "rb") as f:
                words = f.read().replace(b"\0", b" ").decode("utf-8", "replace").split(" ")
            with open(os.path.join(proc, str(pid), "stat"), encoding="utf-8", errors="replace") as f:
                stat = f.read()
            parent = int(stat[stat.rindex(")") + 1:].split()[1])
        except (OSError, ValueError, IndexError):
            return False
        if any(os.path.basename(w) == "yos-mcp" for w in words[:3]):
            return True
        pid = parent
    return False


def shell_class(pid):
    """What the shell says the process `pid` is: "person" or "mind" (it knows which minds are
    attached, by the kernel's pids). Anything else, or no answer, is None."""
    try:
        reply = call_once(
            wire.default_socket_path(gate.SHELL), "app.act",
            {"action": "caller_class", "args": {"pid": pid}},
            timeout=SHELL_ASKED, peer_rule=gate.must_be_the_shell)
        answer = reach._act_result(reply)  # noqa: SLF001 - the one reader of an act's answer
    except (PeerRefused, reach.Unanswered, OSError, ConnectionError, ValueError):
        return None
    kind = answer.get("class") if isinstance(answer, dict) else None
    return kind if kind in ("person", "mind") else None


def is_the_shell(pid):
    """Whether `pid` is the process answering on the shell's socket, by the kernel's word. Nothing
    is sent. The shell reads an action's grade from here on its UI thread while a card waits, and
    `shell_class` is answered on that same thread, so asking it who it is left each waiting on the
    other until the shell gave up and refused the card (#1080). The answer it would have given is
    a mind: what the shell does here it may do for a mind (a glance, a notification's button)."""
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(SHELL_LOOKED_AT)
    try:
        client.connect(wire.default_socket_path(gate.SHELL))
        peer = wire.peer_cred(client)
    except OSError:
        return False
    finally:
        client.close()
    return peer is not None and peer.pid == pid and gate.must_be_the_shell(peer) is None


_UNSET = object()


def side_of_caller(peer=_UNSET, token=_UNSET, ask_shell=None, uid=None, proc="/proc", is_shell=None):
    """PERSON or MIND for the call being handled (the dispatch's own, unless given). Fails towards
    MIND: a caller that cannot be shown to be the person never reaches the person's browser."""
    token = agent_token() if token is _UNSET else token
    peer = caller() if peer is _UNSET else peer
    if token or peer is None:
        return ports.MIND
    if mind_door.is_mind(peer.uid) or peer.uid != (os.getuid() if uid is None else uid):
        return ports.MIND
    if is_bridge_ancestor(peer.pid, proc):
        return ports.MIND
    if (is_shell or is_the_shell)(peer.pid):
        return ports.MIND       # what `shell_class` says of the shell (`classify`, "the companion")
    return ports.PERSON if (ask_shell or shell_class)(peer.pid) == "person" else ports.MIND


class Guard:
    """Decides, and checks, which browser a call uses. The parts that read the machine are
    arguments, so a test can stand in for them."""

    def __init__(self, identify=None, uid=None, side_of=None, starter=None, watch=None,
                 pipes=None):
        self._watch = watch
        self.pipes = pipes
        self.starter = starter or shell_open_mind_browser
        self.identify = identify or listener.identify
        self.uid = uid if uid is not None else os.getuid()
        self.side_of = side_of or side_of_caller

    def side(self):
        return self.side_of()

    def decide(self):
        """(side, gone): who this call is for, decided NOW, before it waits for anything, and a
        function that says the caller has since gone. A call whose caller is gone does nothing."""
        peer = caller()
        watch = self._watch(peer) if self._watch else (Watch(peer.pid) if peer is not None else None)
        if watch is not None and watch.gone():
            return ports.MIND, lambda: True            # exited before it could be asked about
        return self.side_of(), (watch.gone if watch is not None else (lambda: False))

    def vet(self, side):
        """`side`'s browser as a Listener (port 0, the hand-over's serial) if the browser the shell
        handed over for it still runs, as this account, on the profile that makes it that side's:
        Mind View's on Mind View's, the person's on none of a mind's. None when no browser has been
        handed over; Denied (with the sentence) when what was is not that side's. Never a port:
        anything listening on the person's old port is refused by name, whether or not a pipe is
        held, because a Browser that has the pipe and a port too is being driven by something
        other than this service."""
        if side == ports.PERSON:
            self._refuse_the_old_port()
        held = self.pipes.holder(side).current() if self.pipes is not None else None
        if held is None:
            return None
        if self.pipes.uid_of(held.pid) != self.uid:
            raise Denied(self._unclear(side, "it runs as another account"))
        place = listener.place_of_pid(self.pipes.proc, held.pid)
        if side == ports.PERSON and listener.is_mind_side(place):
            raise Denied(
                "Nothing was done. The browser the desktop handed over as your Browser is Mind View's "
                "own. Close it and open your Browser again.")
        if side == ports.MIND and place != listener.MIND_VIEW:
            raise Denied(
                "Nothing was done. The browser the desktop handed over as Mind View's is not on Mind "
                "View's profile, so it may be the person's, with their sign-ins and tabs. A mind does "
                "not use it: no page was opened, read or changed and no tab list is shown. Only the "
                "person can change this: they can close that browser, or do this part themselves. "
                "Trying again will not change it.")
        place = listener.PERSON if side == ports.PERSON else listener.MIND_VIEW
        return listener.Listener(0, self.uid, held.serial, (held.pid,), place)

    def _refuse_the_old_port(self):
        try:
            self.identify(ports.PERSON_PORT)
        except listener.Unidentified as e:
            if e.nobody:
                return
        raise Denied(
            "Nothing was done. Something listens on a DevTools port (%d) that your Browser no longer "
            "has: it is driven over a pipe the desktop opens it with, and a port beside the pipe "
            "lets any process of this account drive it. A browser started before this update, or "
            "outside the desktop, holds it. Close it and open your Browser again." % ports.PERSON_PORT)

    def start_mind_browser(self, wait=START_WAIT):
        """Mind View's browser was wanted and is not open: ask the shell for it and wait for it to be
        handed over. Returns the listener, or raises Denied saying what is true."""
        try:
            self.starter()
        except Starting as e:
            raise Denied("Mind View's browser is not open and could not be started: %s. Nothing was "
                         "done; the person's Browser is a different one and is not yours to use." % e) from None
        deadline = time.monotonic() + wait
        while True:
            held = self.vet(ports.MIND)
            if held is not None:
                return held
            if time.monotonic() > deadline:
                raise Denied("Mind View's browser was asked to open but was not handed over "
                             "within %d s. Nothing was done." % wait)
            time.sleep(0.3)

    @staticmethod
    def _unclear(side, why):
        whose = "Mind View's browser" if side == ports.MIND else "the person's Browser"
        return ("Nothing was done. Which browser holds its pipe, where %s should be, could not be "
                "established (%s), so it is not used. Only the person can change this: they can "
                "close that browser, or do this part themselves." % (whose, why))
