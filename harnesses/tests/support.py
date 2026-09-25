"""A desktop that is not a desktop, and the paths to the code under test.

Everything here is offline and stdlib-only: `python3 -m unittest discover harnesses/tests` on a
machine with no Yantrik OS, no `pi`, no `node` and no API key.

The fake desktop is the important part. It speaks the six harness methods (docs/harness.md) and,
unlike the real host, it *remembers* — so a test can ask the question that actually matters about
a harness: was this turn closed exactly once?
"""

from __future__ import annotations

import json
import os
import socket
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parents[2]
HARNESSES = ROOT / "harnesses"
for _path in (HARNESSES / "lib", HARNESSES / "deepseek", HARNESSES / "pi"):
    if str(_path) not in sys.path:
        sys.path.insert(0, str(_path))

# Agent processes are started in a directory under the data home (#183). The suite points the
# data home somewhere throwaway so that no test writes into the real ~/.local/share, and so a
# test that needs a data home of its own has a previous value to restore.
os.environ["XDG_DATA_HOME"] = tempfile.mkdtemp(prefix="yantrik-tests-data-")

FAKE_MCP = str(Path(__file__).resolve().parent / "fake_mcp_server.py")
FAKE_PI = str(Path(__file__).resolve().parent / "fake_pi.py")

HAS_UNIX_SOCKETS = hasattr(socket, "AF_UNIX")


def wait_for(predicate: Callable[[], bool], timeout: float = 5.0, interval: float = 0.02) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(interval)
    return predicate()


class Recorder:
    """Stands in for the harness, so a `Turn` can be driven without a socket."""

    def __init__(self) -> None:
        self.deltas: List[str] = []
        self.events: List[Dict[str, Any]] = []

    def _chunk(self, turn: Any, delta: str) -> bool:
        self.deltas.append(delta)
        return True

    def _event(self, turn: Any, event: Dict[str, Any]) -> bool:
        self.events.append(event)
        return True

    def kinds(self) -> List[str]:
        return [e.get("kind") for e in self.events]

    def calls(self) -> Dict[str, List[Dict[str, Any]]]:
        """Each tool call's events, in order, by call id."""
        out: Dict[str, List[Dict[str, Any]]] = {}
        for event in self.events:
            if "call" in event:
                out.setdefault(event["call"], []).append(event)
        return out


def recording_turn(text: str, context: Optional[str] = None, conversation: str = "main",
                   agent_token: str = ""):
    """A real `Turn` — trail framing and all — writing into a list."""
    from yantrik_harness import Turn

    recorder = Recorder()
    return Turn(recorder, "s1", 1, text, context, conversation=conversation,
                agent_token=agent_token), recorder


def said(recorder: Recorder) -> str:
    return "".join(recorder.deltas)


class FakeDesktop:
    """The harness socket, with a memory.

    One JSON-RPC request per line, one reply, a fresh connection per call — the same shape the
    shell serves, including the two rules a harness gets caught by: a call on a session that is
    gone is an error telling it to attach again, and closing a turn twice is an error because the
    second close is for a turn this harness was never given.

    It takes `harness.event` as the real host does — for a turn in flight, and a refusal is an
    answer rather than an error — unless it is made with `events=False`, which is a desktop from
    before events: the method is unknown. It does NOT hand a conversation one turn at a time;
    that is the host's rule, tested in Rust, and a fake that enforced it would hide whether the
    library enforces its own.
    """

    def __init__(self, events: bool = True) -> None:
        self.dir = tempfile.mkdtemp(prefix="yantrik-fake-")
        self.path = os.path.join(self.dir, "harness.sock")
        self.lock = threading.Lock()
        self.attachments: List[Dict[str, Any]] = []
        self.sessions: Dict[str, str] = {}
        self.queue: List[Dict[str, Any]] = []
        self.deltas: Dict[int, List[str]] = {}
        self.closes: List[Tuple[int, str, Optional[str]]] = []
        self.rejected: List[Tuple[str, Dict[str, Any]]] = []
        self.drop: set = set()          # turns the panel has stopped listening to
        self.open_turns: set = set()
        self.events_supported = events
        self.events: Dict[int, List[Dict[str, Any]]] = {}
        self.stopped: set = set()       # turns the desktop cancelled; still open until closed
        self.notices: Dict[str, List[Any]] = {"cancelled": [], "ended": []}
        self._next_turn = 1
        self._next_session = 1
        self._stop = threading.Event()

        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(self.path)
        self.server.listen(16)
        self.server.settimeout(0.2)
        self.thread = threading.Thread(target=self._serve, name="fake-desktop", daemon=True)
        self.thread.start()

    # ── what a test drives ──────────────────────────────────────────────

    def ask(self, text: str, context: Optional[str] = None, conversation: Optional[str] = None,
            agent_token: Optional[str] = None) -> int:
        """Type something into the panel. Returns the turn id it will be handed out as."""
        with self.lock:
            turn_id = self._next_turn
            self._next_turn += 1
            turn = {"turn_id": turn_id, "text": text}
            if context is not None:
                turn["context"] = context
            if conversation is not None:
                turn["conversation"] = conversation
            if agent_token is not None:
                turn["agent_token"] = agent_token
            self.queue.append(turn)
            self.deltas[turn_id] = []
            self.events[turn_id] = []
            return turn_id

    def stop_agent(self, turn_id: int = 0, conversation: Optional[str] = None) -> None:
        """What `Host::stop_agent` tells a harness: on its next poll, this turn is cancelled and
        this conversation is ended. The turn stays open until the harness closes it."""
        with self.lock:
            if turn_id:
                self.stopped.add(turn_id)
                self.notices["cancelled"].append(turn_id)
            if conversation is not None:
                self.notices["ended"].append(conversation)

    def events_for(self, turn_id: int) -> List[Dict[str, Any]]:
        with self.lock:
            return list(self.events.get(turn_id, []))

    def kinds(self, turn_id: int) -> List[str]:
        return [e.get("kind") for e in self.events_for(turn_id)]

    def invalidate(self) -> None:
        """The shell restarted: every session id a harness is holding is now worthless."""
        with self.lock:
            self.sessions.clear()

    def text(self, turn_id: int) -> str:
        with self.lock:
            return "".join(d for d in self.deltas.get(turn_id, []) if d)

    def heartbeats(self, turn_id: int) -> int:
        with self.lock:
            return sum(1 for d in self.deltas.get(turn_id, []) if d == "")

    def closes_for(self, turn_id: int) -> List[Tuple[int, str, Optional[str]]]:
        with self.lock:
            return [c for c in self.closes if c[0] == turn_id]

    def wait_closed(self, turn_id: int, timeout: float = 5.0) -> Tuple[int, str, Optional[str]]:
        if not wait_for(lambda: bool(self.closes_for(turn_id)), timeout):
            raise AssertionError("turn %d was never closed" % turn_id)
        return self.closes_for(turn_id)[0]

    def stop(self) -> None:
        self._stop.set()
        try:
            self.server.close()
        except OSError:
            pass
        self.thread.join(timeout=2)
        try:
            os.unlink(self.path)
        except OSError:
            pass

    # ── the socket ──────────────────────────────────────────────────────

    def _serve(self) -> None:
        while not self._stop.is_set():
            try:
                conn, _ = self.server.accept()
            except (socket.timeout, OSError):
                continue
            threading.Thread(target=self._one, args=(conn,), daemon=True).start()

    def _one(self, conn: socket.socket) -> None:
        with conn:
            conn.settimeout(5)
            buf = b""
            try:
                while not buf.endswith(b"\n"):
                    piece = conn.recv(65536)
                    if not piece:
                        return
                    buf += piece
                request = json.loads(buf)
                result, error = self._handle(request.get("method"), request.get("params") or {})
            except Exception as exc:  # a fake that crashes must not hang the harness
                result, error = None, str(exc)
            reply: Dict[str, Any] = {"jsonrpc": "2.0", "id": 1}
            if error is None:
                reply["result"] = result
            else:
                reply["error"] = {"code": -32000, "message": error}
            try:
                conn.sendall(json.dumps(reply).encode("utf-8") + b"\n")
            except OSError:
                pass

    def _handle(self, method: str, params: Dict[str, Any]):
        with self.lock:
            if method == "harness.attach":
                self.attachments.append(params)
                session = "s%d" % self._next_session
                self._next_session += 1
                self.sessions[session] = str(params.get("id"))
                return {"session": session}, None

            session = str(params.get("session") or "")
            if session not in self.sessions:
                self.rejected.append((method, params))
                return None, "this session is not attached any more; call harness.attach again"

            if method == "harness.poll":
                reply: Dict[str, Any] = {}
                for key in ("cancelled", "ended"):
                    if self.notices[key]:
                        reply[key] = self.notices[key]
                        self.notices[key] = []
                if self.queue:
                    turn = self.queue.pop(0)
                    self.open_turns.add(turn["turn_id"])
                    reply.update(turn)
                return reply, None

            if method == "harness.event" and self.events_supported:
                turn_id = params.get("turn_id")
                if turn_id not in self.open_turns:
                    return None, "turn %s is not one this harness was given" % turn_id
                if turn_id in self.stopped or turn_id in self.drop:
                    return {"dropped": True}, None
                event = params.get("event") or {}
                if len(json.dumps(event)) > 64 * 1024:
                    return {"refused": "too big"}, None
                self.events.setdefault(turn_id, []).append(event)
                return {}, None

            if method == "harness.chunk":
                turn_id = params.get("turn_id")
                if turn_id not in self.open_turns:
                    return None, "turn %s is not one this harness was given" % turn_id
                if turn_id in self.stopped:
                    return {"dropped": True}, None
                self.deltas.setdefault(turn_id, []).append(str(params.get("delta") or ""))
                if turn_id in self.drop:
                    self.open_turns.discard(turn_id)
                    return {"dropped": True}, None
                return {}, None

            if method in ("harness.complete", "harness.fail"):
                turn_id = params.get("turn_id")
                if turn_id not in self.open_turns:
                    # Exactly the error the real host gives, and the one that catches a harness
                    # closing a turn twice.
                    return None, "turn %s is not one this harness was given" % turn_id
                self.open_turns.discard(turn_id)
                self.closes.append((
                    turn_id,
                    "complete" if method == "harness.complete" else "fail",
                    params.get("error"),
                ))
                return {}, None

            if method == "harness.detach":
                self.sessions.pop(session, None)
                return {}, None

            return None, "unknown method %s" % method
