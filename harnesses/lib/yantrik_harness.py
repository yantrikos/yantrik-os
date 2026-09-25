"""The half of a harness that is not the mind.

Every harness in this repo does the same six-method dance with the desktop (docs/harness.md) and
the same stdio dance with `yos-mcp`, and gets the same handful of things wrong when it is written
again from scratch: a turn closed twice, a turn never closed, a heartbeat that stops when the work
gets slow, a second message swallowed while the first one is running. That is what is here, once,
so a new harness is only the part that produces an answer.

Stdlib only, Python 3.10 and later. Nothing here imports the Hermes plugin — the discovery rules
are reimplemented rather than shared, because `harnesses/hermes` ships to a machine that has
Hermes and this ships to machines that do not.

What a harness supplies is a handler:

    class Mind(Handler):
        concurrent = False                 # can two turns run at once?
        def answer(self, turn): ...        # turn.text in, turn.emit(...) out
        def reset(self): ...               # /new
        def cancel(self, turn): ...        # /stop, on top of turn.cancelled being set

    Harness("deepseek", "DeepSeek", Mind(), detail="…", tools=True).run()

The one rule the desktop actually enforces: a turn that was handed over is owed exactly one
`complete` or `fail`. Whatever the handler does — return, raise, emit nothing, get stopped — that
happens here, once, in `_close`.

Beside the text, a turn can say what the agent is doing — `turn.tool_start / tool_output /
tool_end / thinking / status / usage` — and the desktop draws each tool call as a card in the
agent's pane. Every one of them is optional, and against a desktop too old to take them they are
quietly skipped: the trail line `tool_start` writes into the text is still there.

A mind that can hold more than one conversation at once wraps itself in `PerConversation`: one
handler per conversation, made when its first turn arrives and let go when the desktop ends it.
Turns in different conversations run at once; a second turn in the same conversation still gets
the "still working" answer.

    Harness("pi", "Pi", PerConversation(lambda conversation, token: PiMind(config, token=token)))
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple, Union

# ── The wire ────────────────────────────────────────────────────────────────────────────

ATTACH = "harness.attach"
POLL = "harness.poll"
CHUNK = "harness.chunk"
COMPLETE = "harness.complete"
FAIL = "harness.fail"
DETACH = "harness.detach"
EVENT = "harness.event"

# The conversation every turn is in when a harness holds only one, or the desktop is older than
# conversations.
MAIN = "main"
# The desktop refuses an event heavier than this, as JSON (crates/yantrik-harness protocol.rs).
# A long output is sent as several `tool_output` events, each well under it.
MAX_EVENT_BYTES = 64 * 1024
EVENT_PIECE_BYTES = 48 * 1024
# The environment variable the tools a harness starts for a conversation read their agent from.
AGENT_TOKEN_ENV = "YANTRIK_AGENT_TOKEN"

# How long to wait after an empty poll. The protocol's own pacing; the host does not hold a poll
# open, so this is the client's wait and nothing else (crates/yantrik-harness/src/protocol.rs).
POLL_INTERVAL = 0.2
# The desktop drops a harness that has not called anything for 90 seconds. A turn being worked on
# says so this often, with a chunk whose delta is empty — presence, not text.
HEARTBEAT_SECONDS = 20.0
# How long to wait before looking for the desktop again after it went away.
RETRY_SECONDS = 5.0
# What the person is told when they type while the mind is mid-answer. A message that arrives
# during a turn is a turn too, and it is owed an answer — queueing it is how a chat app behaves
# and here it leaves the desktop waiting on something nobody will ever close.
BUSY_REPLY = "still working on the previous request — ask again in a moment, or say /stop"


class HarnessError(Exception):
    """The desktop refused a call, or could not be reached."""


def socket_path() -> Optional[str]:
    """Where the desktop's harness socket is, if there is one.

    The same places in the same order as the OS binds them, because the shell takes the first
    directory it can write and a client has to find the one it actually chose.
    """
    explicit = os.environ.get("YANTRIK_HARNESS_SOCKET", "").strip()
    if explicit:
        # Named outright: honour it and look nowhere else, so pointing a harness at one desktop
        # can never silently fall through to another.
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
        raise HarnessError("%s: %s" % (method, exc)) from exc
    if not buf.strip():
        raise HarnessError("%s: the desktop closed the connection without answering" % method)
    try:
        reply = json.loads(buf)
    except ValueError as exc:
        raise HarnessError("%s: unreadable reply %r" % (method, buf[:200])) from exc
    if isinstance(reply, dict) and reply.get("error"):
        err = reply["error"]
        raise HarnessError(err.get("message", str(err)) if isinstance(err, dict) else str(err))
    return reply.get("result") if isinstance(reply, dict) else None


# ── The tool trail ──────────────────────────────────────────────────────────────────────


def tool_trail(name: str, arguments: Optional[Dict[str, Any]] = None) -> str:
    """The one line a tool call contributes to the conversation.

    Every harness shows tool use the same way the panel already shows it for Hermes, because the
    person reading it should not have to learn a second vocabulary when they switch minds.

    `app` and `action` name what was touched, and they go in the label: `os_act` alone says
    nothing, `os_act calendar.add_event` says what happened. The rest of the arguments follow as
    one JSON object on the same line, so the panel can show `studio.generate prompt="a red kite"`
    under the call the way the approval card for the same call already does (#125). This line
    used to leave them out on the grounds that a note's body or a message's text is private —
    but the person reading the panel is the person whose desktop this is, the card two lines up
    shows every argument, and a mind whose calls cannot be read is a mind that cannot be watched.
    The shell keeps the line to one line and opens the arguments in full on a click, so a long
    body does not take over the conversation.
    """
    label = str(name or "tool")
    args = arguments if isinstance(arguments, dict) else {}
    app = str(args.get("app") or "").strip()
    action = str(args.get("action") or "").strip()
    # os_describe lists actions as `new_note()`; a model copying that sends the punctuation too.
    action = action.split("(", 1)[0].strip()
    if app and action:
        label = "%s %s.%s" % (label, app, action)
    elif app:
        label = "%s %s" % (label, app)
    rest = {k: v for k, v in args.items() if k not in ("app", "action")}
    if rest:
        # One line, always: json.dumps escapes newlines, and `default=str` keeps a value the
        # model sent as something odd from taking the whole trail down with it.
        label = "%s %s" % (label, json.dumps(rest, ensure_ascii=False, default=str,
                                              separators=(",", ":")))
    return "⚙️ %s" % label


# ── What the agent is doing ─────────────────────────────────────────────────────────────


def tool_target(name: str, arguments: Optional[Dict[str, Any]] = None) -> str:
    """What a tool call touched, when that can be said: `calendar.add_event`, `notes`, a path.

    The card's heading, beside the tool's name. Empty when nothing in the arguments names a
    thing — an empty target is honest, a guessed one is not.
    """
    args = arguments if isinstance(arguments, dict) else {}
    app = str(args.get("app") or "").strip()
    action = str(args.get("action") or "").strip().split("(", 1)[0].strip()
    if app and action:
        return "%s.%s" % (app, action)
    if app:
        return app
    for key in ("path", "file_path", "file", "dir", "directory", "url", "command"):
        value = args.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip().splitlines()[0][:200]
    return ""


def summary_line(text: str, limit: int = 160) -> str:
    """The first line worth reading, for a card that has settled: "38 files moved"."""
    for line in str(text or "").splitlines():
        line = line.strip()
        if line:
            return line if len(line) <= limit else line[:limit - 1] + "…"
    return ""


def _pieces(text: str, budget: int = EVENT_PIECE_BYTES) -> List[str]:
    """Cut text into pieces that each fit in one event, as the wire encodes them.

    The wire is `json.dumps` with ASCII escaping, so one character can cost twelve bytes (an
    emoji is a surrogate pair of `\\uXXXX`). Measured, not assumed.
    """
    out: List[str] = []
    rest = str(text or "")
    while rest:
        size = min(len(rest), budget)
        while size > 1 and len(json.dumps(rest[:size])) > budget:
            size //= 2
        out.append(rest[:size])
        rest = rest[size:]
    return out


# ── One turn ────────────────────────────────────────────────────────────────────────────


class Turn:
    """One thing the person typed, and the answer being streamed back to it."""

    def __init__(self, harness: "Harness", session: str, turn_id: int, text: str,
                 context: Optional[str] = None, conversation: str = MAIN,
                 agent_token: str = "") -> None:
        self.harness = harness
        self.session = session
        self.turn_id = turn_id
        self.text = text
        self.context = context
        # Which conversation this turn is in — an id the desktop issued, or `main` — and that
        # agent's token. The token goes to the tools the harness starts for this conversation
        # (AGENT_TOKEN_ENV), never to the model and never into a log.
        self.conversation = conversation or MAIN
        self.agent_token = agent_token or ""
        # Set when the person said /stop, or /new arrived while this was running. A handler that
        # watches it can stop between steps; one that does not is simply left to finish.
        self.cancelled = threading.Event()
        # The panel stopped listening (the desktop answered a chunk with {"dropped": true}).
        self.dropped = False
        self.closed = False
        self.said_anything = False
        # When this turn last said anything to the desktop, so the heartbeat only fires for a
        # turn that has actually gone quiet.
        self.last_call = time.monotonic()
        self._tail = ""          # the last delta, so the trail knows whether a newline is owed
        self._lock = threading.Lock()

    @property
    def notes(self) -> List[str]:
        """What the desktop has to tell this agent since its last turn, if anything.

        The `notes` of the turn's `context` (crates/yantrik-harness protocol.rs,
        `Assignment::context`): a command that finished after the call that started it had
        returned, with its exit code and last lines. Each is a sentence meant for the model and
        arrives once. A harness that shows its model nothing else of the context should show it
        these — `notes_before` puts them in front of the person's message.
        """
        if not self.context:
            return []
        try:
            context = json.loads(self.context)
        except ValueError:
            return []
        notes = context.get("notes") if isinstance(context, dict) else None
        if not isinstance(notes, list):
            return []
        return [str(note).strip() for note in notes if str(note).strip()]

    def notes_before(self, text: str) -> str:
        """`text` with this turn's notes in front of it, or `text` alone when there are none."""
        notes = self.notes
        if not notes:
            return text
        said = "\n".join("- " + note.replace("\n", "\n  ") for note in notes)
        return "[From the desktop, since your last turn:\n%s]\n\n%s" % (said, text)

    # The two calls a handler makes.

    def emit(self, delta: str) -> bool:
        """Stream a piece of the answer. False means the panel is no longer listening."""
        if not delta:
            # An empty delta is the heartbeat's meaning, not text. A handler that produced no
            # characters should not accidentally send one.
            return not self.dropped
        with self._lock:
            self.said_anything = True
            self._tail = delta
        return self.harness._chunk(self, delta)

    def tool(self, name: str, arguments: Optional[Dict[str, Any]] = None) -> bool:
        """Note a tool call in the conversation, framed the way the panel expects."""
        with self._lock:
            lead = "" if (not self.said_anything or self._tail.endswith("\n")) else "\n"
        return self.emit(lead + tool_trail(name, arguments) + "\n\n")

    # What the agent is doing, beside the text. Each is one `harness.event`; each returns False
    # only when the panel is no longer listening, like `emit`. A desktop that refuses one says
    # why in the log, and the turn goes on.

    def tool_start(self, call: str, name: str, target: str = "",
                   args: Optional[Any] = None, trail: bool = True) -> bool:
        """A tool call began: the pane opens a card for it, running.

        `call` is the harness's own id for the call, unique within the turn (pi's toolCallId, an
        OpenAI tool_call id); the output and the end name it again. Also writes the same trail
        line `tool` does, unless `trail` is False, so a panel that draws no cards — and every
        reader of the text — still sees the call.
        """
        name = str(name or "tool")
        if trail:
            self.tool(name, args if isinstance(args, dict) else None)
        event: Dict[str, Any] = {
            "kind": "tool_start", "call": str(call), "name": name,
            "target": str(target if target else tool_target(name, args if isinstance(args, dict) else None)),
            "args": args if args is not None else {},
        }
        if len(json.dumps(event)) > EVENT_PIECE_BYTES:
            # Arguments that big are a file's contents or a page of text. The card shows how it
            # began and says it was cut, rather than the desktop refusing the whole call.
            whole = json.dumps(event["args"])
            event["args"] = {"truncated": True, "bytes": len(whole), "preview": whole[:8000]}
        return self._event(event)

    def tool_output(self, call: str, delta: str, stream: str = "stdout") -> bool:
        """Output from a running call, as it arrives. Long output is sent in several events."""
        listening = not self.dropped
        for piece in _pieces(delta):
            listening = self._event({"kind": "tool_output", "call": str(call),
                                     "stream": stream, "delta": piece})
            if not listening:
                break
        return listening

    def tool_end(self, call: str, ok: bool, summary: str = "",
                 exit_code: Optional[int] = None) -> bool:
        """A call ended: the card settles, ✓ or ✗, with one line on how it went."""
        event: Dict[str, Any] = {"kind": "tool_end", "call": str(call), "ok": bool(ok),
                                 "summary": summary_line(summary, 400) if summary else ""}
        if exit_code is not None:
            event["exit_code"] = int(exit_code)
        return self._event(event)

    def thinking(self, delta: str) -> bool:
        """The mind's reasoning, when it shares it. Shown folded, never as the answer."""
        listening = not self.dropped
        for piece in _pieces(delta):
            listening = self._event({"kind": "thinking", "delta": piece})
            if not listening:
                break
        return listening

    def status(self, text: str) -> bool:
        """What the agent is doing or waiting on, in a few words: "waiting for your approval"."""
        return self._event({"kind": "status", "text": summary_line(text, 200)})

    def usage(self, model: str = "", input_tokens: Optional[int] = None,
              output_tokens: Optional[int] = None, cost_usd: Optional[float] = None) -> bool:
        """What a model call cost. Send what you have; usage events add up over a turn."""
        event: Dict[str, Any] = {"kind": "usage", "model": str(model or "")}
        if input_tokens is not None:
            event["input_tokens"] = max(0, int(input_tokens))
        if output_tokens is not None:
            event["output_tokens"] = max(0, int(output_tokens))
        if cost_usd is not None:
            event["cost_usd"] = float(cost_usd)
        return self._event(event)

    def _event(self, event: Dict[str, Any]) -> bool:
        if self.closed or self.dropped:
            return not self.dropped
        send = getattr(self.harness, "_event", None)
        if send is None:
            # Something standing in for a harness that only takes text: the events are extra.
            return True
        return send(self, event)


class Handler:
    """What a harness needs from the mind behind it.

    Subclass or duck-type. Only `answer` is required.
    """

    #: Two turns at once, or one at a time? A mind with a single conversation and a single
    #: subprocess says False and gets the "still working" answer for free. With conversations,
    #: this is per conversation: turns in different conversations always run at once.
    concurrent = False

    #: Can this hold more than one conversation? Said to the desktop when attaching;
    #: `turn.conversation` then says which one each turn is in. `PerConversation` sets it.
    conversations = False

    def answer(self, turn: Turn) -> None:
        raise NotImplementedError

    def reset(self) -> None:
        """/new — forget the conversation so far."""

    def reset_conversation(self, conversation: str) -> None:
        """/new in one conversation. A handler that holds one conversation just resets."""
        self.reset()

    def cancel(self, turn: Turn) -> None:
        """/stop — on top of `turn.cancelled` being set, for a mind that needs telling."""

    def end(self, conversation: str) -> None:
        """The desktop ended this conversation — the person stopped the agent, or the desktop
        restarted — and it will never be asked anything again. Let go of what it holds. Called on
        the poll loop's thread, so it must not block: close a process on a thread of its own."""


class PerConversation(Handler):
    """One handler per conversation, made when the conversation's first turn arrives.

    The simplest way to hold many conversations: a mind that already knows how to hold one — a
    process, a history — is made again for each, with that agent's token, and closed (its
    `close()`, if it has one) when the desktop ends the conversation. `make(conversation, token)`
    returns the handler.

    A turn that arrives with a different token than the conversation was made with means the
    desktop has started that conversation over (its `main` after the agent was stopped, or after
    the desktop restarted); the old handler is closed and a new one made, so a process started
    under an old token never answers for a new agent.
    """

    conversations = True

    def __init__(self, make: Callable[[str, str], Handler], limit: int = 8,
                 log: Optional[Callable[[str], None]] = None) -> None:
        self.make = make
        # The desktop caps live agents itself (six); this is the harness's own backstop, so a
        # desktop that does not cannot start processes without end.
        self.limit = limit
        self.log = log or (lambda message: print("[conversations] %s" % message, file=sys.stderr))
        self._lock = threading.Lock()
        self._held: Dict[str, Tuple[Handler, str]] = {}

    def held(self) -> List[str]:
        """The conversations with a handler right now."""
        with self._lock:
            return sorted(self._held)

    def mind(self, conversation: str) -> Optional[Handler]:
        with self._lock:
            entry = self._held.get(conversation)
        return entry[0] if entry else None

    def _for(self, turn: Turn) -> Handler:
        stale: Optional[Handler] = None
        with self._lock:
            entry = self._held.get(turn.conversation)
            if entry is not None and turn.agent_token and entry[1] != turn.agent_token:
                stale = entry[0]
                del self._held[turn.conversation]
                entry = None
            if entry is None:
                if len(self._held) >= self.limit:
                    raise RuntimeError(
                        "this harness is already holding %d conversations, the most it keeps at "
                        "once; stop one of its agents to start another" % self.limit)
                mind = self.make(turn.conversation, turn.agent_token)
                self._held[turn.conversation] = (mind, turn.agent_token)
            else:
                mind = entry[0]
        if stale is not None:
            self._retire(stale)
        return mind

    def answer(self, turn: Turn) -> None:
        self._for(turn).answer(turn)

    def cancel(self, turn: Turn) -> None:
        mind = self.mind(turn.conversation)
        if mind is not None:
            mind.cancel(turn)

    def reset(self) -> None:
        with self._lock:
            minds = [entry[0] for entry in self._held.values()]
        for mind in minds:
            mind.reset()

    def reset_conversation(self, conversation: str) -> None:
        mind = self.mind(conversation)
        if mind is not None:
            mind.reset()

    def end(self, conversation: str) -> None:
        with self._lock:
            entry = self._held.pop(conversation, None)
        if entry is not None:
            self._retire(entry[0])

    def close(self) -> None:
        """Close every conversation's handler, and wait for them: the harness is exiting."""
        with self._lock:
            minds = [entry[0] for entry in self._held.values()]
            self._held.clear()
        for mind in minds:
            self._close(mind)

    def _retire(self, mind: Handler) -> None:
        threading.Thread(target=self._close, args=(mind,), name="conversation-close",
                         daemon=True).start()

    def _close(self, mind: Handler) -> None:
        close = getattr(mind, "close", None)
        if close is None:
            return
        try:
            close()
        except Exception as exc:  # noqa: BLE001 — one conversation's close must not stop the rest
            self.log("closing a conversation raised: %s" % exc)


# ── The harness ─────────────────────────────────────────────────────────────────────────


class Harness:
    """Attach, poll, hand each turn to the handler, and close every turn exactly once."""

    def __init__(self, id: str, name: str, handler: Handler, detail: Optional[str] = None,
                 tools: bool = False, memory: bool = False, address: Optional[str] = None,
                 log: Optional[Callable[[str], None]] = None,
                 heartbeat_seconds: float = HEARTBEAT_SECONDS,
                 poll_interval: float = POLL_INTERVAL,
                 retry_seconds: float = RETRY_SECONDS,
                 busy_reply: str = BUSY_REPLY) -> None:
        self.id = id
        self.name = name
        self.handler = handler
        self.detail = detail
        self.tools = tools
        self.memory = memory
        # None means "find it each time we attach", so a harness started before the desktop
        # picks it up when it appears.
        self.address = address
        self.log = log or (lambda message: print("[%s] %s" % (id, message), file=sys.stderr))
        self.heartbeat_seconds = heartbeat_seconds
        self.poll_interval = poll_interval
        self.retry_seconds = retry_seconds
        self.busy_reply = busy_reply

        self.session: Optional[str] = None
        self._resolved: Optional[str] = address
        self._open: Dict[int, Turn] = {}
        self._lock = threading.Lock()
        self._stopping = threading.Event()
        self._workers: List[threading.Thread] = []
        self._beat: Optional[threading.Thread] = None
        self._complained_about_socket = False
        # Whether this desktop takes `harness.event`. An older one answers "unknown method", and
        # from then on the events are skipped for the session: the trail lines are in the text.
        self._events_ok = True
        # Conversations turns have arrived in this session, so a new session can end them: the
        # desktop that issued them is gone, and so are their agents and tokens.
        self._seen: set = set()

    # ── running ─────────────────────────────────────────────────────────

    def run(self) -> None:
        """Poll forever. Returns when `stop()` is called."""
        self._beat = threading.Thread(target=self._heartbeat, name="harness-heartbeat", daemon=True)
        self._beat.start()
        try:
            while not self._stopping.is_set():
                if self.session is None:
                    if not self._attach():
                        self._stopping.wait(self.retry_seconds)
                    continue
                try:
                    reply = self._call(POLL, {"session": self.session}) or {}
                except HarnessError as exc:
                    # The shell restarted, or this session aged out. Attaching again is the whole
                    # recovery — and the new session id MUST replace the old one, or the loop
                    # recovers forever and never succeeds.
                    self.log("poll failed (%s); re-attaching" % exc)
                    self.session = None
                    self._stopping.wait(min(self.retry_seconds, 2.0))
                    continue
                self._notices(reply)
                turn_id = reply.get("turn_id")
                if not isinstance(turn_id, int):
                    self._stopping.wait(self.poll_interval)
                    continue
                conversation = str(reply.get("conversation") or MAIN)
                self._seen.add(conversation)
                self._dispatch(Turn(self, self.session, turn_id,
                                    str(reply.get("text") or ""), reply.get("context"),
                                    conversation=conversation,
                                    agent_token=str(reply.get("agent_token") or "")))
        finally:
            self._shutdown()

    def _notices(self, reply: Dict[str, Any]) -> None:
        """What the desktop stopped waiting for: turns it cancelled, conversations it ended."""
        for turn_id in reply.get("cancelled") or []:
            with self._lock:
                turn = self._open.get(turn_id)
            if turn is not None:
                # The same as the panel going away: nothing more is sent, and the mind is asked
                # to stop. The turn is still closed once, by whoever is running it.
                self._drop(turn)
        for conversation in reply.get("ended") or []:
            self._end(str(conversation))

    def _end(self, conversation: str) -> None:
        with self._lock:
            running = [t for t in self._open.values() if t.conversation == conversation]
        for turn in running:
            self._drop(turn)
        self._seen.discard(conversation)
        end = getattr(self.handler, "end", None)
        if end is None:
            return
        try:
            end(conversation)
        except Exception as exc:  # noqa: BLE001 — a handler's end must not kill the loop
            self.log("ending a conversation raised: %s" % exc)

    def stop(self) -> None:
        self._stopping.set()

    def _shutdown(self) -> None:
        for turn in list(self._open.values()):
            turn.cancelled.set()
            self._close(turn, error="the harness is shutting down")
        if self.session:
            try:
                self._call(DETACH, {"session": self.session})
            except HarnessError:
                pass
            self.session = None

    def _attach(self) -> bool:
        address = self.address or socket_path()
        if not address:
            if not self._complained_about_socket:
                self.log("no desktop harness socket yet; waiting for one")
                self._complained_about_socket = True
            return False
        params: Dict[str, Any] = {"id": self.id, "name": self.name,
                                  "tools": self.tools, "memory": self.memory,
                                  "conversations": bool(getattr(self.handler, "conversations", False))}
        if self.detail:
            params["detail"] = self.detail
        try:
            reply = call(address, ATTACH, params) or {}
        except HarnessError as exc:
            self.log("could not attach: %s" % exc)
            return False
        session = reply.get("session")
        if not session:
            self.log("the desktop attached us without a session id, which cannot be used")
            return False
        self.session = str(session)
        self._resolved = address
        self._complained_about_socket = False
        self._events_ok = True
        self.log("attached as `%s` (session %s)" % (self.id, self.session))
        # Whatever the last session's conversations held — a process each, a history each — was
        # for agents that desktop issued. Its tokens name nothing now.
        for conversation in sorted(self._seen):
            self._end(conversation)
        self._seen = set()
        return True

    def _call(self, method: str, params: Dict[str, Any]) -> Any:
        address = self.address or getattr(self, "_resolved", None) or socket_path()
        if not address:
            raise HarnessError("%s: the desktop's harness socket is gone" % method)
        return call(address, method, params)

    # ── turns ───────────────────────────────────────────────────────────

    def _dispatch(self, turn: Turn) -> None:
        command = turn.text.strip().split(None, 1)[0].lower() if turn.text.strip() else ""
        if command in ("/stop", "/new"):
            self._command(turn, command)
            return

        with self._lock:
            # Per conversation: a turn in one conversation never waits for another. The desktop
            # already hands a conversation one turn at a time; this is for one that does not.
            busy = (not getattr(self.handler, "concurrent", False)
                    and any(t.conversation == turn.conversation for t in self._open.values()))
            if not busy:
                self._open[turn.turn_id] = turn
        if busy:
            # Answered immediately and closed here: this turn is owed an answer exactly like the
            # one being worked on, and the worst thing to do with it is nothing.
            turn.emit(self.busy_reply)
            self._close(turn)
            return

        worker = threading.Thread(target=self._work, args=(turn,),
                                  name="turn-%d" % turn.turn_id, daemon=True)
        with self._lock:
            self._workers = [t for t in self._workers if t.is_alive()]
            self._workers.append(worker)
        worker.start()

    def _command(self, turn: Turn, command: str) -> None:
        """/stop and /new are answered here, never handed to the mind — in their conversation."""
        with self._lock:
            running = [t for t in self._open.values() if t.conversation == turn.conversation]
        for other in running:
            other.cancelled.set()
            try:
                self.handler.cancel(other)
            except Exception as exc:  # a mind that cannot be stopped must not break /stop
                self.log("cancel raised: %s" % exc)
        if command == "/stop":
            turn.emit("stopping." if running else "nothing was running.")
        else:
            try:
                reset = getattr(self.handler, "reset_conversation", None)
                if reset is not None:
                    reset(turn.conversation)
                else:
                    self.handler.reset()
            except Exception as exc:
                turn.emit("could not start a new conversation: %s" % _readable(exc))
                self._close(turn)
                return
            turn.emit("new conversation — what came before is forgotten." if not running
                      else "stopped, and started a new conversation.")
        self._close(turn)

    def _work(self, turn: Turn) -> None:
        try:
            self.handler.answer(turn)
        except Exception as exc:
            # The mind failed. The person gets a sentence instead of an answer, and the turn is
            # closed — a failure that leaves a turn open is worse than the failure.
            self.log("turn %d failed: %r" % (turn.turn_id, exc))
            if turn.said_anything:
                turn.emit("\n\n" + _readable(exc))
                self._close(turn)
            else:
                self._close(turn, error=_readable(exc))
            return
        if not turn.said_anything:
            # A handler that returns without saying anything leaves the panel with an empty
            # bubble and no way to tell it from a hang. Say something.
            turn.emit("(stopped)" if turn.cancelled.is_set() else "(no answer)")
        self._close(turn)

    def _chunk(self, turn: Turn, delta: str) -> bool:
        if turn.closed or self._stopping.is_set():
            return False
        try:
            reply = self._call(CHUNK, {"session": turn.session, "turn_id": turn.turn_id,
                                       "delta": delta}) or {}
        except HarnessError as exc:
            # The desktop no longer holds this turn — it was failed for us (a re-attach, a
            # restart, a person who moved on). A turn that is dropped is over: the mind behind
            # it is told to stop, and this is logged once, not on every delta. Pi kept
            # answering a weather question for three minutes into a turn the desktop had
            # dropped at the first second, one log line per token.
            if not turn.dropped:
                self.log("chunk on turn %d failed: %s" % (turn.turn_id, exc))
            self._drop(turn)
            return False
        turn.last_call = time.monotonic()
        if reply.get("dropped"):
            self._drop(turn)
            return False
        return True

    def _event(self, turn: Turn, event: Dict[str, Any]) -> bool:
        """One `harness.event`. False only when the panel is no longer listening."""
        if turn.closed or turn.dropped or self._stopping.is_set():
            return not turn.dropped
        if not self._events_ok:
            return True
        try:
            reply = self._call(EVENT, {"session": turn.session, "turn_id": turn.turn_id,
                                       "event": event}) or {}
        except HarnessError as exc:
            if "unknown method" in str(exc):
                # A desktop from before events. Nothing is lost that it could have shown: the
                # trail line is in the text, and the text is all it draws.
                self._events_ok = False
                self.log("this desktop does not take harness.event; tool calls show as trail "
                         "lines only")
                return True
            if not turn.dropped:
                self.log("event on turn %d failed: %s" % (turn.turn_id, exc))
            self._drop(turn)
            return False
        turn.last_call = time.monotonic()
        if reply.get("dropped"):
            self._drop(turn)
            return False
        if reply.get("refused"):
            # The desktop kept the turn and refused this one event. Worth a log line — it is a
            # bug here or in the mind — and not worth the turn.
            self.log("the desktop refused a %s event on turn %d: %s"
                     % (event.get("kind"), turn.turn_id, reply["refused"]))
        return True

    def _drop(self, turn: Turn) -> None:
        """The panel stopped listening: nothing more is sent, and the mind is asked to stop."""
        if turn.dropped:
            return
        turn.dropped = True
        turn.cancelled.set()
        try:
            self.handler.cancel(turn)
        except Exception as exc:  # noqa: BLE001 — a handler's cancel must not kill the loop
            self.log("cancel raised: %s" % exc)

    def _close(self, turn: Turn, error: Optional[str] = None) -> None:
        """Complete or fail, once. Every path out of a turn comes through here."""
        with self._lock:
            if turn.closed:
                return
            turn.closed = True
            self._open.pop(turn.turn_id, None)
        try:
            if error is None:
                self._call(COMPLETE, {"session": turn.session, "turn_id": turn.turn_id})
            else:
                self._call(FAIL, {"session": turn.session, "turn_id": turn.turn_id, "error": error})
        except HarnessError as exc:
            # The desktop has already given up on this turn (it restarted, or we re-attached and
            # it failed what the old session owed). Nothing left to close.
            self.log("could not close turn %d: %s" % (turn.turn_id, exc))

    def _heartbeat(self) -> None:
        """An empty chunk on every open turn, well inside the desktop's 90-second window.

        A long turn can go minutes between anything worth showing — a model thinking, an `os_act`
        waiting on an approval card — and without this the desktop reaps the harness mid-answer
        and the person is told it stopped responding while it is working.
        """
        while not self._stopping.is_set():
            self._stopping.wait(max(0.05, min(self.heartbeat_seconds / 2.0, 2.0)))
            if self._stopping.is_set():
                return
            now = time.monotonic()
            for turn in list(self._open.values()):
                if turn.closed or now - turn.last_call < self.heartbeat_seconds:
                    continue
                turn.last_call = now
                try:
                    self._call(CHUNK, {"session": turn.session, "turn_id": turn.turn_id, "delta": ""})
                except HarnessError:
                    pass


def _readable(exc: BaseException) -> str:
    """A failure as a sentence a person can act on, never a traceback."""
    detail = str(exc).strip() or exc.__class__.__name__
    if not detail.endswith((".", "!", "?")):
        detail += "."
    return detail


# ── The desktop's tools, over MCP ───────────────────────────────────────────────────────

# How long one tool call may take. An `os_act` above the session's ceiling asks the person and
# waits for them inside the bridge — up to OS_ACT_MAX_SECONDS, which is a little over 270s (see
# deploy/yantrik-os/yos-mcp). A client that gives up sooner cuts the person off mid-decision and
# reports a timeout for a machine that was working correctly.
MCP_TIMEOUT = 300.0
MCP_COMMAND = os.environ.get("YOS_MCP_BIN", "/opt/yantrik/bin/yos-mcp")
MCP_PROTOCOL_VERSION = "2024-11-05"
# The bridge's command tools, offered to a conversation's bridge when it carries the agent's
# token, wait for the command itself as well: `wait_seconds`, 120 by default and at most 600, on
# top of everything an os_act can wait for. A client that allowed only MCP_TIMEOUT would cut off
# the command it asked to wait for. `hand_off` waits for a catalog role's answer the same way,
# and not at all when it is not told to.
MCP_WAITING_TOOLS = {"run_command": 120.0, "command_status": 120.0, "hand_off": 0.0}
MCP_WAIT_MOST = 600.0


def mcp_timeout(name: str, arguments: Optional[Dict[str, Any]] = None,
                base: float = MCP_TIMEOUT) -> float:
    """How long to give one tool call: `base`, and as long again as the command it waits for."""
    if name not in MCP_WAITING_TOOLS:
        return base
    wait = (arguments or {}).get("wait_seconds", MCP_WAITING_TOOLS[name])
    try:
        wait = float(wait)
    except (TypeError, ValueError):
        wait = MCP_WAITING_TOOLS[name]
    return base + min(max(wait, 0.0), MCP_WAIT_MOST)


class McpTools:
    """The desktop's own tools, as an MCP client over stdio.

    One child process, one reader thread, thread-safe, and restarted if it dies — a harness that
    loses the bridge should lose one tool call, not the desktop.

    `clientInfo` is the harness's own name and version because that is what the approval card
    shows the person: the bridge keeps it for the card's `says the caller` line, and a card that
    reads "an unnamed caller is asking to use this machine" is one the person cannot answer.
    """

    def __init__(self, client_name: str, client_version: str = "1.0.0",
                 command: Optional[Union[str, Sequence[str]]] = None,
                 env: Optional[Dict[str, str]] = None,
                 timeout: float = MCP_TIMEOUT,
                 log: Optional[Callable[[str], None]] = None) -> None:
        self.client_name = client_name
        self.client_version = client_version
        self.command: Sequence[str] = ([command] if isinstance(command, str)
                                       else list(command) if command else [MCP_COMMAND])
        self.env = env
        self.timeout = timeout
        self.log = log or (lambda message: print("[mcp] %s" % message, file=sys.stderr))

        self._proc: Optional[subprocess.Popen] = None
        self._proc_lock = threading.RLock()
        self._write_lock = threading.Lock()
        self._pending: Dict[int, Dict[str, Any]] = {}
        self._pending_lock = threading.Lock()
        self._next_id = 0
        self._tools: Optional[List[Dict[str, Any]]] = None

    # ── public ──────────────────────────────────────────────────────────

    def list(self, refresh: bool = False) -> List[Dict[str, Any]]:
        """Every tool the desktop publishes, with its real description and schema."""
        if self._tools is not None and not refresh:
            return self._tools
        self._ensure()
        result = self._rpc("tools/list", {}, timeout=30.0)
        tools = [t for t in (result.get("tools") or []) if isinstance(t, dict) and t.get("name")]
        self._tools = tools
        return tools

    def call(self, name: str, arguments: Optional[Dict[str, Any]] = None,
             timeout: Optional[float] = None) -> Tuple[str, bool]:
        """Run one tool. Returns (text, is_error) — never raises.

        `is_error` is the bridge's own flag and means "this did not run". A policy answer — the
        person said no, the mode forbids it, the session is tainted — comes back unflagged and
        says REFUSED in its first word, because it is an answer and not a fault. A mind must not
        retry it or route around it, and a harness must not turn it into an error either.
        """
        try:
            self._ensure()
            result = self._rpc("tools/call", {"name": name, "arguments": arguments or {}},
                               timeout=(mcp_timeout(name, arguments, self.timeout)
                                        if timeout is None else timeout))
        except McpError as exc:
            return (str(exc), True)
        parts = result.get("content") or []
        text = "".join(str(p.get("text") or "") for p in parts
                       if isinstance(p, dict) and p.get("type") == "text")
        return (text or "(the tool returned nothing)", bool(result.get("isError")))

    def as_openai_tools(self) -> List[Dict[str, Any]]:
        """The same tools in the shape an OpenAI-compatible chat API wants."""
        out = []
        for tool in self.list():
            out.append({
                "type": "function",
                "function": {
                    "name": tool["name"],
                    "description": str(tool.get("description") or "")[:4000],
                    "parameters": tool.get("inputSchema") or {"type": "object", "properties": {}},
                },
            })
        return out

    def close(self) -> None:
        with self._proc_lock:
            proc, self._proc = self._proc, None
        end_process(proc)

    # ── plumbing ────────────────────────────────────────────────────────

    def _ensure(self) -> None:
        with self._proc_lock:
            if self._proc is not None and self._proc.poll() is None:
                return
            if self._proc is not None:
                self.log("the desktop bridge exited (%s); restarting it" % self._proc.poll())
                end_process(self._proc)
            self._start()

    def _start(self) -> None:
        env = dict(os.environ)
        if self.env:
            env.update(self.env)
        try:
            self._proc = subprocess.Popen(
                list(self.command), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env,
            )
        except OSError as exc:
            self._proc = None
            raise McpError("the desktop bridge could not be started (%s): %s"
                           % (" ".join(self.command), exc)) from exc
        threading.Thread(target=self._reader, args=(self._proc,),
                         name="mcp-reader", daemon=True).start()
        self._rpc("initialize", {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            # Self-declared, and the bridge says so on the card. It is the name the person
            # recognises, which is the whole reason to send it.
            "clientInfo": {"name": self.client_name, "version": self.client_version},
        }, timeout=30.0)
        self._notify("notifications/initialized", {})

    def _reader(self, proc: subprocess.Popen) -> None:
        try:
            for line in proc.stdout:  # type: ignore[union-attr]
                line = line.strip()
                if not line:
                    continue
                try:
                    msg = json.loads(line)
                except ValueError:
                    continue  # a stray line on stdout is not a reason to lose the bridge
                if not isinstance(msg, dict) or msg.get("id") is None:
                    continue
                with self._pending_lock:
                    slot = self._pending.get(msg["id"])
                if slot is None:
                    continue
                slot["message"] = msg
                slot["event"].set()
        except Exception:
            pass
        finally:
            try:
                if proc.stdout:
                    proc.stdout.close()
            except Exception:
                pass
            self._wake_all("the desktop bridge stopped before it answered")

    def _wake_all(self, why: str) -> None:
        with self._pending_lock:
            slots = list(self._pending.values())
        for slot in slots:
            if "message" not in slot:
                slot["error"] = why
            slot["event"].set()

    def _notify(self, method: str, params: Dict[str, Any]) -> None:
        self._write({"jsonrpc": "2.0", "method": method, "params": params})

    def _rpc(self, method: str, params: Dict[str, Any], timeout: float) -> Dict[str, Any]:
        with self._pending_lock:
            self._next_id += 1
            msg_id = self._next_id
            slot: Dict[str, Any] = {"event": threading.Event()}
            self._pending[msg_id] = slot
        try:
            self._write({"jsonrpc": "2.0", "id": msg_id, "method": method, "params": params})
            if not slot["event"].wait(timeout):
                raise McpError(
                    "%s did not answer within %ds. The desktop may be busy rather than broken; "
                    "nothing was rolled back." % (method, int(timeout)))
            if slot.get("error"):
                raise McpError(str(slot["error"]))
            msg = slot["message"]
        finally:
            with self._pending_lock:
                self._pending.pop(msg_id, None)
        if msg.get("error"):
            err = msg["error"]
            raise McpError(err.get("message", str(err)) if isinstance(err, dict) else str(err))
        result = msg.get("result")
        return result if isinstance(result, dict) else {}

    def _write(self, msg: Dict[str, Any]) -> None:
        with self._write_lock:
            proc = self._proc
            if proc is None or proc.stdin is None or proc.poll() is not None:
                raise McpError("the desktop bridge is not running")
            try:
                proc.stdin.write(json.dumps(msg) + "\n")
                proc.stdin.flush()
            except (OSError, ValueError) as exc:
                raise McpError("could not reach the desktop bridge: %s" % exc) from exc


class McpError(Exception):
    """The bridge could not be reached, or did not answer. Not a refusal — see `McpTools.call`."""


def end_process(proc: Optional[subprocess.Popen]) -> None:
    """End a child and close its pipes.

    Shared because a harness restarts its child and a leaked pipe per restart is a file
    descriptor leak in a process that is meant to run for weeks.
    """
    if proc is None:
        return
    for stream in (proc.stdin, proc.stdout, proc.stderr):
        try:
            if stream:
                stream.close()
        except Exception:
            pass
    try:
        if proc.poll() is None:
            proc.terminate()
        proc.wait(timeout=5)
    except Exception:
        try:
            proc.kill()
        except Exception:
            pass


# ── Where an agent process runs ─────────────────────────────────────────────────────────


def mind_directory(harness_id: str, conversation: str = MAIN) -> str:
    """The working directory for one conversation's agent processes, made if it is missing.

    A coding agent reads the instruction files of the directory it is started in and of every
    parent — pi reads `CLAUDE.md` and `AGENTS.md`, and other agents read the same names or
    their own. A harness runs as a user service, whose working directory is `$HOME`, so an
    agent started with no directory of its own reads whatever brief the person left in
    `~/CLAUDE.md` or `~/AGENTS.md` for their own coding work, and that brief steers the
    desktop's mind (#183: one made pi answer a readiness check with a manifesto and start a
    Blender render). So every agent process is started here instead: under the desktop's own
    data directory, where the only instructions are the ones put there on purpose. A task
    about a project may still pass the project's directory explicitly; `$HOME` is never the
    default.

    The conversation id crossed the wire, so it is reduced to a plain name: a directory
    outside this tree is never what an id may ask for.
    """
    name = "".join(ch if (ch.isalnum() or ch in "-_.") else "-"
                   for ch in str(conversation or MAIN))
    if not name.strip("."):
        name = "conversation"
    data = os.environ.get("XDG_DATA_HOME", "").strip()
    root = (Path(os.path.expanduser(data)) if data
            else Path(os.path.expanduser("~")) / ".local" / "share")
    where = root / "yantrik" / "minds" / str(harness_id) / name
    where.mkdir(parents=True, exist_ok=True)
    return str(where)
