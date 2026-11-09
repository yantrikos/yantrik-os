"""The person's Browser, held over its DevTools pipe (#1137).

The person's Chromium runs with `--remote-debugging-pipe` and listens on no TCP port at all. The
shell starts it (it owns every launch: window, session environment, registry), makes the
connected socket pair its fds 3 and 4 are, and hands the other end to this service over
`browser-pipe.sock` (SCM_RIGHTS). From then on this service is the only holder of that stream:
a process running as the person has no address to connect to, and the socket cannot be reopened
through /proc/<pid>/fd as a pipe could.

Why the shell launches and the service holds, rather than the service launching the browser: the
launch is the shell's everywhere else (browser_launch.rs, the same code that starts Mind View's
browser), and a browser started by the service would sit in the service's cgroup and die or
restart with it. Handing one fd over costs nothing and keeps one owner for each fact.

The price: a browser lives on one pipe, so if this service restarts the person's open Browser
cannot be driven until it is opened again. The guard says so; it never falls back to a port.
"""

import json
import os
import socket
import threading

from yantrik_surface import gate, wire

from . import guard, listener, transport

SOCKET_NAME = "browser-pipe.sock"
HEADER_MAX = 4096


def socket_path():
    return os.path.join(wire.socket_dir(), SOCKET_NAME)


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

    # ── what is held ──

    def give(self, sock, pid):
        """Take a browser's pipe. A new hand-over replaces the old one only when the old browser
        is gone: a live browser's pipe is never taken away by a later offer."""
        with self.lock:
            if self.current() is not None:
                sock.close()
                return False
            self._drop()
            self.serial += 1
            self.held = Held(sock, pid, self._start_of(pid), self.serial)
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
                pass

    def take(self, conn, peer=None):
        """One hand-over on `conn`: a JSON line {"pid": N} and the stream's end as an SCM_RIGHTS
        fd. Only the desktop's own shell may give a browser. Returns the reply line."""
        peer = peer if peer is not None else wire.peer_cred(conn)
        why = gate.must_be_the_shell(peer)
        if why is not None or peer.uid != os.getuid():
            return "refused: only the desktop's shell hands over a browser"
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
        if self.uid_of(pid) != os.getuid():
            sock.close()
            return "refused: that browser is not run by this account"
        if listener.place_of_pid(self.proc, pid) != listener.PERSON:
            sock.close()
            return "refused: that is a mind's browser, not the person's"
        return "ok" if self.give(sock, pid) else "refused: a Browser is already held"
