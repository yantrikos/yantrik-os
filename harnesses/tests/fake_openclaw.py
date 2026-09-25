"""An OpenClaw that is not OpenClaw: the gateway's HTTP route, and the CLI's stdout.

Two fakes in one file because they are two ends of the same harness. Both were rewritten after
the harness was run against a live OpenClaw 2026.9.1, and both now say what that install says
rather than what the harness once assumed:

`FakeGateway` is an HTTP server in a thread serving `POST /v1/chat/completions` — the gateway's
OpenAI-compatible route — with Server-Sent Events. It is a real socket server rather than a stub
so the harness's own `http.client` request, its SSE line reader and its connection teardown are
all exercised against something that is not the code under test.

Run as a script it is the CLI instead — `python3 fake_openclaw.py <scenario> …` stands in for
`openclaw agent …`, which is exactly how the harness launches it, so the harness's own argv
building is exercised unchanged. The `text` scenario prints the *one indented JSON document* a
current build prints, because a line-by-line reader passing on a stream of JSON lines while the
real thing prints a document is precisely the bug these tests exist to keep out.

Gateway scenarios (constructor):

    text          an assistant role delta, two content deltas, then [DONE]
    tool          a tool event inside the stream (2026.9.1 does not send one over this route —
                  internal tool calls stay internal — but the decoder handles it if a build does)
    abort         says "working" and then nothing, and records that the client hung up
    drop          sends one delta and kills the TCP connection without [DONE]
    silent        200, the right headers, and then nothing at all
    snapshot      resends the whole answer each time instead of sending deltas
    error         an OpenAI error object in the stream
    off           404 with an error body, the way a gateway answers when the route is disabled
    unauthorized  401, the way a gateway answers a missing or wrong token

CLI scenarios (argv[1]):

    text        the real `--json` document — one indented object — and exit 0
    tool        the same, with the run's tool summary filled in
    plain       no JSON at all: the final text on stdout, exit 0
    lines       JSON-lines events, for a build that streams instead of printing a document
    error       a document with status "error", exit 1 (which is what a failed run does)
    inflight    a document with status "in_flight"
    empty       exit 0 having printed nothing
    silent      says nothing and waits to be killed
    fail        exits 1 having said nothing, complaining about a flag
    down        exits 1 the way a CLI does when its daemon is not running
"""

from __future__ import annotations

import json
import os
import socket
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, List, Optional


def free_port() -> int:
    """A port nothing is listening on, so a test can be refused on it and then serve on it."""
    sock = socket.socket()
    try:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])
    finally:
        sock.close()


# ── The gateway ─────────────────────────────────────────────────────────────────────────


def _chunk(**delta: Any) -> Dict[str, Any]:
    """One `chat.completion.chunk`, shaped the way the live gateway shapes it."""
    return {
        "id": "chatcmpl_fake", "object": "chat.completion.chunk", "created": 0,
        "model": "openclaw/default",
        "choices": [{"index": 0, "delta": delta, "finish_reason": None}],
    }


class _Handler(BaseHTTPRequestHandler):
    # HTTP/1.0, so the body runs to end-of-connection and neither side needs a content length or
    # chunked framing. The harness reads SSE lines until the stream ends, which is what this is.
    protocol_version = "HTTP/1.0"

    def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A003
        pass                                   # a test suite does not want an access log

    # ── plumbing ────────────────────────────────────────────────────────

    @property
    def fake(self) -> "FakeGateway":
        return self.server.fake                # type: ignore[attr-defined]

    def _say(self, event: Dict[str, Any]) -> bool:
        return self._raw("data: %s\n\n" % json.dumps(event))

    def _raw(self, text: str) -> bool:
        try:
            self.wfile.write(text.encode("utf-8"))
            self.wfile.flush()
        except OSError:
            self.fake.note_hangup()
            return False
        return True

    def _wait_for_hangup(self, probe: bool) -> None:
        """Hold the response open until the client goes away or the server stops.

        `probe` writes an SSE comment now and then, which is how the server notices the client
        has hung up. Comments carry no data, so they cannot reset the harness's silence clock —
        which is what the `silent` scenario is about.
        """
        while not self.fake.stopping.is_set():
            if probe and not self._raw(": alive\n\n"):
                return
            time.sleep(0.05)

    # ── the route ───────────────────────────────────────────────────────

    def do_POST(self) -> None:                 # noqa: N802 — BaseHTTPRequestHandler's spelling
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            body = json.loads(raw.decode("utf-8"))
        except ValueError:
            body = {}
        self.fake.record(self.path, dict(self.headers), body)

        if self.path != "/v1/chat/completions":
            self._refuse(404, "not_found", "Unknown route %s" % self.path)
            return
        scenario = self.fake.scenario
        if scenario == "off":
            # What a gateway with `gateway.http.endpoints.chatCompletions.enabled` unset says.
            self._refuse(404, "not_found", "Unknown route /v1/chat/completions")
            return
        if scenario == "unauthorized":
            self._refuse(401, "invalid_request_error", "missing or invalid gateway token")
            return

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()

        if scenario == "silent":
            self._wait_for_hangup(probe=False)
            return
        if scenario == "abort":
            if self._say(_chunk(role="assistant")) and self._say(_chunk(content="working")):
                self._wait_for_hangup(probe=True)
            return
        if scenario == "drop":
            # The stream opens and then the daemon goes away before saying anything: no content,
            # no [DONE], no close frame.
            self._say(_chunk(role="assistant"))
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self.close_connection = True
            return
        if scenario == "error":
            self._say({"error": {"message": "no model configured",
                                 "type": "invalid_request_error"}})
            self._raw("data: [DONE]\n\n")
            return
        if scenario == "snapshot":
            # No `delta` anywhere: each event is the whole answer so far.
            self._say({"type": "message", "text": "Two "})
            self._say({"type": "message", "text": "Two windows."})
            self._raw("data: [DONE]\n\n")
            return
        if scenario == "tool":
            self._say(_chunk(role="assistant"))
            self._say({"type": "tool_use", "name": "os_act",
                       "input": {"app": "calendar", "action": "add_event",
                                 "args": {"title": "dentist"}}})
            self._say(_chunk(content="Added it."))
            self._raw("data: [DONE]\n\n")
            return

        self._say(_chunk(role="assistant"))
        self._say(_chunk(content="Two "))
        self._say(_chunk(content="windows."))
        self._raw("data: [DONE]\n\n")

    def _refuse(self, status: int, kind: str, message: str) -> None:
        payload = json.dumps({"error": {"message": message, "type": kind}}).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        try:
            self.wfile.write(payload)
        except OSError:
            pass


class FakeGateway:
    """The gateway's chat-completions route, as much of it as a harness can tell apart."""

    def __init__(self, scenario: str = "text", port: int = 0) -> None:
        self.scenario = scenario
        self.lock = threading.Lock()
        self.requests: List[Dict[str, Any]] = []
        self.hangups = 0
        self.stopping = threading.Event()

        self.server = ThreadingHTTPServer(("127.0.0.1", port), _Handler)
        self.server.daemon_threads = True
        self.server.fake = self                # type: ignore[attr-defined]
        self.port = int(self.server.server_address[1])
        self.thread = threading.Thread(target=self.server.serve_forever, name="fake-gateway",
                                       kwargs={"poll_interval": 0.05}, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        """Host and port only: the harness knows the route, which is the point."""
        return "http://127.0.0.1:%d" % self.port

    def record(self, path: str, headers: Dict[str, str], body: Dict[str, Any]) -> None:
        with self.lock:
            self.requests.append({
                "path": path,
                "headers": {k.lower(): v for k, v in headers.items()},
                "body": body,
            })

    def note_hangup(self) -> None:
        with self.lock:
            self.hangups += 1

    def sent(self) -> List[Dict[str, Any]]:
        """What the harness asked for, oldest first."""
        with self.lock:
            return list(self.requests)

    def messages(self) -> List[str]:
        """The user text of each request."""
        out = []
        for request in self.sent():
            for message in request["body"].get("messages") or []:
                if isinstance(message, dict) and message.get("role") == "user":
                    out.append(str(message.get("content") or ""))
        return out

    def stop(self) -> None:
        self.stopping.set()
        try:
            self.server.shutdown()
        except Exception:
            pass
        try:
            self.server.server_close()
        except Exception:
            pass
        self.thread.join(timeout=2)


# ── The CLI ─────────────────────────────────────────────────────────────────────────────


def _document(text: str, tools: Optional[List[str]] = None, status: str = "ok",
              summary: str = "completed") -> str:
    """The envelope `openclaw agent --json` prints, indented the way it really is."""
    return json.dumps({
        "runId": "3f2c0b1e-fake",
        "status": status,
        "summary": summary,
        "result": {
            "payloads": [{"text": text, "mediaUrl": None}] if text else [],
            "meta": {
                "durationMs": 1200,
                "aborted": False,
                "stopReason": "stop",
                "finalAssistantVisibleText": text,
                "toolSummary": ({"calls": len(tools), "tools": list(tools), "failures": 0}
                                if tools else {"calls": 0, "tools": [], "failures": 0}),
            },
        },
    }, indent=2)


def _cli() -> int:
    scenario = os.environ.get("FAKE_OPENCLAW_SCENARIO") or (sys.argv[1] if len(sys.argv) > 1
                                                            else "text")
    dump = os.environ.get("FAKE_OPENCLAW_ARGV_DUMP")
    if dump:
        with open(dump, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(sys.argv[1:]) + "\n")
    # The directory the harness started this run in, one line per run (#183).
    cwd_dump = os.environ.get("FAKE_OPENCLAW_CWD_DUMP")
    if cwd_dump:
        with open(cwd_dump, "a", encoding="utf-8") as handle:
            handle.write(os.getcwd() + "\n")

    if scenario == "down":
        sys.stderr.write("Error: connect ECONNREFUSED 127.0.0.1:18789\n")
        return 1
    if scenario == "fail":
        sys.stderr.write("error: unknown option '--json'\n")
        return 1
    if scenario == "empty":
        return 0
    if scenario == "plain":
        sys.stdout.write("Two windows.\nNotes and Files.\n")
        sys.stdout.flush()
        return 0
    if scenario == "lines":
        for event in ({"type": "assistant", "delta": "Two "},
                      {"type": "assistant", "delta": "windows."},
                      {"type": "done"}):
            sys.stdout.write(json.dumps(event) + "\n")
        sys.stdout.flush()
        return 0
    if scenario == "silent":
        sys.stdout.flush()
        while True:
            time.sleep(0.2)
    if scenario == "error":
        sys.stdout.write(_document("", status="error", summary="model unavailable"))
        sys.stdout.flush()
        return 1            # a failed run prints its document and then exits non-zero
    if scenario == "inflight":
        sys.stdout.write(_document("", status="in_flight", summary="already running"))
        sys.stdout.flush()
        return 1
    if scenario == "tool":
        sys.stdout.write(_document("Added it.", tools=["yantrik-os__os_act"]))
        sys.stdout.flush()
        return 0

    sys.stdout.write(_document("Two windows."))
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    if "--version" in sys.argv:
        print("OpenClaw 2026.9.1 (fake)")
        sys.exit(0)
    sys.exit(_cli())
