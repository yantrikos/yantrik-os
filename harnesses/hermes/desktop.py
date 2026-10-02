"""The desktop side of the bridge: the harness socket, and which turn a message belongs to.

Nothing here imports Hermes, so it is tested without one. `adapter.py` is the thin part that
translates between this and the gateway.

The socket is the one every harness uses (docs/harness.md): a unix socket, one JSON-RPC request
per line, a fresh connection per call. The OS never calls a harness; a harness attaches, polls for
what the person typed, and streams the answer back.
"""

from __future__ import annotations

import json
import os
import socket
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

ATTACH = "harness.attach"
POLL = "harness.poll"
CHUNK = "harness.chunk"
COMPLETE = "harness.complete"
FAIL = "harness.fail"
DETACH = "harness.detach"


class HarnessError(Exception):
    """The desktop refused a call, or could not be reached."""


def socket_path() -> Optional[str]:
    """Where the desktop's harness socket is, if there is one.

    The same places, in the same order, as the OS binds them and as Yantrik Mind looks: the shell
    takes the first directory it can write, so a client has to find the one it actually chose.
    """
    explicit = os.environ.get("YANTRIK_HARNESS_SOCKET", "").strip()
    if explicit:
        # Named outright: honour it and look nowhere else, so pointing Hermes at one desktop can
        # never silently fall through to another.
        return explicit if Path(explicit).exists() else None

    candidates: List[Path] = []
    runtime = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    if runtime:
        candidates.append(Path(runtime) / "yantrik")
    candidates.append(Path("/run/yantrik"))
    try:
        candidates.extend(sorted(p for p in Path("/tmp").iterdir() if p.name.startswith("yantrik-")))
    except OSError:
        pass
    for directory in candidates:
        sock = directory / "harness.sock"
        if sock.exists():
            return str(sock)
    return None


def call(address: str, method: str, params: Dict[str, Any], timeout: float = 10.0) -> Any:
    """One round trip: connect, write a line, read a line, close."""
    request = json.dumps({"jsonrpc": "2.0", "method": method, "params": params, "id": 1})
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
            conn.settimeout(timeout)
            conn.connect(address)
            conn.sendall(request.encode("utf-8") + b"\n")
            buf = b""
            while not buf.endswith(b"\n"):
                piece = conn.recv(65536)
                if not piece:
                    break
                buf += piece
    except OSError as exc:
        raise HarnessError(f"{method}: {exc}") from exc
    if not buf.strip():
        raise HarnessError(f"{method}: the desktop closed the connection without answering")
    try:
        reply = json.loads(buf)
    except ValueError as exc:
        raise HarnessError(f"{method}: unreadable reply {buf[:200]!r}") from exc
    if isinstance(reply, dict) and reply.get("error"):
        err = reply["error"]
        raise HarnessError(err.get("message", str(err)) if isinstance(err, dict) else str(err))
    return reply.get("result") if isinstance(reply, dict) else None


@dataclass
class Turn:
    """One message the person typed, and what has been said back so far."""

    turn_id: str
    chat_id: str
    text: str
    opened: float = field(default_factory=time.monotonic)
    # When anything was last streamed on this turn, so a quiet turn can be told from a dead one.
    active: float = field(default_factory=time.monotonic)
    said_anything: bool = False
    started: bool = False
    # The gateway session working on it, and how many heartbeats in a row found it not running.
    session_key: str = ""
    idle_beats: int = 0


class Ledger:
    """Which open turn each outgoing message belongs to.

    The gateway thinks in messages — send one, edit it, send another — and the desktop thinks in
    turns: one question, one answer streamed in pieces, then done. A turn can receive several
    gateway messages (commentary between tool calls, the tool-progress line, the answer), and edits
    to a message already streamed can only be appended to, never retracted. This is the bookkeeping
    that makes those two views agree. Thread-safe, because the gateway's agent runs in worker
    threads.
    """

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._turns: Dict[str, Turn] = {}
        self._messages: Dict[str, Tuple[str, str]] = {}  # message id -> (turn id, text so far)
        self._counter = 0

    def open(self, turn_id: str, chat_id: str, text: str) -> Turn:
        with self._lock:
            turn = Turn(turn_id=turn_id, chat_id=chat_id, text=text)
            self._turns[turn_id] = turn
            return turn

    def get(self, turn_id: str) -> Optional[Turn]:
        with self._lock:
            return self._turns.get(turn_id)

    def open_turns(self) -> List[Turn]:
        with self._lock:
            return list(self._turns.values())

    def mark_started(self, turn_id: str) -> None:
        with self._lock:
            if turn_id in self._turns:
                self._turns[turn_id].started = True

    def close(self, turn_id: str) -> Optional[Turn]:
        """Forget a turn. Returns it if it was still open, so it is closed exactly once."""
        with self._lock:
            turn = self._turns.pop(turn_id, None)
            if turn is not None:
                self._messages = {m: v for m, v in self._messages.items() if v[0] != turn_id}
            return turn

    def route(self, chat_id: str, reply_to: Optional[str]) -> Optional[Turn]:
        """The turn a message sent to `chat_id` answers.

        The gateway names the turn when it can — a final answer is sent in reply to the message
        that asked — and that wins. Otherwise it is the newest open turn in the chat, which is the
        one the person is looking at.
        """
        with self._lock:
            if reply_to and reply_to in self._turns:
                return self._turns[reply_to]
            in_chat = [t for t in self._turns.values() if t.chat_id == chat_id]
            return max(in_chat, key=lambda t: t.opened) if in_chat else None

    def sent(self, turn: Turn, content: str) -> Tuple[str, str]:
        """Record a new message on `turn`. Returns (message id, the text to stream)."""
        with self._lock:
            self._counter += 1
            message_id = f"{turn.turn_id}.{self._counter}"
            delta = ("\n\n" if turn.said_anything else "") + content
            turn.said_anything = turn.said_anything or bool(content)
            turn.active = time.monotonic()
            self._messages[message_id] = (turn.turn_id, content)
            return message_id, delta

    def edited(self, message_id: str, content: str) -> Optional[Tuple[Turn, str]]:
        """Record an edit. Returns (turn, the text to stream), or None if its turn is gone.

        What was streamed cannot be taken back, so an edit contributes only what it adds. Growth at
        the end streams as-is. A rewrite — a progress line whose status changed — streams the lines
        from the first one that differs, which repeats a line rather than losing one.
        """
        with self._lock:
            known = self._messages.get(message_id)
            if known is None:
                return None
            turn_id, before = known
            turn = self._turns.get(turn_id)
            if turn is None:
                return None
            self._messages[message_id] = (turn_id, content)
            if content == before:
                return turn, ""
            if content.startswith(before):
                delta = content[len(before):]
            else:
                old, new = before.splitlines(), content.splitlines()
                same = 0
                while same < min(len(old), len(new)) and old[same] == new[same]:
                    same += 1
                delta = "\n" + "\n".join(new[same:]) if new[same:] else ""
            turn.said_anything = turn.said_anything or bool(delta)
            turn.active = time.monotonic()
            return turn, delta
