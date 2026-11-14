"""The browsers' DevTools pipes, held here: the person's Browser (#1137) and Mind View's (#1200).

Both go through one hand-over (`Pipes.take`), one socket and one set of checks; the header says
which side the shell offers a pipe for, and `place_of_pid` (the profile the process owns) must
agree, so neither can be handed over as the other. Mind View's browser has no port: a job that
speaks raw DevTools to a port goes round the fence this service puts on a browser's tabs.

The person's Chromium runs with `--remote-debugging-pipe` and listens on no TCP port at all. The
shell starts it (it owns every launch: window, session environment, registry), makes the
connected socket pair its fds 3 and 4 are, and hands the other end to this service over
`browser-pipe.sock` (SCM_RIGHTS). The stream cannot be reopened through /proc/<pid>/fd as a pipe
could. That is not a wall against a process of the same account: it can still bind a socket of
its own at `browser-pipe.sock` (the shell therefore checks who answers before it sends anything,
browser_pipe.rs), and until Minds run as another account it can start its own browser on the
default profile with a port. See #1137 step 3 for the part that makes this a boundary.

Why the shell launches and the service holds, rather than the service launching the browser: the
launch is the shell's everywhere else (browser_launch.rs, the same code that starts Mind View's
browser), and a browser started by the service would sit in the service's cgroup and die or
restart with it. Handing one fd over costs nothing and keeps one owner for each fact.

A browser lives on one stream, and Chromium closes itself when that stream ends (measured on
headless 153: it exits within about half a second of the other end closing). So the shell keeps
its own end open for as long as the browser runs and stays on this socket as a lease: the
connection it handed over on is held open here while the browser is held. If this service
restarts, the lease ends, the browser is not told (the shell's end is still open), and the shell
offers the stream again to the new service; what the browser wrote meanwhile (replies to the dead
process's calls, events of sessions it attached) is read off and dropped before the stream is
held, so a reply never lands on one of the new process's ids. The guard never falls back to a port.
"""

import json
import os
import socket
import threading

from yantrik_surface import wire

from . import guard, handover, listener, ports, transport

SOCKET_NAME = "browser-pipe.sock"
HEADER_MAX = 4096
LEASE_POLL = 1.0


def socket_path():
    return os.path.join(wire.socket_dir(), SOCKET_NAME)


def _drain(sock):
    """Read and drop whatever is queued on `sock`: after a service restart the browser's stream
    carries the dead process's replies and events, which a new reader would file under its own
    ids (review of aff60d15, L-a)."""
    sock.setblocking(False)
    try:
        while sock.recv(65536):
            pass
    except (BlockingIOError, OSError):
        pass
    finally:
        sock.setblocking(True)


class Held:
    """The browser whose pipe this is: its pid, and when that pid started (so a reused pid is not
    it), and which pipe it is (a new hand-over is a new serial)."""

    def __init__(self, sock, pid, start, serial):
        self.sock, self.pid, self.start, self.serial = sock, pid, start, serial


class PipeHolder:
    """One side's browser pipe: what is held, and the connection in use over it."""

    def __init__(self, proc="/proc", start_of=None):
        self.proc = proc
        self._start_of = start_of or (lambda pid: guard._start_of(pid, proc))  # noqa: SLF001
        self.held = None
        self.serial = 0
        self.lock = threading.Lock()
        self.server = None
        self.open_transport = None      # the transport of the connection in use, if any
        self.on_give = None             # called (off this lock) once a browser's pipe is held

    # ── what is held ──

    def give(self, sock, pid):
        """Take this side's browser's pipe. A new hand-over replaces the old one only when the old
        browser is gone: a live browser's pipe is never taken away by a later offer."""
        with self.lock:
            if self.current() is not None:
                sock.close()
                return False
            self._drop()
            _drain(sock)
            self.serial += 1
            self.held = Held(sock, pid, self._start_of(pid), self.serial)
        if self.on_give:
            try:
                self.on_give()
            except Exception:  # noqa: BLE001 - a listener's failure never loses the pipe
                pass
        return True

    def _drop(self):
        held, self.held = self.held, None
        if held is not None:
            try:
                held.sock.close()
            except OSError:
                pass

    def current(self):
        """The Held browser if it still runs (same pid, same start time), else None."""
        held = self.held
        if held is None:
            return None
        if held.start is None or self._start_of(held.pid) != held.start:
            return None
        return held

    def forget_if_gone(self):
        with self.lock:
            if self.held is not None and self.current() is None:
                self._drop()

    def uid_of(self, pid):
        try:
            return os.stat(os.path.join(self.proc, str(pid))).st_uid
        except OSError:
            return None

    # ── a connection over it ──

    def open(self):
        """A transport for cdp.Browser: the held pipe, or cdp.BrowserClosed when there is none.
        The browser's stream is one: the previous connection is let go first."""
        from . import cdp
        with self.lock:
            held = self.current()
            if held is None:
                raise cdp.BrowserClosed()
            previous, self.open_transport = self.open_transport, None
            if previous is not None:
                previous.close()
            fd = os.dup(held.sock.fileno())
            self.open_transport = transport.PipeTransport(fd)
            return self.open_transport


class Pipes:
    """The hand-over: the shell offers a browser's stream over `browser-pipe.sock`, and the holder
    of the side it is offered for takes it. One path for both browsers."""

    def __init__(self, proc="/proc", start_of=None):
        self.proc = proc
        self.holders = {ports.PERSON: PipeHolder(proc, start_of), ports.MIND: PipeHolder(proc, start_of)}
        self.server = None

    def holder(self, side):
        return self.holders[side]

    def give(self, side, sock, pid):
        """Hold `sock` as `side`'s browser's pipe, with no checks: a hand-over that passed them
        (`take`), or a test that stands in for the shell."""
        return self.holders[side].give(sock, pid)

    def forget_if_gone(self):
        for holder in self.holders.values():
            holder.forget_if_gone()

    def uid_of(self, pid):
        return self.holders[ports.PERSON].uid_of(pid)

    # ── the hand-over socket ──

    def serve(self, path=None):
        """Listen for the shell's hand-over on a socket in the session's 0700 directory."""
        path = path or socket_path()
        try:
            os.unlink(path)
        except OSError:
            pass
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        old = os.umask(0o177)
        try:
            server.bind(path)
        finally:
            os.umask(old)
        server.listen(4)
        self.server = (server, path)
        threading.Thread(target=self._accept, args=(server,), name="browser-pipe", daemon=True).start()
        return path

    def stop(self):
        if self.server is not None:
            server, path = self.server
            self.server = None
            try:
                server.close()
                os.unlink(path)
            except OSError:
                pass

    def _accept(self, server):
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            threading.Thread(target=self._receive, args=(conn,), daemon=True).start()

    def _receive(self, conn):
        with conn:
            conn.settimeout(5.0)
            holder = None
            try:
                reply, holder = self._take(conn)
            except (OSError, ValueError, listener.Unidentified) as e:
                # `Unidentified`: which profile the browser owns could not be read this once, so the
                # shell is told to offer again ("retry:"), never left with silence, which it would
                # read as a final answer.
                reply = ("retry: %s" if isinstance(e, listener.Unidentified) else "refused: %s") % e
            try:
                conn.sendall((reply + "\n").encode())
            except OSError:
                return
            if reply == "ok":
                self._hold_lease(conn, holder)

    def _hold_lease(self, conn, holder):
        """Keep the shell's connection open while its browser is held: the shell reads its end
        closing as 'the service lost the pipe' and offers it again."""
        serial = holder.held.serial
        conn.settimeout(LEASE_POLL)
        while self.server is not None:
            held = holder.current()
            if held is None or held.serial != serial:
                return
            try:
                if conn.recv(1) == b"":
                    return
            except socket.timeout:
                continue
            except OSError:
                return

    def take(self, conn, peer=None):
        """One hand-over on `conn`; the reply line (`_take` also says whose pipe it now is)."""
        return self._take(conn, peer)[0]

    def _take(self, conn, peer=None):
        """One hand-over on `conn`: a JSON line {"pid": N, "side": "person"|"mind"} and the stream's
        end as an SCM_RIGHTS fd. Only the installed shell may give a browser, only one it started
        with the pipe and no port, only the stream that browser holds, and only for the side whose
        profile it runs on. Returns (the reply line, the holder that took it or None)."""
        peer = peer if peer is not None else wire.peer_cred(conn)
        if peer is None or peer.uid != os.getuid():
            return "refused: only the desktop's shell hands over a browser", None
        why = handover.shell_is_installed(peer.pid, proc=self.proc)
        if why is not None:
            return "refused: only the desktop's shell hands over a browser (%s)" % why, None
        data, fds, _, _ = socket.recv_fds(conn, HEADER_MAX, 1)
        try:
            if len(fds) != 1:
                raise ValueError("one stream is handed over, not %d" % len(fds))
            header = json.loads(data.split(b"\n")[0].decode("utf-8", "replace"))
            pid = header.get("pid")
            if not isinstance(pid, int) or pid <= 1:
                raise ValueError("no browser pid")
            side = header.get("side", ports.PERSON)
            if side not in self.holders:
                raise ValueError("no such side: %r" % (side,))
            sock = socket.socket(fileno=fds[0])
            fds = []
        finally:
            for fd in fds:
                os.close(fd)
        why = self._not_this_sides_browser(side, sock, pid, peer.pid)
        if why is not None:
            sock.close()
            return why, None
        holder = self.holders[side]
        if not holder.give(sock, pid):
            return "refused: %s is already held" % ("a Browser" if side == ports.PERSON else "Mind View's browser"), None
        return "ok", holder

    def _not_this_sides_browser(self, side, sock, pid, shell_pid):
        """None when `pid` is `side`'s browser, else the reply to send: "refused: ..." for an
        answer that will not change, "retry: ..." for one that may. A Mind View browser that does
        not yet own its profile is the second kind: a cold start has not written its
        SingletonLock or opened its LOCK yet (the shell offers again, and stops the browser if it
        never does)."""
        if self.uid_of(pid) != os.getuid():
            return "refused: that browser is not run by this account"
        why = (handover.browser_is_the_callers(pid, shell_pid, self.proc)
               or handover.stream_is_the_browsers(sock, pid, self.proc))
        if why is not None:
            return "refused: " + why
        try:
            place = listener.place_of_pid(self.proc, pid)
        except listener.Unidentified as e:
            return "retry: %s" % e
        if side == ports.PERSON and place != listener.PERSON:
            return "refused: that is a mind's browser, not the person's"
        if side == ports.MIND and place != listener.MIND_VIEW:
            return "retry: that browser does not run on Mind View's profile (yet), so it is not Mind View's"
        return None
