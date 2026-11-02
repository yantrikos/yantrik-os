"""Handing the desktop's tools bridge its agent token without an environment (#411).

Under the mind account the desktop refuses a mind's acts unless they carry an agent token, and the
tools bridge (`yos-mcp`) has to be given one. In Hermes the bridge is started by Hermes's own MCP
machinery, which passes a child nothing but an environment, so the plugin cannot hand it a pipe. An
environment is where a token must not be: `/proc/<pid>/environ` keeps it for the life of the
process, for every process of the account. So:

- `TokenService` listens on a unix socket in Hermes's own home (`$HERMES_HOME/yantrik-bridge.sock`,
  0600, in a directory only the account can open) and answers one token to a connecting process and
  closes. The bridge's MCP entry in Hermes's config names that path (a path is not a secret); the
  bridge connects once at start, reads the token into memory, drops the variable and makes itself
  non-dumpable (`deploy/yantrik-os/yos-mcp`, `take_agent_token`).
- It answers only a process of the same account that descends from this one (the kernel's
  `SO_PEERCRED` and the parent chain in `/proc`), and nothing at all while there is no token.
- `popen_with_token` is the other way, for a bridge the plugin starts itself: a pipe's descriptor,
  never the environment and never the command line.

The token is the gateway agent's (`hermes:gateway`): what Hermes's background work acts as. It is
never logged, never in an error, and never the person's chat.
"""

from __future__ import annotations

import os
import socket
import struct
import subprocess
import threading
from typing import List, Optional

SOCKET_NAME = "yantrik-bridge.sock"
TOKEN_FD_ENV = "YANTRIK_AGENT_TOKEN_FD"
TOKEN_ENV = "YANTRIK_AGENT_TOKEN"
# How far up the process tree a connecting process may be from this one. Hermes's MCP client starts
# a bridge a few generations down, never hundreds.
DESCENT_BOUND = 32


def socket_path(hermes_home: Optional[str] = None) -> str:
    """Where the token service listens: in Hermes's home, which only the account can open. The same
    path `harnesses/lib/install/hermes_config.py` writes into the bridge's MCP entry."""
    home = hermes_home or os.environ.get("HERMES_HOME") or os.path.join(os.path.expanduser("~"), ".hermes")
    return os.path.join(home, SOCKET_NAME)


def _parent_of(pid: int) -> int:
    try:
        with open("/proc/%d/stat" % pid, "rb") as f:
            # pid (comm) state ppid ...: comm may hold spaces and parentheses, so cut at the last ')'.
            return int(f.read().rsplit(b")", 1)[1].split()[1])
    except (OSError, ValueError, IndexError):
        return 0


def descends_from(pid: int, ancestor: int) -> bool:
    """Whether `pid` is `ancestor` or one of its descendants, by the parent chain."""
    for _ in range(DESCENT_BOUND):
        if pid == ancestor:
            return True
        if pid <= 1:
            return False
        pid = _parent_of(pid)
    return False


class TokenService:
    """Answers the gateway agent's token to a bridge that asks, and to no one else."""

    def __init__(self, path: Optional[str] = None):
        self._path = path or socket_path()
        self._token: Optional[str] = None
        self._lock = threading.Lock()
        self._server: Optional[socket.socket] = None
        self._thread: Optional[threading.Thread] = None

    @property
    def path(self) -> str:
        return self._path

    def set_token(self, token: Optional[str]) -> None:
        """The token the desktop's latest attach gave, or None once the attach is lost: a bridge
        that asks then is answered nothing and acts as no agent (the desktop refuses it)."""
        token = str(token).strip() if token else None
        with self._lock:
            self._token = token or None

    def start(self) -> None:
        if self._server is not None:
            return
        directory = os.path.dirname(self._path)
        os.makedirs(directory, mode=0o700, exist_ok=True)
        try:
            os.unlink(self._path)
        except FileNotFoundError:
            pass
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        old = os.umask(0o177)
        try:
            server.bind(self._path)
        finally:
            os.umask(old)
        server.listen(8)
        self._server = server
        self._thread = threading.Thread(target=self._serve, args=(server,), daemon=True, name="yantrik-token-service")
        self._thread.start()

    def stop(self) -> None:
        server, self._server = self._server, None
        if server is not None:
            try:
                server.close()
            except OSError:
                pass
        try:
            os.unlink(self._path)
        except OSError:
            pass
        self.set_token(None)

    def _peer(self, conn: socket.socket):
        raw = conn.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
        pid, uid, _gid = struct.unpack("3i", raw)
        return pid, uid

    def _serve(self, server: socket.socket) -> None:
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            with conn:
                try:
                    conn.settimeout(2.0)
                    pid, uid = self._peer(conn)
                    with self._lock:
                        token = self._token
                    if token and uid == os.getuid() and descends_from(pid, os.getpid()):
                        conn.sendall(token.encode("utf-8"))
                except (OSError, struct.error):
                    continue


def popen_with_token(command: List[str], token: Optional[str], **kw) -> subprocess.Popen:
    """`subprocess.Popen(command, **kw)` for a bridge, handing it `token` through a pipe's
    descriptor (YANTRIK_AGENT_TOKEN_FD): never in its environment or on its command line. Any
    token variable in the environment it would have inherited is dropped."""
    env = {k: v for k, v in (kw.pop("env", None) or os.environ).items() if k not in (TOKEN_ENV, TOKEN_FD_ENV)}
    if not token:
        return subprocess.Popen(command, env=env, **kw)
    read_end, write_end = os.pipe()
    try:
        os.write(write_end, token.encode("utf-8"))
        os.close(write_end)
        write_end = -1
        env[TOKEN_FD_ENV] = str(read_end)
        return subprocess.Popen(command, env=env, pass_fds=(read_end,), **kw)
    finally:
        os.close(read_end)
        if write_end >= 0:
            os.close(write_end)
