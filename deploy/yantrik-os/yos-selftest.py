#!/usr/bin/env python3
"""Self-test for `yos`: the real script, against fake sockets.

    python3 deploy/yantrik-os/yos-selftest.py

`yos` is a JSON-RPC client over unix sockets and nothing else, so the desktop can be faked by
being one: this puts a socket called `app-shell.sock` in a scratch runtime directory, answers
`app.act` from it, and imports the real `yos` with `XDG_RUNTIME_DIR` pointing at that directory.
Nothing here touches a real machine, a window or the shell.

Linux only (AF_UNIX); WSL is fine. It takes a couple of seconds.

What it is checking, in one line each:

  * `yos perception` on a machine where the service has never been started asks the shell to
    start it, with exactly the action and arguments the shell's control surface publishes, and
    then reads the feed — the defect this file was written for, where "on demand" was a caption
    on a process nothing anywhere ever ran and every os_perception answered "no socket";
  * a service that is already answering is not started a second time, so a privileged
    perception-service is found rather than shadowed;
  * a source that could not start is reported whatever the count asked for, because those
    notices are the oldest observations in the run and a limit would slice them off the end —
    leaving a reader to take a quiet feed for a quiet machine;
  * with no desktop to ask, the answer is a plain sentence and exit 0 — not a traceback and not
    the "failed (exit 1)" that a mind used to be handed for a perfectly good question;
  * `ensure_service`, which the notify path uses, still ends the command when the service
    will not come up;
  * `yos act` sends each argument as the type the app declared for it — `dismiss id=67`
    reaches the service as the string "67", because that action publishes `id: string`, while
    `add_event duration_min=30` still arrives as the number 30;
  * and when an app refuses an action for want of a grant — `sensitive` in `ask` mode, which
    the app's own dispatch refuses on every door since issue #116 — `yos act` asks the shell
    for the person's Allow, says so, waits, and acts again carrying the grant; a denial and an
    unanswered card are plain sentences, `--no-ask` hands the refusal back, and `--grant`
    carries one already held;
  * the same path for a service that answers `app.act` itself: `yos act system-monitor
    kill_process` with the window shut reaches the service, which refuses a `dangerous` act in
    `ask` mode with the same sentence since issue #153, and `yos` asks and acts again there;
  * an agent's token — `--agent-token`, `YANTRIK_AGENT_TOKEN`, or an `agent_token=` argument —
    rides beside the arguments on `app.act` and never among them, so the approval card it may
    raise is asked for with the arguments alone; and an act told to `wait` is given longer than
    its wait;
  * `yos check` passes a fake surface that keeps docs/surface-protocol.md, runs none of its
    handlers doing it, and names only its safe, recoverable action in a probe; it fails a surface
    that keeps nothing on each thing it breaks, and sends it nothing that names a real action;
  * the describe schema `yos check` carries is docs/schema/describe.schema.json, the envelopes the
    Rust builders make (surface-vectors.json) match both schemas, and the revision, the phrase
    list and every revision the spec quotes are the gate's;
  * an app that declares a surface in its `.desktop` file is found while it is closed: the keys
    are read as the shell reads them, an alias or the app's name reaches its socket without the
    shell's link, a closed one is named as closed with how to open it, and `yos ls` lists it as
    `(closed)` with what it is for — from the shell's listing, or from the files when no desktop
    answers;
  * a socket that answers but not `app.describe` — the harness host, the store behind an app —
    is named nowhere in `yos ls` (only under `--all`, as the desktop's own), so no mind is invited
    to describe it (#190); describing it anyway says what it is, and the store behind a closed app
    says the app is closed and how to open it;
  * `yos` writes nothing to `app-shell.sock` unless a `yantrik-ui` binary is what listens there;
  * the socket-directory chain is the transport's, in its order;
  * and a describe state carrying the shell's large `agents` and `catalog` lists is printed in
    the fixed order — the small, high-value fields first, with `clock` inside the first bytes a
    length-clipped reader keeps, and the large lists last (#319).
"""

import contextlib
import importlib.util
import io
import json
import os
import pathlib
import re
import shutil
import socket
import sys
import tempfile
import threading
from importlib.machinery import SourceFileLoader

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = HERE / "yos"

FAILURES = []


def check(label, condition, detail=""):
    if condition:
        print("  ok    %s" % label)
    else:
        # `str`, like the MCP bridge's own selftest: a check handed the dict it was comparing
        # used to die here with a TypeError instead of printing what it saw, which turns a
        # readable failure into a traceback in the middle of the run.
        print("  FAIL  %s%s" % (label, ("\n        " + str(detail)) if detail else ""))
        FAILURES.append(label)


class FakeService(threading.Thread):
    """One socket that answers one line at a time, and records what it was asked.

    A connection that sends nothing is the liveness probe `yos` uses to decide whether anything
    is behind a socket at all — so it is accepted and dropped rather than treated as an error.
    """

    daemon = True

    def __init__(self, path, reply):
        super().__init__()
        self.path = str(path)
        self.reply = reply
        self.calls = []
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(self.path)
        self.listener.listen(8)
        self.stopping = False

    def run(self):
        while not self.stopping:
            try:
                conn, _ = self.listener.accept()
            except OSError:
                return
            with conn:
                conn.settimeout(2)
                buf = b""
                try:
                    while not buf.endswith(b"\n"):
                        chunk = conn.recv(1 << 16)
                        if not chunk:
                            break
                        buf += chunk
                except OSError:
                    continue
                if not buf.strip():
                    continue  # the liveness probe
                asked = json.loads(buf)
                self.calls.append(asked)
                answer = self.reply(self, asked)
                if answer is None:
                    continue
                # A reply of `{"__error__": {...}}` is a JSON-RPC error — how an app's dispatch
                # refuses, and what `yos act` has to be able to read past.
                if isinstance(answer, dict) and set(answer) == {"__error__"}:
                    conn.sendall((json.dumps({"jsonrpc": "2.0", "id": asked.get("id"),
                                              "error": answer["__error__"]}) + "\n").encode())
                    continue
                conn.sendall((json.dumps({"jsonrpc": "2.0", "id": asked.get("id"),
                                          "result": answer}) + "\n").encode())

    def close(self):
        self.stopping = True
        self.listener.close()
        with contextlib.suppress(OSError):
            os.unlink(self.path)


PAGE = {
    # What a session-started perception-service actually answers with: it has no CAP_NET_ADMIN
    # and no CAP_SYS_ADMIN, so it comes up on PSI alone and says which sources it had to do
    # without, at the salience it gives a source going blind.
    "observations": [
        {"kind": {"type": "source_failed", "source": "processes",
                  "reason": "bind: needs CAP_NET_ADMIN"},
         "salience": 1.0, "summary": "perception cannot use its processes source: "
                                     "bind: needs CAP_NET_ADMIN"},
        {"kind": {"type": "source_failed", "source": "files",
                  "reason": "fanotify_init: needs CAP_SYS_ADMIN"},
         "salience": 1.0, "summary": "perception cannot use its files source: "
                                     "fanotify_init: needs CAP_SYS_ADMIN"},
        {"kind": {"type": "pressure", "resource": "io", "stalled_pct_10s": 71.6},
         "salience": 0.8, "summary": "io stalled 72% of the last 10s"},
    ],
    "next_seq": 3,
    "missed": 0,
}

# Two surfaces as they really publish themselves, because the types in here are the whole point:
# the notification ids on a live machine are a decimal counter rendered as a string, and the
# calendar is where a number and a flag genuinely are a number and a flag.
NOTIFICATIONS = {
    "app": "notifications",
    "summary": "Notifications — 2 showing, 1 unread",
    "state": {"count": 2, "unread": 1},
    "revision": "dd6f0b179da77956",
    "actions": [
        {"name": "dismiss", "description": "Dismiss one notification by id",
         "permission": "standard", "settles": "on return",
         "parameters": {"type": "object", "required": ["id"], "properties": {
             "id": {"type": "string",
                    "description": "The notification id, as shown in the list"}}}},
        {"name": "mark_read", "description": "Clear the unread badge",
         "permission": "standard", "settles": "on return",
         "parameters": {"type": "object", "required": [], "properties": {
             "id": {"type": "string", "description": "One notification, or omit for all"}}}},
    ],
}

CALENDAR = {
    "app": "calendar",
    "summary": "Calendar — September 2026",
    "state": {"showing": "September 2026"},
    "revision": "1234567890123456",
    "actions": [
        {"name": "add_event", "description": "Put something on the calendar",
         "permission": "standard", "settles": "on return",
         "parameters": {"type": "object", "required": ["title", "date"], "properties": {
             "title": {"type": "string", "description": ""},
             "date": {"type": "string", "description": "YYYY-MM-DD"},
             "duration_min": {"type": "number", "description": "How long it runs, in minutes"},
             "all_day": {"type": "boolean", "description": "A whole day rather than a time"}}}},
    ],
}


def load_yos(runtime_dir):
    """The real `yos`, with its socket directory pointed at the scratch one.

    `SOCKET_DIRS` is built at import time from `XDG_RUNTIME_DIR`, and it also lists
    `/run/yantrik` — which on a developer's machine may hold a real perception socket. Replacing
    the list outright is what makes this test say the same thing everywhere.
    """
    os.environ["XDG_RUNTIME_DIR"] = str(runtime_dir)
    loader = SourceFileLoader("yos_under_test", str(SOURCE))
    spec = importlib.util.spec_from_loader("yos_under_test", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    module.SOCKET_DIRS = [str(pathlib.Path(runtime_dir) / "yantrik")]
    # The fake shell below is served by this very process, so the kernel names this Python as the
    # process behind `app-shell.sock`. `yos` refuses to talk to anything but a `yantrik-ui` binary
    # there; this test's own interpreter is added to what it accepts, and the refusal itself is
    # checked on its own, with the real list, further down.
    module.SHELL_BINARIES = ("yantrik-ui", os.path.basename(os.readlink("/proc/self/exe")))
    return module


def fnv(summary, state):
    """`View::revision()` written out a second time, so a fake surface can publish a revision that
    was not computed by the function under test."""
    h = 0xCBF29CE484222325
    text = json.dumps(state, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    for byte in summary.encode("utf-8") + b"\x00" + text.encode("utf-8"):
        h = ((h ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return "%016x" % h


class ProtocolSurface:
    """A surface that keeps docs/surface-protocol.md: the dispatch's checks in the dispatch's
    order and words, and a record of every handler that ran — which `yos check` must leave empty.
    """

    ACTIONS = [
        {"name": "greet", "description": "Say hello to someone", "permission": "safe",
         "settles": "on return",
         "parameters": {"type": "object", "required": ["name"], "properties": {
             "loud": {"type": "boolean", "description": "Shout it"},
             "name": {"type": "string", "description": "Who to greet"}}}},
        {"name": "wipe", "description": "Delete every greeting. It is not recoverable.",
         "permission": "standard", "settles": "on return",
         "parameters": {"type": "object", "required": [], "properties": {}}},
        {"name": "render", "description": "Render the greetings to a file",
         "permission": "sensitive", "settles": "later",
         "parameters": {"type": "object", "required": ["out"], "properties": {
             "out": {"type": "string", "description": "Where to write"}}}},
    ]

    def __init__(self):
        self.ran = []
        self.summary, self.state = "Hello — 2 greetings", {"greetings": 2, "last": "Ada"}

    def describe(self):
        return {"protocol": 1, "app": "hello", "summary": self.summary, "state": self.state,
                "revision": fnv(self.summary, self.state), "actions": self.ACTIONS}

    def reply(self, _svc, asked):
        refuse = lambda message, code=-32602: {"__error__": {"code": code, "message": message}}
        if asked["method"] == "rpc.ping":
            return "pong"
        if asked["method"] == "app.describe":
            return self.describe()
        if asked["method"] != "app.act":
            return refuse("unknown method `%s`; this app serves app.describe, app.act"
                          % asked["method"], -32601)
        params = asked.get("params") or {}
        name = (params.get("action") or "").strip()
        args = params.get("args") or {}
        if not name:
            return refuse("act needs a non-empty `action`")
        spec = next((a for a in self.ACTIONS if a["name"] == name), None)
        if spec is None:
            return refuse("unknown action `%s`; this app offers: %s"
                          % (name, ", ".join(a["name"] for a in self.ACTIONS)))
        declared = list(spec["parameters"]["properties"])
        for p in spec["parameters"]["required"]:
            if p not in args:
                return refuse("`%s` needs argument `%s`" % (name, p))
        for key in sorted(args):
            if key not in declared:
                return refuse(("`%s` has no argument `%s`; it takes: %s"
                               % (name, key, ", ".join(declared))) if declared else
                              "`%s` takes no arguments, but `%s` was given" % (name, key))
        kinds = {"string": (str, "a string"), "boolean": (bool, "a boolean")}
        arrived = lambda v: ("a boolean" if isinstance(v, bool) else "a number"
                             if isinstance(v, (int, float)) else "a string"
                             if isinstance(v, str) else "an object")
        for p, pspec in spec["parameters"]["properties"].items():
            kind, wanted = kinds[pspec["type"]]
            value = args.get(p)
            if value is not None and not (isinstance(value, kind)
                                          and (kind is bool or not isinstance(value, bool))):
                return refuse("`%s` argument `%s` must be %s, and %s arrived"
                              % (name, p, wanted, arrived(value)))
        current = fnv(self.summary, self.state)
        if params.get("expect_revision") not in (None, current):
            return refuse("STALE: this app is at revision %s and you acted on %s. It now reports: "
                          "%s. Read it again before deciding."
                          % (current, params["expect_revision"], self.summary))
        self.ran.append((name, args))
        return {"app": "hello", "action_id": "app-hello#1", "accepted": True, "settled": True,
                "result": {}, "revision": current, "summary": self.summary, "state": self.state}


# A surface that keeps nothing: no `protocol`, a grade off the ladder, a parameter typed as
# nothing JSON Schema has, a password among the arguments, a revision that is not the hash, and an
# unknown-action refusal in a service's own words — as the three services that answer `app.act`
# themselves still do (their dispatch moves onto the shared one in the SDK's piece B).
BROKEN = {
    "app": "broken", "summary": "Broken", "state": {"x": 1}, "revision": "0000000000000000",
    "actions": [
        {"name": "connect", "description": "Join a network", "permission": "standard",
         "settles": "later",
         "parameters": {"type": "object", "required": ["ssid"], "properties": {
             "ssid": {"type": "string", "description": ""},
             "password": {"type": "string", "description": ""}}}},
        {"name": "nuke", "description": "Everything", "permission": "catastrophic",
         "settles": "on return", "parameters": {"type": "object", "required": [], "properties": {
             "how": {"type": "list", "description": ""}}}},
    ],
}


def run(fn):
    """Call `fn`, returning (stdout, stderr, exit code or None)."""
    out, err = io.StringIO(), io.StringIO()
    code = None
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            fn()
        except SystemExit as e:
            code = e.code if e.code is not None else 0
    return out.getvalue(), err.getvalue(), code


def main():
    if not hasattr(socket, "AF_UNIX"):
        print("skipped: this test needs unix sockets")
        return 0

    # A token in the environment this runs in would ride on every act below and change what they
    # send; the agent-token checks set their own.
    os.environ.pop("YANTRIK_AGENT_TOKEN", None)
    tmp = tempfile.mkdtemp(prefix="yos-selftest-")
    sockets = pathlib.Path(tmp) / "yantrik"
    sockets.mkdir(parents=True)
    yos = load_yos(tmp)
    started = []
    services = []

    def perception_reply(_self, asked):
        return PAGE if asked["method"] == "perception.since" else {}

    def shell_reply(_self, asked):
        """The shell's control surface, as far as this test needs it.

        `start_service` is the standard-grade action the shell publishes and the ServiceManager
        backs; the fake honours it by putting the service's socket where a started one would be.
        """
        params = asked.get("params") or {}
        if asked["method"] == "app.act" and params.get("action") == "start_service":
            name = (params.get("args") or {}).get("name")
            started.append(name)
            svc = FakeService(sockets / ("%s.sock" % name), perception_reply)
            svc.start()
            services.append(svc)
            return {"accepted": True, "settled": True,
                    "result": {"service": name, "state": "started"}}
        return {"accepted": True, "settled": True}

    shell = FakeService(sockets / "app-shell.sock", shell_reply)
    shell.start()
    services.append(shell)

    try:
        print("yos perception, on a machine where the service has never been started")
        out, err, code = run(lambda: yos.cmd_perception([]))
        check("it asked the desktop to start perception", started == ["perception"],
              "start_service was asked for %r" % (started,))
        acts = [c for c in shell.calls if c["method"] == "app.act"]
        check("through the action the shell publishes, with the arguments it names",
              acts and acts[0]["params"] == {"action": "start_service",
                                             "args": {"name": "perception"}},
              "sent %s" % json.dumps(acts[0]["params"] if acts else None))
        check("and then read the feed", "3 held, next_seq 3, missed 0" in out, out)
        check("naming the sources it cannot use", "needs CAP_NET_ADMIN" in out, out)
        check("with nothing on stderr and no failure", err == "" and code is None,
              "stderr=%r exit=%r" % (err, code))

        print("yos perception, with the service already answering")
        started.clear()
        shell.calls.clear()
        # Asserted rather than assumed: with nothing behind the socket the next check would
        # pass for the wrong reason, which is how it read on the version this test was written
        # against. `_answers` and `socket_candidates` rather than the module's own `is_up`, so
        # this line says the same thing about a `yos` that predates it.
        check("the service is up before this case",
              any(yos._answers(p) for p in yos.socket_candidates("perception")))
        out, err, code = run(lambda: yos.cmd_perception([]))
        check("nothing was started a second time", started == [],
              "start_service was asked for %r" % (started,))
        check("and the feed was still read", "3 held" in out, out)

        print("yos perception 1, where the limit would cut off the blindness")
        out, err, code = run(lambda: yos.cmd_perception(["1"]))
        check("the newest observation is shown", "io stalled" in out, out)
        check("and so is every source that could not start, whatever the limit",
              "needs CAP_NET_ADMIN" in out and "needs CAP_SYS_ADMIN" in out, out)

        print("yos act, against the types the app publishes")
        # `parse_args` ran every value through `json.loads`, so `dismiss id=67` sent the number
        # 67 to an action that publishes `id: string` and the service answered "missing `id`" —
        # a mind that had read the id off the list could not dismiss a notification at all.

        def surface(view):
            """A control surface that describes itself and accepts anything."""
            def reply(_self, asked):
                if asked["method"] == "app.describe":
                    return view
                return {"summary": view["summary"], "accepted": True, "settled": True,
                        "revision": view["revision"], "result": {"ok": True}}
            return reply

        surfaces = {}
        for view in (NOTIFICATIONS, CALENDAR):
            service = FakeService(sockets / ("%s.sock" % view["app"]), surface(view))
            service.start()
            services.append(service)
            surfaces[view["app"]] = service

        def last_act(app):
            acts = [c for c in surfaces[app].calls if c["method"] == "app.act"]
            return acts[-1]["params"] if acts else None

        def describes(app):
            return len([c for c in surfaces[app].calls if c["method"] == "app.describe"])

        run(lambda: yos.cmd_act(["notifications", "dismiss", "id=67"]))
        check("an id bound for a `string` parameter arrives as the text it was typed as",
              (last_act("notifications") or {}).get("args") == {"id": "67"},
              last_act("notifications"))
        check("and the app was asked once what its arguments are",
              describes("notifications") == 1, describes("notifications"))

        surfaces["notifications"].calls.clear()
        run(lambda: yos.cmd_act(["notifications", "dismiss", "id=abc"]))
        check("a value that is not JSON in the first place costs no extra round trip",
              describes("notifications") == 0, surfaces["notifications"].calls)

        surfaces["notifications"].calls.clear()
        run(lambda: yos.cmd_act(["notifications", "dismiss", 'id="67"']))
        check("quoting still says `string` explicitly, without the quotes surviving",
              (last_act("notifications") or {}).get("args") == {"id": "67"},
              last_act("notifications"))

        surfaces["notifications"].calls.clear()
        run(lambda: yos.cmd_act(["notifications", "dismiss", "id=67", "reason=3"]))
        check("an argument the app does not publish is still read as JSON, as it always was",
              (last_act("notifications") or {}).get("args") == {"id": "67", "reason": 3},
              last_act("notifications"))

        surfaces["notifications"].calls.clear()
        # 16 hex digits, and about one revision in six thousand is all of them decimal. It
        # guards the call rather than being an argument to it, so no app declares it — and as a
        # number the runtime reads it with `as_str`, finds nothing, and acts without the guard.
        run(lambda: yos.cmd_act(["notifications", "dismiss", "id=67",
                                 "expect_revision=1234567890123456"]))
        check("a revision made only of digits is still a revision",
              (last_act("notifications") or {}).get("expect_revision") == "1234567890123456",
              last_act("notifications"))

        run(lambda: yos.cmd_act(["calendar", "add_event", "title=Dentist", "date=2026-10-02",
                                 "duration_min=30", "all_day=false"]))
        check("a number stays a number and a flag stays a flag",
              (last_act("calendar") or {}).get("args") == {
                  "title": "Dentist", "date": "2026-10-02",
                  "duration_min": 30, "all_day": False},
              last_act("calendar"))

        print("yos act, when the desktop wants a person's Allow")
        # The account from inside VM 520 (issue #116): `blender.render` is `sensitive`, the
        # machine was in `ask` mode, and `yos act` ran it in 1.72 s with no card. The app's own
        # dispatch refuses that now, on every door, and says how to get a grant. `yos` has to
        # read that refusal, ask the shell, say so, wait for the person, and act again with the
        # grant — the same three steps the MCP bridge does for a mind, for whoever is at a
        # terminal.
        REFUSAL = ("GRANT: blender.render is graded `sensitive` and this machine is in ask mode, "
                   "which runs nothing above `standard` without asking — so it was not run. Ask "
                   "the shell for approval first (`request_approval` with this app, action and "
                   "these exact arguments, poll `approval_status`, then send the granted "
                   "request_id as `grant` on app.act — `yos act` does all of that for you), or "
                   "have the person at the machine press Allow when the card appears.")
        RENDERED = {"summary": "Blender — rendered", "accepted": True, "settled": True,
                    "revision": "b1", "result": {"rendered_to": "x.png"}}
        answers = {"status": "granted"}
        polls = []

        def blender_reply(_self, asked):
            params = asked.get("params") or {}
            if asked["method"] != "app.act":
                return {"app": "blender", "summary": "Blender — cube.blend", "state": {},
                        "revision": "b0", "actions": []}
            if params.get("grant") == "appr-7":
                return RENDERED
            if params.get("grant"):
                return {"__error__": {"code": -32602, "message": (
                    "GRANT: `%s` does not authorise blender.render — no approval request `%s`. "
                    "Nothing was run" % (params["grant"], params["grant"]))}}
            return {"__error__": {"code": -32602, "message": REFUSAL}}

        def asking_shell_reply(_self, asked):
            params = asked.get("params") or {}
            action = params.get("action")
            if asked["method"] == "app.act" and action == "request_approval":
                return {"accepted": True, "settled": True, "result": {
                    "request_id": "appr-7", "status": "pending", "expires_in_secs": 120}}
            if asked["method"] == "app.act" and action == "approval_status":
                polls.append(1)
                # Pending on the first poll, so the wait is a real wait.
                status = answers["status"] if len(polls) >= 2 else "pending"
                return {"accepted": True, "settled": True,
                        "result": {"request_id": "appr-7", "status": status}}
            return {"accepted": True, "settled": True}

        blender = FakeService(sockets / "app-blender.sock", blender_reply)
        blender.start()
        services.append(blender)
        shell.reply = asking_shell_reply
        shell.calls.clear()
        yos.APPROVAL_POLL = 0.02

        def acts():
            return [c["params"] for c in blender.calls if c["method"] == "app.act"]

        def asked():
            return [c["params"]["args"] for c in shell.calls
                    if c["method"] == "app.act" and c["params"].get("action") == "request_approval"]

        out, err, code = run(lambda: yos.cmd_act(["blender", "render", "out=x.png"]))
        check("the refusal made yos ask the shell for the person's Allow, once",
              len(asked()) == 1, shell.calls)
        check("with the app, action, grade and the exact arguments the grant binds",
              asked() and asked()[0].get("app") == "blender" and asked()[0].get("action") == "render"
              and asked()[0].get("grade") == "sensitive" and asked()[0].get("args_json") == {"out": "x.png"},
              asked())
        check("and said so on the terminal",
              "asking — a card is on the screen (120 s)" in out, out)
        check("the action was sent once without a grant and once with the one the person gave",
              [a.get("grant") for a in acts()] == [None, "appr-7"], acts())
        check("carrying the same arguments both times",
              all(a.get("args") == {"out": "x.png"} for a in acts()), acts())
        check("and the second one ran, with nothing on stderr",
              "accepted: True" in out and code is None and err == "", (out, err, code))

        answers["status"] = "denied"
        polls.clear()
        blender.calls.clear()
        shell.calls.clear()
        out, err, code = run(lambda: yos.cmd_act(["blender", "render", "out=x.png"]))
        check("a denial runs nothing", [a.get("grant") for a in acts()] == [None], acts())
        check("and is a plain sentence that ends the command",
              code == 1 and "said no" in err and "Traceback" not in err, (err, code))

        answers["status"] = "pending"
        polls.clear()
        blender.calls.clear()
        yos.APPROVAL_WAIT = 0.2
        out, err, code = run(lambda: yos.cmd_act(["blender", "render", "out=x.png"]))
        check("an unanswered card runs nothing and says nobody answered",
              code == 1 and "nobody answered" in err and len(acts()) == 1, (err, acts()))
        yos.APPROVAL_WAIT = 120

        shell.calls.clear()
        blender.calls.clear()
        out, err, code = run(lambda: yos.cmd_act(["blender", "render", "out=x.png", "--no-ask"]))
        check("--no-ask hands the refusal back instead of asking",
              code == 1 and "GRANT:" in err and not asked(), (err, shell.calls))
        check("in the app's own words, without the transport's prefix",
              "app.act refused:" not in err, err)

        blender.calls.clear()
        out, err, code = run(lambda: yos.cmd_act(["blender", "render", "out=x.png", "--grant", "appr-7"]))
        check("--grant carries a grant already held, and asks nobody",
              [a.get("grant") for a in acts()] == ["appr-7"] and code is None and not asked(),
              (acts(), err))

        blender.calls.clear()
        out, err, code = run(lambda: yos.cmd_act(["blender", "render", "out=x.png", "--grant", "stale"]))
        check("a grant that does not hold is a refusal, not a second card",
              code == 1 and "does not authorise" in err and not asked() and len(acts()) == 1,
              (err, acts()))

        print("yos act, for one of the person's agents")
        # The agent token says which agent a call is for. It rides BESIDE the arguments, never
        # among them: the arguments are what the approval card shows and the audit log keeps,
        # and a token in either is a token anyone reading them could replay.

        def shell_acts(action):
            return [c["params"] for c in shell.calls
                    if c["method"] == "app.act" and c["params"].get("action") == action]

        shell.calls.clear()
        run(lambda: yos.cmd_act(["shell", "agent_run", "command=ls -la", "--agent-token", "tok-flag"]))
        sent = shell_acts("agent_run")
        check("--agent-token travels beside the arguments",
              sent and sent[-1].get("agent_token") == "tok-flag", sent)
        check("and never among them", sent and sent[-1].get("args") == {"command": "ls -la"}, sent)

        os.environ["YANTRIK_AGENT_TOKEN"] = "tok-env"
        try:
            shell.calls.clear()
            run(lambda: yos.cmd_act(["shell", "agent_run", "command=ls"]))
            sent = shell_acts("agent_run")
            check("YANTRIK_AGENT_TOKEN, as a harness sets it, is carried the same way",
                  sent and sent[-1].get("agent_token") == "tok-env"
                  and sent[-1].get("args") == {"command": "ls"}, sent)
            shell.calls.clear()
            run(lambda: yos.cmd_act(["shell", "agent_run", "command=ls", "--agent-token", "tok-flag"]))
            sent = shell_acts("agent_run")
            check("a token given on the command line wins over the environment's",
                  sent and sent[-1].get("agent_token") == "tok-flag", sent)
        finally:
            os.environ.pop("YANTRIK_AGENT_TOKEN", None)

        shell.calls.clear()
        run(lambda: yos.cmd_act(["shell", "agent_run", "command=ls", "agent_token=0042"]))
        sent = shell_acts("agent_run")
        check("an agent_token= argument is lifted out beside the rest, as the text it was typed as",
              sent and sent[-1].get("agent_token") == "0042"
              and sent[-1].get("args") == {"command": "ls"}, sent)

        shell.calls.clear()
        run(lambda: yos.cmd_act(["shell", "agent_run", "command=ls"]))
        sent = shell_acts("agent_run")
        check("with no token anywhere, none is sent", sent and "agent_token" not in sent[-1], sent)

        # The card: what the person is shown, and what a grant is bound to, is the arguments —
        # so the token has to be absent from the request and present on both acts.
        answers["status"] = "granted"
        polls.clear()
        blender.calls.clear()
        shell.calls.clear()
        out, err, code = run(lambda: yos.cmd_act(["blender", "render", "out=x.png",
                                                  "--agent-token", "tok-card"]))
        check("the card is asked for with the arguments alone",
              asked() and asked()[0].get("args_json") == {"out": "x.png"}, asked())
        check("and nothing sent to the shell carries the token",
              "tok-card" not in json.dumps(shell.calls), shell.calls)
        check("while both acts carry it beside the same arguments",
              [a.get("agent_token") for a in acts()] == ["tok-card", "tok-card"]
              and all(a.get("args") == {"out": "x.png"} for a in acts())
              and [a.get("grant") for a in acts()] == [None, "appr-7"], acts())

        check("an act told to wait is given longer than its wait before yos gives up on it",
              yos.act_timeout("agent_run", {}) == 140
              and yos.act_timeout("agent_run", {"wait": 600}) == 620
              and yos.act_timeout("agent_job", {"wait": 0}) == 40
              and yos.act_timeout("dismiss", {"id": "67"}) == 40,
              [yos.act_timeout("agent_run", {}), yos.act_timeout("agent_run", {"wait": 600})])

        print("yos act, against a service that answers app.act itself (issue #153)")
        # System Monitor's service answers `app.act` in its own handler, and with the window shut
        # `yos act system-monitor` reaches `system-monitor.sock`, not `app-system-monitor`. Its
        # `kill_process` ran on any call until #153. Under a ceiling a person has raised to
        # `dangerous`, in ask mode, the service now refuses it with the sentence every app's
        # dispatch uses — and `yos` has to take the same ask-and-wait path, to the same socket.
        KILL_REFUSAL = (REFUSAL.replace("blender.render", "system-monitor.kill_process")
                        .replace("`sensitive`", "`dangerous`"))
        SYSMON = {
            "app": "system-monitor", "summary": "System — CPU 3%", "state": {},
            "revision": "s0",
            "actions": [
                {"name": "kill_process", "description": "End a running process by PID",
                 "permission": "dangerous", "settles": "on return",
                 "parameters": {"type": "object", "required": ["pid"], "properties": {
                     "pid": {"type": "number", "description": "The process id to end"}}}},
            ],
        }

        def sysmon_reply(_self, asked):
            params = asked.get("params") or {}
            if asked["method"] == "app.describe":
                return SYSMON
            if params.get("grant") == "appr-7":
                return {"summary": "System — CPU 2%", "accepted": True, "settled": True,
                        "revision": "s1", "result": {"killed": params["args"].get("pid")}}
            return {"__error__": {"code": -32602, "message": KILL_REFUSAL}}

        sysmon = FakeService(sockets / "system-monitor.sock", sysmon_reply)
        sysmon.start()
        services.append(sysmon)
        answers["status"] = "granted"
        polls.clear()
        shell.calls.clear()

        def kills():
            return [c["params"] for c in sysmon.calls if c["method"] == "app.act"]

        out, err, code = run(lambda: yos.cmd_act(["system-monitor", "kill_process", "pid=4242"]))
        check("the service's refusal made yos ask the shell, once",
              len(asked()) == 1, shell.calls)
        check("for system-monitor.kill_process, graded dangerous, with the pid the grant binds",
              asked() and asked()[0].get("app") == "system-monitor"
              and asked()[0].get("action") == "kill_process"
              and asked()[0].get("grade") == "dangerous"
              and asked()[0].get("args_json") == {"pid": 4242},
              asked())
        check("and said so on the terminal",
              "asking — a card is on the screen (120 s)" in out, out)
        check("the service was asked twice: without a grant, then with the person's",
              [k.get("grant") for k in kills()] == [None, "appr-7"]
              and all(k.get("args") == {"pid": 4242} for k in kills()), kills())
        check("and the second one ran", "accepted: True" in out and code is None, (out, err, code))

        answers["status"] = "denied"
        polls.clear()
        sysmon.calls.clear()
        out, err, code = run(lambda: yos.cmd_act(["system-monitor", "kill_process", "pid=4242"]))
        check("a denial ends nothing", [k.get("grant") for k in kills()] == [None]
              and code == 1 and "said no" in err, (kills(), err))

        print("yos check, against a surface that keeps the protocol")
        hello = ProtocolSurface()
        hello_svc = FakeService(sockets / "app-hello.sock", hello.reply)
        hello_svc.start()
        services.append(hello_svc)
        out, err, code = run(lambda: yos.cmd_check(["hello"]))
        check("it passes, and says what it saw for every check",
              code is None and "0 failed" in out
              and all(("  pass  %s" % c) in out for c in (
                  "ping", "describe", "protocol", "schema", "grades", "params", "secrets",
                  "revision", "steady", "method", "empty", "unknown", "missing", "undeclared",
                  "types", "stale")), out + err)
        check("no handler ran: every act it sent was one the dispatch refuses first",
              hello.ran == [], hello.ran)
        acts = [c["params"] for c in hello_svc.calls if c["method"] == "app.act"]
        named = sorted({a.get("action") for a in acts if a.get("action")})
        check("the only real action it named is the safe, recoverable one",
              named == ["greet", "yos-check-no-such-action"], named)
        check("and every act that named one carried a revision the app had moved past",
              all(a.get("expect_revision") and a["expect_revision"] != hello.describe()["revision"]
                  for a in acts if a.get("action")), acts)
        out, err, code = run(lambda: yos.cmd_check(["hello", "--json"]))
        try:
            answer = json.loads(out)
        except ValueError:
            answer = {}
        rows = (answer.get("surfaces") or [{}])[0].get("checks") or []
        check("--json says the same, as JSON",
              answer.get("ok") is True and len(rows) == 16
              and {r["status"] for r in rows} == {"pass"}, out)
        out, err, code = run(lambda: yos.cmd_check([str(sockets / "app-hello.sock")]))
        check("a socket can be named by its path, for a surface under development",
              code is None and "0 failed" in out, out + err)

        print("yos check, against a surface that keeps nothing")
        broken_svc = FakeService(sockets / "app-broken.sock", lambda _s, asked: (
            "pong" if asked["method"] == "rpc.ping" else
            BROKEN if asked["method"] == "app.describe" else
            {"__error__": {"code": -32601, "message": "unknown action `%s`; this service offers: "
                           "connect, nuke" % (asked.get("params") or {}).get("action")}}))
        broken_svc.start()
        services.append(broken_svc)
        out, err, code = run(lambda: yos.cmd_check(["broken"]))
        failed = {line.split()[1] for line in out.splitlines() if line.startswith("  fail")}
        check("it fails, exit 1", code == 1, (code, out))
        check("on the missing protocol, the schema, the grade, the parameter type, the password, "
              "the revision and the unknown-action refusal",
              {"protocol", "schema", "grades", "params", "secrets", "revision", "unknown"} <= failed,
              sorted(failed))
        check("naming what it saw",
              "nuke is graded \"catastrophic\"" in out and "connect(password)" in out
              and "nuke(how) is typed \"list\"" in out and "this service offers" in out, out)
        acts = [c["params"] for c in broken_svc.calls if c["method"] == "app.act"]
        check("and a dispatch that is not the protocol's is sent nothing that names a real action",
              all(a.get("action") in (None, "yos-check-no-such-action") for a in acts)
              and "  skip  missing" in out, acts)

        print("yos check's schema is the one beside the spec, and the envelopes the Rust builders "
              "make match both schemas")
        repo = HERE.parent.parent
        on_disk = json.loads((repo / "docs" / "schema" / "describe.schema.json").read_text("utf-8"))
        check("the describe schema yos carries is docs/schema/describe.schema.json",
              yos.DESCRIBE_SCHEMA == on_disk, "regenerate one from the other")
        vectors = json.loads((HERE / "surface-vectors.json").read_text("utf-8"))
        act_schema = json.loads((repo / "docs" / "schema" / "act.schema.json").read_text("utf-8"))
        envelopes = vectors.get("envelopes") or {}
        check("describe_json's envelope matches the describe schema",
              envelopes.get("describe") and not yos.schema_errors(envelopes["describe"], on_disk),
              yos.schema_errors(envelopes.get("describe") or {}, on_disk))
        check("act_json's envelope matches the act schema",
              envelopes.get("act") and not yos.schema_errors(envelopes["act"], act_schema),
              yos.schema_errors(envelopes.get("act") or {}, act_schema))
        refusal = {"code": -32602, "message": "STALE: …"}
        check("and a refusal matches the act schema's",
              not yos.schema_errors(refusal, {"$ref": "#/$defs/refusal"}, act_schema)
              and yos.schema_errors({"code": "x"}, {"$ref": "#/$defs/refusal"}, act_schema), None)
        wrong = [v["summary"] for v in vectors.get("revision") or []
                 if yos.revision_of(v["summary"], v["state"]) != v["revision"]]
        check("the revision yos computes is the gate's, on every vector",
              vectors.get("revision") and not wrong, wrong)
        check("and its phrase list is the gate's",
              list(yos.UNRECOVERABLE_PHRASES) == vectors.get("phrases"), yos.UNRECOVERABLE_PHRASES)
        spec = (repo / "docs" / "surface-protocol.md").read_text("utf-8")
        generated = {v["revision"] for v in vectors.get("revision") or []}
        generated.add((envelopes.get("describe") or {}).get("revision"))
        quoted = set(re.findall(r"`([0-9a-f]{16})`", spec)) | set(
            re.findall(r'"revision": "([0-9a-f]{16})"', spec))
        check("every revision the spec quotes is one the code generated",
              quoted and quoted <= generated, sorted(quoted - generated))

        print("a closed app is found by its .desktop file: listed, resolved by alias, and named "
              "when it is closed")
        # The keys an app declares itself with (docs/app-control.md, "Findable while closed"),
        # in a scratch applications directory: one app somebody else wrote, one of ours.
        apps_dir = pathlib.Path(tmp) / "applications"
        apps_dir.mkdir()
        (apps_dir / "org.example.Howdy.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Howdy\nExec=/usr/bin/hello\n"
            "X-Yantrik-Surface=howdy\nX-Yantrik-Purpose=say hello to someone, by name\n"
            "X-Yantrik-Aliases=hi;Greeter;shell;../x\n", "utf-8")
        (apps_dir / "yantrik-system-monitor.desktop").write_text(
            (repo / "apps" / "desktop-files" / "yantrik-system-monitor.desktop").read_text("utf-8"),
            "utf-8")
        (apps_dir / "org.example.Quiet.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Quiet\nExec=/usr/bin/quiet\n"
            "X-Yantrik-Surface=quiet\nX-Yantrik-Purpose=nothing to see here, very quietly\n"
            "X-Yantrik-Aliases=hush\n", "utf-8")
        (apps_dir / "vim.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim %F\n", "utf-8")
        (apps_dir / "hidden.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Hidden\nExec=h\nNoDisplay=true\n"
            "X-Yantrik-Surface=hidden\n", "utf-8")
        # `TryExec` is the standard "only if this program is installed" rule, and the shell's
        # catalogue applies it too (#214): Ghost names a program no machine has, so nothing
        # lists it, however fine its Exec looks. Wrapped is an adapter's entry's shape — its
        # Exec is a wrapper that is not here, but the app it names IS — and it is listed like
        # any other.
        (apps_dir / "org.example.Ghost.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Ghost\nExec=/usr/bin/ghost-wrapper\n"
            "TryExec=no-such-program-anywhere\nX-Yantrik-Surface=ghost\n"
            "X-Yantrik-Purpose=an app whose program is not on this machine\n", "utf-8")
        (apps_dir / "org.example.Wrapped.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Wrapped\nExec=/usr/bin/wrapped-wrapper\n"
            "TryExec=/bin/sh\nX-Yantrik-Surface=wrapped\n"
            "X-Yantrik-Purpose=an adapter's entry whose program is on this machine\n", "utf-8")
        declared = yos.declared_surfaces([str(apps_dir)])
        check("the .desktop keys are read the way the shell reads them",
              declared == [
                  {"id": "howdy", "title": "Howdy", "purpose": "say hello to someone, by name",
                   "aliases": ["hi", "greeter"], "entry": "org.example.Howdy"},
                  {"id": "quiet", "title": "Quiet", "purpose": "nothing to see here, very quietly",
                   "aliases": ["hush"], "entry": "org.example.Quiet"},
                  {"id": "system-monitor", "title": "System Monitor",
                   "purpose": "CPU, memory, disk and processes", "aliases": ["sysmonitor"],
                   "entry": "yantrik-system-monitor"},
                  {"id": "wrapped", "title": "Wrapped",
                   "purpose": "an adapter's entry whose program is on this machine",
                   "aliases": [], "entry": "org.example.Wrapped"}],
              declared)
        check("an entry whose TryExec names a program this machine does not have is not listed",
              not any(s["id"] == "ghost" for s in declared), declared)
        saved_dirs = yos.application_dirs
        yos.application_dirs = lambda: [str(apps_dir)]
        hello_up = FakeService(sockets / "app-howdy.sock", lambda _s, asked: (
            {"app": "howdy", "summary": "Howdy — nobody greeted yet", "state": {}, "actions": []}
            if asked["method"] == "app.describe" else {}))
        hello_up.start()
        services.append(hello_up)
        try:
            found = yos.socket_candidates("hi")
            check("an alias reaches the surface without the shell's link, from its .desktop file",
                  found == [str(sockets / "app-howdy.sock")], found)
            check("and so does what the app is called",
                  yos.socket_candidates("Howdy") == [str(sockets / "app-howdy.sock")],
                  yos.socket_candidates("Howdy"))
            out, err, code = run(lambda: yos.cmd_describe(["greeter"]))
            check("`describe greeter` answers as howdy", code is None and "nobody greeted" in out,
                  (out, err))
            out, err, code = run(lambda: yos.cmd_describe(["sysmonitor"]))
            check("an alias of a closed window reaches the service that answers for it",
                  code is None and "System — CPU" in out, (out, err))
            out, err, code = run(lambda: yos.cmd_describe(["hush"]))
            check("a closed app is named as closed, with its id, its purpose and how to open it",
                  code == 1 and "is closed" in err and "quiet: nothing to see here" in err
                  and "open_app name=quiet" in err and "no socket for" in err, err)

            print("yos ls, with a desktop that lists what it can open")
            listing = [
                {"name": "howdy", "opens": "app", "describe_as": "howdy", "running": True,
                 "for": "say hello to someone, by name", "aliases": ["hi", "greeter"]},
                {"name": "system-monitor", "opens": "app", "describe_as": "system-monitor",
                 "running": False, "for": "CPU, memory, disk and processes",
                 "aliases": ["sysmonitor"], "title": "System Monitor"},
                {"name": "files", "opens": "a screen of the desktop itself", "describe_as": "shell"},
            ]
            shell.reply = lambda _s, asked: (
                {"app": "shell", "summary": "Yantrik", "state": {"apps": listing}, "actions": []}
                if asked["method"] == "app.describe" else {"accepted": True, "settled": True})
            out, err, code = run(lambda: yos.cmd_ls([]))
            check("a closed app is listed as (closed), with what it is for and its other names",
                  re.search(r"^    system-monitor +\(closed\)  CPU, memory, disk and processes  "
                            r"\(also sysmonitor\)$", out, re.M) is not None, out)
            check("under the heading harnesses read it by",
                  "Can be opened with `act shell open_app name=<name>`" in out, out)
            check("and an open one is not listed as closed",
                  "howdy " in out and not re.search(r"^    howdy +\(closed\)", out, re.M), out)

            print("yos ls, with sockets that answer but are not surfaces (#190)")
            # The harness host, as it refuses anything but its own protocol; the store behind a
            # closed app, in the transport's words; and a service that is a surface.
            unknown = lambda code, words: (lambda _s, asked: {"__error__": {
                "code": code, "message": words % asked["method"]}})
            plumbing = [
                FakeService(sockets / "harness.sock", unknown(
                    -32000, "unknown method `%s`; this service speaks: harness.attach, harness.poll")),
                FakeService(sockets / "quiet.sock", unknown(-32601, "Unknown method: %s")),
                FakeService(sockets / "weather.sock", lambda _s, asked: (
                    {"app": "weather", "summary": "Weather — 21°C", "state": {}, "actions": []}
                    if asked["method"] == "app.describe" else {})),
            ]
            for svc in plumbing:
                svc.start()
                services.append(svc)
            out, err, code = run(lambda: yos.cmd_ls([]))
            line = next((l for l in out.splitlines() if l.startswith("Services answering:")), "")
            check("a service that answers describe is listed as answering", "weather" in line, out)
            check("one that does not is named nowhere in the listing a mind reads",
                  "harness" not in out and "quiet" not in line, out)
            out, err, code = run(lambda: yos.cmd_ls(["--all"]))
            check("--all names it, as the desktop's own and not a surface",
                  re.search(r"^The desktop's own, not surfaces \(they do not answer describe\): "
                            r".*harness", out, re.M) is not None, out)
            out, err, code = run(lambda: yos.cmd_describe(["harness"]))
            check("describing it anyway says what it is and where to look instead",
                  code == 1 and "plumbing, not an app or a service" in err and "`yos ls`" in err
                  and "harness.attach" in err, err)
            out, err, code = run(lambda: yos.cmd_describe(["hush"]))
            check("and the store behind a closed app says the app is closed and how to open it",
                  code == 1 and "hush is closed (quiet: nothing to see here" in err
                  and "open_app name=quiet" in err and "plumbing" not in err, err)

            print("yos ls, with no desktop answering")
            shell_reply_before = shell.reply
            shell.reply = lambda _s, asked: None  # accepts and says nothing: not a listing
            out, err, code = run(lambda: yos.cmd_ls([]))
            shell.reply = shell_reply_before
            check("closed apps are still listed, read from the .desktop files, and it says so",
                  "read from the .desktop files" in out
                  and re.search(r"^    system-monitor +\(closed\)  CPU, memory", out, re.M), out)
        finally:
            yos.application_dirs = saved_dirs
            shell.reply = asking_shell_reply

        print("the shell's name is the shell's: yos talks to app-shell only when yantrik-ui is "
              "what answers")
        accepted = yos.SHELL_BINARIES
        yos.SHELL_BINARIES = ("yantrik-ui",)
        shell.calls.clear()
        try:
            out, err, code = run(lambda: yos.cmd_describe(["shell"]))
        finally:
            yos.SHELL_BINARIES = accepted
        check("a process that is not yantrik-ui is refused, named, and sent nothing",
              code == 1 and "not the desktop's own yantrik-ui" in err
              and ("pid %d" % os.getpid()) in err and shell.calls == [], (err, shell.calls))
        check("while any other socket is talked to as before",
              yos.shell_peer_problem(None, str(sockets / "app-hello.sock")) is None, None)

        print("the socket chain is the transport's, in its order")
        saved = os.environ.get("XDG_RUNTIME_DIR")
        try:
            os.environ["XDG_RUNTIME_DIR"] = "/run/user/4242"
            chain = yos.socket_dirs()
            del os.environ["XDG_RUNTIME_DIR"]
            unset = yos.socket_dirs()
        finally:
            os.environ["XDG_RUNTIME_DIR"] = saved or tmp
        check("$XDG_RUNTIME_DIR/yantrik, then /run/yantrik, then /tmp/yantrik-<uid>",
              chain == ["/run/user/4242/yantrik", "/run/yantrik", "/tmp/yantrik-%d" % os.getuid()],
              chain)
        check("and with no XDG_RUNTIME_DIR, where logind puts it, in the first place",
              unset[0] == "/run/user/%d/yantrik" % os.getuid() and unset[1:] == chain[1:], unset)

        print("yos describe shell, carrying the large lists a busy desktop has (#319)")
        # The wire order is alphabetical — the shell's state is a serde_json map, which is a
        # BTreeMap — so a describe carrying large `agents` and `catalog` lists arrives with them
        # ahead of `clock`. Padded to about the sizes measured on a live desktop: 11.5 KB and
        # 4.5 KB. `sort_keys` makes the dict hold them in the wire's order, the way `json.loads`
        # of a real reply would.
        state = json.loads(json.dumps({
            "agents": [{"id": "pi:a%d" % i, "task": "x" * 100} for i in range(100)],
            "catalog": [{"role": "r%d" % i, "may": "y" * 100} for i in range(40)],
            "services": [{"id": "svc%d" % i, "state": "running"} for i in range(20)],
            "clock": {"date": "2026-09-24", "weekday": "Thursday", "time": "17:55",
                      "utc_offset": "-05:00", "zone": "America/Chicago"},
            "screen": "desktop",
            "windows": [{"title": "Notes", "app": "notes"}],
            "companion_online": False,
            "bond": "Partner-in-Crime",
            "cpu_percent": 10,
            "date": "Thu 24 Sep",
        }, sort_keys=True))
        reply_before = shell.reply
        shell.reply = lambda _s, asked: (
            {"app": "shell", "summary": "Yantrik — desktop screen", "state": state,
             "actions": []}
            if asked["method"] == "app.describe" else {"accepted": True, "settled": True})
        try:
            out, err, code = run(lambda: yos.cmd_describe(["shell", "--fold"]))
        finally:
            shell.reply = reply_before
        check("it answered, with nothing on stderr", code is None and err == "", (err, code))
        clock_at = out.find('"clock"')
        check("clock is within the first 300 bytes of what a mind reads, however big the lists",
              0 <= clock_at < 300, clock_at)
        order = [json.loads(m.group(1)) for m in
                 (re.match(r'^  ("(?:[^"\\]|\\.)*"): ', line) for line in out.splitlines())
                 if m]
        check("the small, high-value fields lead, in the fixed order",
              order[:4] == ["screen", "clock", "windows", "companion_online"], order)
        check("and the large lists come last, after every small field",
              order[-3:] == ["agents", "catalog", "services"], order)
        check("every key is printed exactly once", sorted(order) == sorted(state), order)
        check("and each value byte for byte as it arrived",
              '  "clock": %s' % json.dumps(state["clock"], ensure_ascii=False) in out
              and '  "agents": %s' % json.dumps(state["agents"], ensure_ascii=False) in out,
              out[:200])

        print("yos perception, with no desktop to ask")
        for svc in services:
            svc.close()
        services.clear()
        out, err, code = run(lambda: yos.cmd_perception([]))
        check("it is a sentence, not an exit code", code is None, "exit=%r" % (code,))
        check("that says the request was fine",
              "Nothing is wrong with the request" in out, out)
        check("and says the desktop could not start it",
              "could not start it" in out, out)
        check("with nothing on stderr", err == "", err)

        print("ensure_service, with no desktop to ask")
        out, err, code = run(lambda: yos.ensure_service("notifications"))
        check("still ends the command", code == 1, "exit=%r" % (code,))
        check("naming the service", "notifications" in err, err)
    finally:
        for svc in services:
            svc.close()
        shutil.rmtree(tmp, ignore_errors=True)

    print()
    if FAILURES:
        print("%d check(s) failed: %s" % (len(FAILURES), ", ".join(FAILURES)))
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
