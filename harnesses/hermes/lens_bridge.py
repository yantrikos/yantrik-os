"""The second `yos-mcp`: the one that carries the Lens conversation's token (#1070, item 4).

`Bridge` is one `yos-mcp` child process spoken to over stdio (MCP, one JSON object per line).
`LensBridge` owns the one that carries the conversation's agent token, which arrives on every
`harness.poll` and is replaced when the person's + begins a new conversation. See `lens_route.py`
for which calls reach it.

What this file keeps true:

- The token is passed in the child's environment (`YANTRIK_AGENT_TOKEN`) and nowhere else: not on
  a command line, not in a log line, not in an error, and not in anything handed back to a tool's
  caller (every string from the child is scrubbed of it before it leaves this file).
- A token that changes ends the process that carried the old one. A call in flight on it fails; it
  is not retried on another bridge, as a retry could do a thing twice or under another trust.
- The child never outlives the gateway: it is stopped on `stop()`, at exit, and by the kernel if
  the gateway is killed (`PR_SET_PDEATHSIG`), and it runs in a session of its own so that its own
  children go with it.
"""

from __future__ import annotations

import atexit
import json
import logging
import os
import signal
import subprocess
import sys
import threading
from typing import Any, Dict, List, Optional, Tuple

logger = logging.getLogger(__name__)

COMMAND = os.environ.get("YOS_MCP_BIN") or "/opt/yantrik/bin/yos-mcp"
TOKEN_ENV = "YANTRIK_AGENT_TOKEN"
PROTOCOL = "2024-11-05"
CLIENT = {"name": "Hermes Agent", "version": "yantrik-desktop"}
# The bridge may hold a call while a person answers a card (README: up to 270 seconds).
CALL_SECONDS = 300.0
START_SECONDS = 20.0


class Unavailable(Exception):
    """The bridge could not take or finish the call. Nothing is retried elsewhere."""


def _scrub(value: Any, token: Optional[str]) -> Any:
    """`value` without the token, whatever shape it is in."""
    if not token:
        return value
    if isinstance(value, str):
        return value.replace(token, "[token]")
    if isinstance(value, list):
        return [_scrub(v, token) for v in value]
    if isinstance(value, dict):
        return {k: _scrub(v, token) for k, v in value.items()}
    return value


def _die_with_parent() -> None:  # pragma: no cover - runs in the child, before exec
    if sys.platform.startswith("linux"):
        try:
            import ctypes

            ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGTERM)  # PR_SET_PDEATHSIG
        except Exception:
            pass


class Bridge:
    """One `yos-mcp` process. `token=None` is the tokenless bridge."""

    def __init__(self, command: List[str], token: Optional[str] = None):
        self._command = command
        self._token = token or None
        self._proc: Optional[subprocess.Popen] = None
        self._lock = threading.Lock()
        self._next = 1
        self._waiting: Dict[int, Tuple[threading.Event, list]] = {}
        self._gone = False

    @property
    def pid(self) -> Optional[int]:
        return self._proc.pid if self._proc is not None else None

    @property
    def alive(self) -> bool:
        return self._proc is not None and self._proc.poll() is None and not self._gone

    def _env(self) -> Dict[str, str]:
        env = {k: v for k, v in os.environ.items() if k != TOKEN_ENV}
        if self._token:
            env[TOKEN_ENV] = self._token
        env["YOS_MCP_FOLLOW"] = env.get("YOS_MCP_FOLLOW", "1")
        return env

    def start(self) -> None:
        with self._lock:
            if self.alive:
                return
            try:
                self._proc = subprocess.Popen(
                    self._command,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,  # a child's stderr could echo what it was given
                    env=self._env(),
                    start_new_session=True,
                    preexec_fn=_die_with_parent if sys.platform.startswith("linux") else None,
                    text=True,
                    bufsize=1,
                )
            except OSError as exc:
                raise Unavailable("the desktop's tools could not be started: %s" % exc.__class__.__name__) from None
            self._gone = False
            threading.Thread(target=self._read, args=(self._proc,), daemon=True, name="yantrik-bridge-read").start()
        self._request("initialize", {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": CLIENT}, START_SECONDS)
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _read(self, proc: subprocess.Popen) -> None:
        try:
            for line in proc.stdout:
                try:
                    msg = json.loads(line)
                except ValueError:
                    continue
                slot = self._waiting.pop(msg.get("id"), None) if isinstance(msg, dict) else None
                if slot is not None:
                    slot[1].append(msg)
                    slot[0].set()
        except (OSError, ValueError):
            pass
        finally:
            self._gone = True
            for event, _ in list(self._waiting.values()):
                event.set()  # nobody is left to answer: wake the waiters, who find no reply

    def _send(self, message: Dict[str, Any]) -> None:
        proc = self._proc
        if proc is None or proc.stdin is None or not self.alive:
            raise Unavailable("the desktop's tools stopped")
        try:
            with self._lock:
                proc.stdin.write(json.dumps(message) + "\n")
                proc.stdin.flush()
        except (OSError, ValueError):
            raise Unavailable("the desktop's tools stopped") from None

    def _request(self, method: str, params: Dict[str, Any], timeout: float) -> Any:
        with self._lock:
            ident = self._next
            self._next += 1
            slot: Tuple[threading.Event, list] = (threading.Event(), [])
            self._waiting[ident] = slot
        self._send({"jsonrpc": "2.0", "id": ident, "method": method, "params": params})
        if not slot[0].wait(timeout) or not slot[1]:
            self._waiting.pop(ident, None)
            raise Unavailable("the desktop's tools did not answer")
        reply = slot[1][0]
        if reply.get("error"):
            raise Unavailable("the desktop's tools refused %s" % method)
        return reply.get("result")

    def list_tools(self) -> List[Dict[str, Any]]:
        self.start()
        result = self._request("tools/list", {}, START_SECONDS) or {}
        return [t for t in result.get("tools", []) if isinstance(t, dict) and t.get("name")]

    def call(self, name: str, arguments: Dict[str, Any], timeout: float = CALL_SECONDS) -> Tuple[str, bool]:
        """`(text, is_error)`, scrubbed of the token."""
        self.start()
        result = self._request("tools/call", {"name": name, "arguments": arguments}, timeout) or {}
        text = "".join(c.get("text", "") for c in result.get("content", []) if isinstance(c, dict))
        return _scrub(text, self._token), bool(result.get("isError"))

    def stop(self) -> None:
        proc, self._proc = self._proc, None
        self._gone = True
        if proc is None:
            return
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(proc.pid, sig) if hasattr(os, "killpg") else proc.send_signal(sig)
            except (OSError, ProcessLookupError):
                break
            try:
                proc.wait(2.0)
                break
            except subprocess.TimeoutExpired:
                continue
        for stream in (proc.stdin, proc.stdout):
            try:
                if stream:
                    stream.close()
            except OSError:
                pass


class LensBridge:
    """The bridge for the Lens conversation: the child, and the token it was started with."""

    def __init__(self, command: Optional[List[str]] = None):
        self._command = command or [COMMAND]
        self._token: Optional[str] = None
        self._bridge: Optional[Bridge] = None
        self._lock = threading.Lock()
        atexit.register(self.stop)

    @property
    def token_current(self) -> bool:
        """Whether there is a token the desktop gave for the conversation as it stands."""
        return self._token is not None

    def set_token(self, token: Optional[str]) -> None:
        """The token the desktop's latest poll carried. A different one, or none, ends the process
        that carried the old: a rotated token never leaves a bridge running under the old one."""
        token = str(token).strip() if token else None
        with self._lock:
            if token == self._token:
                return
            old, self._bridge, self._token = self._bridge, None, token
        if old is not None:
            old.stop()
            logger.info("[yantrik] the Lens bridge's token changed; its bridge was stopped")

    def _current(self) -> Tuple[Bridge, str]:
        with self._lock:
            if self._token is None:
                raise Unavailable("no conversation token")
            if self._bridge is None:
                self._bridge = Bridge(self._command, self._token)
            return self._bridge, self._token

    def call(self, name: str, arguments: Dict[str, Any]) -> Tuple[str, bool]:
        bridge, token = self._current()
        try:
            text, is_error = bridge.call(name, arguments)
        except Unavailable as exc:
            raise Unavailable(_scrub(str(exc), token)) from None
        with self._lock:
            if self._token != token:
                # The token moved while this was running. What came back was made under the old
                # one; it is not handed on as the new conversation's.
                raise Unavailable("the conversation changed while this ran")
        return text, is_error

    def stop(self) -> None:
        with self._lock:
            old, self._bridge, self._token = self._bridge, None, None
        if old is not None:
            old.stop()

    @property
    def pid(self) -> Optional[int]:
        return self._bridge.pid if self._bridge is not None else None


def tool_list(command: Optional[List[str]] = None) -> List[Dict[str, Any]]:
    """The bridge's tools, from a short-lived tokenless process (no token is involved)."""
    bridge = Bridge(command or [COMMAND], None)
    try:
        return bridge.list_tools()
    finally:
        bridge.stop()
