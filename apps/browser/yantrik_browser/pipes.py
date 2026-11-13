"""The person's Browser, held over its DevTools pipe (#1137).

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

from . import guard, handover, listener, transport

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
        """Take a browser's pipe. A new hand-over replaces the old one only when the old browser
        is gone: a live browser's pipe is never taken away by a later offer."""
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
            try:
                reply = self.take(conn)
            except (OSError, ValueError) as e:
                reply = "refused: %s" % e
            try:
                conn.sendall((reply + "\n").encode())
            except OSError:
                return
            if reply == "ok":
                self._hold_lease(conn)

    def _hold_lease(self, conn):
        """Keep the shell's connection open while its browser is held: the shell reads its end
        closing as 'the service lost the pipe' and offers it again."""
        serial = self.held.serial
        conn.settimeout(LEASE_POLL)
        while self.server is not None:
            held = self.current()
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
        """One hand-over on `conn`: a JSON line {"pid": N} and the stream's end as an SCM_RIGHTS
        fd. Only the installed shell may give a browser, only one it started with the pipe, and
        only the stream that browser holds. Returns the reply line."""
        peer = peer if peer is not None else wire.peer_cred(conn)
        if peer is None or peer.uid != os.getuid():
            return "refused: only the desktop's shell hands over a browser"
        why = handover.shell_is_installed(peer.pid, proc=self.proc)
        if why is not None:
            return "refused: only the desktop's shell hands over a browser (%s)" % why
        data, fds, _, _ = socket.recv_fds(conn, HEADER_MAX, 1)
        try:
            if len(fds) != 1:
                raise ValueError("one stream is handed over, not %d" % len(fds))
            header = json.loads(data.split(b"\n")[0].decode("utf-8", "replace"))
            pid = header.get("pid")
            if not isinstance(pid, int) or pid <= 1:
                raise ValueError("no browser pid")
            sock = socket.socket(fileno=fds[0])
            fds = []
        finally:
            for fd in fds:
                os.close(fd)
        why = self._not_the_persons_browser(sock, pid, peer.pid)
        if why is not None:
            sock.close()
            return "refused: " + why
        return "ok" if self.give(sock, pid) else "refused: a Browser is already held"

    def _not_the_persons_browser(self, sock, pid, shell_pid):
        if self.uid_of(pid) != os.getuid():
            return "that browser is not run by this account"
        why = (handover.browser_is_the_callers(pid, shell_pid, self.proc)
               or handover.stream_is_the_browsers(sock, pid, self.proc))
        if why is not None:
            return why
        if listener.place_of_pid(self.proc, pid) != listener.PERSON:
            return "that is a mind's browser, not the person's"
        return None
