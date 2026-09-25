#!/usr/bin/env python3
"""Self-test for yos-mcp's approval flow: the real script, against a fake desktop.

    python3 deploy/yantrik-os/yos-mcp-selftest.py

`yos-mcp` reaches the desktop by running `yos`, so the desktop can be faked by putting a script
called `yos` in front of it — which is what this does. Nothing here touches a real machine, a
socket or a window: `YOS_BIN` points at a scratch script that speaks `yos`'s own output format,
records every call, and answers the approval poll from a file the test writes.

Run it under Linux (the fake is an executable script with a shebang; WSL is fine). It takes a few
seconds: two of the cases wait for an approval that never comes.

What it is actually checking, in one line each:

  * a `standard` action still runs with nobody asked;
  * a `sensitive` action asks, and what the card is bound to is what the app will be run with —
    including the type the CLI will read the value as, which is the thing that would silently
    drift apart; an id the app publishes as `string` stays "3" all the way to the app, and a
    parameter it publishes as a number still arrives as one;
  * granted / denied / no-answer / above-the-machine-ceiling / no-shell each produce a distinct
    message, and only the first of them runs anything;
  * a grant whose arguments do not match is refused and nothing runs;
  * each of the four modes does what `design/mind-modes-2026-09-21.md` says it does — including
    that `bypass` still cannot pass the machine ceiling and `plan` refuses browser writes;
  * in `auto`, an action whose own published purpose says it cannot be undone is asked about
    exactly as a `dangerous` one is, the mind is told why, and no session rule covers it;
  * `YOS_MCP_MAX_PERMISSION` can only make things stricter than the desktop's mode;
  * an unreadable desktop falls back to `ask` and says so, rather than assuming anything;
  * an action nobody was asked about is reported to the shell's audit action, with its outcome;
  * a card on the person's screen does not stop the bridge answering anything else — the whole
    of the 22 September hang — and a poll that fails is retried, logged, and never throws away a
    question somebody is still looking at;
  * that this bridge's copy of the decision table still agrees with the shell's, on every
    combination of mode, grade, machine ceiling, session rule, browser tool and harness cap —
    read from `mind-mode-vectors.json`, which the shell's own tests generate;
  * and a bridge started with an agent token (YANTRIK_AGENT_TOKEN): every act carries the token
    through `yos`'s environment and it is on no command line, argument, card, audit line or
    answer; `terminal.run` goes to `shell.agent_run` with no window raised; the command tools are
    listed only with a token, keep the shell's grades, and are given as long as their wait. And
    without a token, all of that is exactly as it was;
  * the other-agent tools — new_agent, send_to_agent, stop_agent, read_agent — are listed only with
    a token, keep the shell's grades (new_agent asks in `ask`), pass the shell's own argument names
    and nothing else, answer in the shell's sentences, never carry the token, and reading another
    agent's session taints this one as reading private state does;
  * `hand_off` — a role from the agent catalog — is listed only with a token, asks like new_agent,
    passes only the shell's names, is given as long as its wait (and the harness's client allows
    it), taints the session when it waited for the role's answer, and an agent held to a role's
    reach hears the reach's refusal as a policy answer and is never shown asking the person for an
    act outside it, and a closed app its reach names says the role may open it (#195);
  * the shell's describe carries `clock` as an object — date, weekday, time, UTC offset and
    zone name — and it reaches a mind through os_describe untouched, so learning the day never
    has to go through a sensitive `agent_run date` again (#207);
  * os_describe names the apps this machine declares in their .desktop files, with what each is
    for, and no list of its own; os_apps says closed apps are listed; and the bridge reads the
    keys exactly as `yos` does;
  * and os_apps, run through the real `yos` against sockets, offers no socket that does not
    answer describe — the harness host a Red team once described (#190) — and os_describe on one
    anyway says what it is and where to look.
"""

import hashlib
import importlib.util
import io
import json
import os
import pathlib
import socket
import stat
import sys
import tempfile
import threading
import time
from importlib.machinery import SourceFileLoader

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = HERE / "yos-mcp"

# The fake `yos`. It mirrors the real one's argument parsing (each value read against the type
# the app published for that parameter, falling back to JSON where nothing was said) and its
# printing (`result` as indent-2 JSON after the header), because those two details are exactly
# what yos-mcp reads back.
FAKE_YOS = r'''#!/usr/bin/env python3
import json, os, re, sys, time

STATE = os.environ["FAKE_YOS_STATE"]

def load():
    with open(STATE) as fh:
        return json.load(fh)

def save(s):
    # Written beside and renamed over. This was `open(STATE, "w")`, which truncates the file
    # and THEN writes it, while the test reads the same file from another process: on a loaded
    # machine a read landed in between and the whole selftest died on a JSONDecodeError about
    # an empty file. It runs in CI now, where a loaded machine is the normal case.
    tmp = STATE + ".tmp%d" % os.getpid()
    with open(tmp, "w") as fh:
        json.dump(s, fh)
    os.replace(tmp, STATE)

def parse_args(pairs, types):
    # `yos.read_value`, which keeps a value bound for a `string` parameter as the text it
    # arrived as. The fake has to read arguments the way the real CLI does, or the bridge's
    # prediction of what the app will receive would be checked against something else.
    out = {}
    for pair in pairs:
        key, value = pair.split("=", 1)
        try:
            parsed = json.loads(value)
        except ValueError:
            out[key] = value
            continue
        if isinstance(parsed, str) or types.get(key) != "string":
            out[key] = parsed
        else:
            out[key] = value
    return out

def declared(target, action):
    """The types this fake desktop publishes for one action's arguments."""
    types, seen = {}, None
    text = {"calendar": DESCRIBE_CALENDAR, "shell": SHELL_ACTIONS, "terminal": DESCRIBE_TERMINAL}
    for line in text.get(target, "").splitlines():
        head = re.match(r"^\s*act:\s*(\w+)\(", line)
        if head:
            seen = head.group(1)
            continue
        arg = re.match(r"^ {8,}(\w+)\??:\s*(\S+)\s*(.*)", line)
        if arg and seen == action:
            kind = arg.group(2)
            if kind == "one" and arg.group(3).startswith("of "):
                # An enum renders as `mode: one of a | b`, but the real CLI reads its type off
                # the JSON describe, where an enum is published as "string" — so the fake has
                # to read one as a string too, or the bridge's predictions get checked against
                # a behaviour no real desktop has.
                kind = "string"
            types[arg.group(1)] = kind
    return types

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))

def envelope(result):
    print("fake desktop")
    print("accepted: True, settled: True")
    print("revision: deadbeef")
    print(json.dumps(result, indent=2))
    print("(state omitted)")

def die(msg):
    sys.stderr.write("yos: " + msg + "\n")
    raise SystemExit(1)

DESCRIBE_CALENDAR = """Calendar - September 2026
revision: c0ffee
{
  "events": [{"id": "evt-3", "title": "Dentist"}]
}
  act: list_events()  [safe, settles on return]
       List the events currently in view.
  act: add_event(title, date, duration_min?)  [standard, settles on return]
       Add an event to the calendar.
         title: string - what it is
         date: string - YYYY-MM-DD
         duration_min?: number - how long it runs, in minutes
  act: move_event(id, date)  [sensitive, settles on return]
       Move an event to another day. Move it back to undo it.
         id: string - the event's id, as list_events reports it
         date: string - YYYY-MM-DD
  act: delete_event(id)  [sensitive, settles on return]
       Delete an event from the calendar. It is not recoverable.
         id: string - the event's id, as list_events reports it
  act: repeat_event(id, mode)  [sensitive, settles on return]
       Set how an event repeats. The rule it had, if any, is replaced.
         id: string - the event's id, as list_events reports it
         mode: one of 1 | true | weekly - the repeat rule, as the calendar spells it
"""
# The shell's own actions, as `describe shell` lists them: opening an app, and an agent's terminal
# (`agent_run` and `agent_input` sensitive, `agent_job` and `agent_kill` standard, as the shell
# grades them in control_agent_terminal.rs).
SHELL_ACTIONS = """  act: open_app(name)  [standard, settles later]
       Launch an app, or focus it if it is already running.
  act: agent_run(command, cwd?, wait?)  [sensitive, settles later]
       Run one command line in a fresh terminal of your own, in your pane.
         command: string - one command line, as it would be typed
         cwd?: string - where to run it
         wait?: number - seconds to wait before answering running
  act: agent_job(job, wait?)  [standard, settles later]
       Wait for one of your commands that answered running.
         job: string - the job id agent_run answered with
         wait?: number - seconds to wait
  act: agent_input(job, text)  [sensitive, settles later]
       Type into one of your running commands.
         job: string - the job id agent_run answered with
         text: string - the exact characters to send
  act: agent_kill(job)  [standard, settles later]
       Stop one of your commands.
         job: string - the job id agent_run answered with
  act: new_agent(mind, task)  [sensitive, settles on return]
       Start another agent and give it a task.
         mind: string - which mind
         task: string - what it is to do
  act: send_to_agent(agent, text)  [standard, settles on return]
       Say more to an agent you started.
         agent: string - the agent
         text: string - what to say to it
  act: stop_agent(agent)  [standard, settles on return]
       Stop an agent you started.
         agent: string - the agent
  act: read_agent(agent, last?)  [safe, settles on return]
       Read an agent's recent turns as text.
         agent: string - the agent
         last?: number - how many of its latest turns
  act: hand_off(role, task, context?, wait_seconds?)  [sensitive, settles on return]
       Hand a piece of work to a role from the agent catalog.
         role: string - the role's id or name
         task: string - what it is to do
         context?: string - what it should read first
         wait_seconds?: number - seconds to wait for its answer
"""
DESCRIBE_TERMINAL = """Terminal - 1 tab
revision: 7e57
{
  "tabs": 1
}
  act: run(command)  [sensitive, settles later]
       Type a command line into the active shell and press Return.
         command: string - one command line
"""

# Two `sensitive` actions, and the difference between them is the sentence under the signature.
# `move_event` is the routine sensitive surface `auto` exists for; `delete_event` says it cannot
# be undone, so `auto` asks about it anyway. Before 21 September 2026 there was only the second
# one here, and every "auto runs it quietly" case in this file was written against an action the
# desktop should have been asking about.

argv = sys.argv[1:]
state = load()

if argv[:1] == ["describe"]:
    target = argv[1]
    # Every describe is recorded with its whole command line, so the test can tell the READER's
    # form (`--fold`, for a mind paying by the token) from the one the bridge parses for itself
    # (plain, because a folded family has no purpose lines and the card needs one).
    state.setdefault("describes", []).append(argv)
    save(state)
    if target == "shell":
        if state.get("shell_down"):
            die("shell is not open.")
        body = {
            "screen": "desktop",
            # What time the shell says it is, in full (#207): the read that keeps a mind off
            # `agent_run date`, which is sensitive and raised an approval card just to learn
            # the day. An object under `clock` — a key of its own sorts past the cut in the
            # condensed description minds read, and nothing read `clock` as the string it was.
            "clock": {
                "date": "2026-09-23",
                "weekday": "Wednesday",
                "time": "11:55",
                "utc_offset": "-05:00",
                "zone": "America/Chicago",
            },
            "pending_approvals": [],
            # What the desktop says about who is answering. The last-resort source for the name
            # on an approval card, when the MCP client sent no clientInfo.
            "minds": [
                {"id": "builtin", "name": "Yantrik Mind", "answering": False},
                {"id": "hermes", "name": "Hermes Agent", "answering": True},
            ],
            "mind_audit_recent": [],
        }
        # A desktop with no `mode` key at all is the older-shell case: the bridge must fall back
        # to `ask` and say it did, rather than guessing something looser out of a missing field.
        if "mode" in state:
            body["mind_mode"] = {
                "mode": state["mode"],
                "previous": "ask",
                "bypass_expires_in_secs": None,
                "session_rules": state.get("rules", []),
            }
        if state.get("machine_ceiling"):
            body["tool_permission"] = state["machine_ceiling"]
        if "--fold" in argv:
            # What `render_state` does: the same JSON, one top-level key per line, and the
            # shell's `apps` table left out. Parsed from the first `{` exactly as before.
            body.pop("apps", None)
        print("Yantrik - desktop screen")
        print("revision: c0ffee")
        print(json.dumps(body, indent=2))
        sys.stdout.write(SHELL_ACTIONS)
        raise SystemExit(0)
    if target == "calendar":
        sys.stdout.write(DESCRIBE_CALENDAR)
        raise SystemExit(0)
    if target == "terminal" and state.get("terminal_open"):
        sys.stdout.write(DESCRIBE_TERMINAL)
        raise SystemExit(0)
    if target in (state.get("no_socket_for") or []):
        # A declared app whose window is closed, in the real `yos`'s words — including the
        # "(no socket for ...)" the bridge keys its own way-forward sentence on (#195).
        die("%s is closed. Open it first: act shell open_app name=%s — then describe %s. "
            "(no socket for %r yet)" % (target, target, target, target))
    die("%s is not open." % target)

if argv[:1] == ["web"]:
    state.setdefault("web", []).append(argv)
    save(state)
    envelope({"navigated": True})
    raise SystemExit(0)

if argv[:1] == ["act"]:
    target, action = argv[1], argv[2]
    # What the app's own dispatch does now (issue #116): a `--grant` is spent — bound to the
    # app, the action and the exact arguments — before anything runs, and the call is refused
    # in the shell's words if it does not hold. `--no-ask` is the bridge telling yos not to
    # raise a card of its own; this fake has no card to raise. Both are recorded, because the
    # bridge carrying them is half of what this file checks.
    rest = argv[3:]
    # Where an agent token travelled, for every act: the environment `yos` reads it from (and the
    # real `yos` sends it beside the arguments), and the whole command line, which any user on the
    # machine can read and which must therefore never carry it.
    state.setdefault("carried", []).append({"action": "%s.%s" % (target, action),
                                            "env_token": os.environ.get("YANTRIK_AGENT_TOKEN"),
                                            "argv": argv})
    save(state)
    grant = None
    if "--grant" in rest:
        at = rest.index("--grant")
        grant = rest[at + 1]
        rest = rest[:at] + rest[at + 2:]
    no_ask = "--no-ask" in rest
    rest = [a for a in rest if a != "--no-ask"]
    args = parse_args(rest, declared(target, action))
    # An agent started as a catalog role, asking about or acting outside its reach: the shell and
    # the app's own dispatch refuse it in the reach's words (yantrik_ipc_transport::reach) — before
    # any grant is spent, as the dispatch does.
    reach = state.get("reach")
    if reach and target == "shell" and action == "request_approval":
        die("shell.app.act refused: REACH: %s.%s is graded `%s`, above the Reviewer's `safe` ceiling, "
            "so it was not run, and nobody was asked. `deepseek:c-role1` is the Reviewer, which may "
            "touch editor, documents and notes, at most `safe`." % (args.get("app"), args.get("action"), args.get("grade")))
    if reach and target != "shell":
        die("%s.app.act refused: REACH: %s.%s is outside the Reviewer's reach, so it was not run. "
            "`deepseek:c-role1` is the Reviewer, which may touch editor, documents and notes, at most "
            "`safe`." % (target, target, action))

    if grant is not None and target != "shell":
        def spent_refusal(why):
            die("%s.app.act refused: GRANT: `%s` does not authorise %s.%s — %s Nothing was run; "
                "a grant covers one action, once, with the arguments the person was shown."
                % (target, grant, target, action, why))
        if grant not in state.get("ids", {}):
            spent_refusal("no approval request `%s`." % grant)
        if state.get("answer") != "granted":
            spent_refusal("`%s` is %s." % (grant, state.get("answer")))
        if grant in state.get("spent", []):
            spent_refusal("`%s` was already used." % grant)
        want = state["ids"][grant]
        got = canonical(args)
        if want != got:
            spent_refusal("`%s` was approved with arguments %s, and this call carries %s. "
                          "Nothing was authorised." % (grant, want, got))
        if target != state["requests"][int(grant.split("-")[1]) - 1].get("app"):
            spent_refusal("wrong app.")
        state.setdefault("spent", []).append(grant)
        save(state)

    if target == "shell" and action == "request_approval":
        if state.get("shell_down"):
            die("shell.app.act refused: the shell is gone")
        state.setdefault("requests", []).append(args)
        rid = "appr-%d" % len(state["requests"])
        state.setdefault("ids", {})[rid] = canonical(args.get("args_json", {}))
        save(state)
        envelope({"request_id": rid, "status": "pending", "expires_in_secs": 120})
        raise SystemExit(0)

    if target == "shell" and action == "approval_status":
        rid = args["request_id"]
        # A desktop that is briefly unwell, which a card already on somebody's screen has to
        # survive. `poll_fails` refuses the next few polls; `poll_hang` makes every poll outlast
        # whatever the bridge gives it. Both are what the live failure looked like from here.
        state["polls"] = state.get("polls", 0) + 1
        if state.get("poll_fails", 0) > 0:
            state["poll_fails"] -= 1
            save(state)
            die("shell.app.act refused: the desktop is busy")
        save(state)
        if state.get("poll_hang"):
            time.sleep(state["poll_hang"])
        answer = state.get("answer", "pending")
        if rid in state.get("spent", []):
            answer = "consumed"
        envelope({"request_id": rid, "status": answer})
        raise SystemExit(0)

    if target == "shell" and action == "consume_approval":
        rid = args["request_id"]
        if state.get("answer") != "granted":
            die("shell.app.act refused: `%s` is %s" % (rid, state.get("answer")))
        if rid in state.get("spent", []):
            die("shell.app.act refused: `%s` was already used." % rid)
        want = state.get("ids", {}).get(rid)
        got = canonical(args.get("args_json", {}))
        if want != got:
            die("shell.app.act refused: `%s` was approved with arguments %s and this call "
                "carries %s. Nothing was authorised." % (rid, want, got))
        if args.get("app") != state["requests"][int(rid.split("-")[1]) - 1].get("app"):
            die("shell.app.act refused: wrong app. Nothing was authorised.")
        state.setdefault("spent", []).append(rid)
        save(state)
        envelope({"request_id": rid, "consumed": True})
        raise SystemExit(0)

    if target == "shell" and action == "record_unasked_action":
        if state.get("shell_down"):
            die("shell.app.act refused: the shell is gone")
        state.setdefault("audited", []).append(args)
        save(state)
        envelope({"recorded": "%s.%s" % (args.get("app"), args.get("action"))})
        raise SystemExit(0)

    state.setdefault("acted", []).append({"app": target, "action": action, "args": args,
                                          "grant": grant, "no_ask": no_ask})
    save(state)
    if target == "shell" and action.startswith("agent_"):
        # The shell's answer about a command, in the shape `RunAnswer::to_json` gives it.
        # `agent_running` answers as a command still going when its wait ran out; `echo_token`
        # prints whatever token the call carried, which is how a test catches the bridge
        # repeating one.
        answer = {"job": args.get("job", "job-1a2b"), "agent": "pi:c-7f3a91",
                  "command": args.get("command", "ls"), "cwd": "/home/me", "elapsed_ms": 1200,
                  "tail": "hi", "tail_clipped": False, "output_bytes": 3, "truncated_bytes": 0}
        if state.get("agent_running"):
            answer.update(running=True, waiting_for_input=False, elapsed_ms=120000,
                          tail="working", next="still running: `agent_job` waits for it again")
        else:
            answer.update(running=False, exit_code=0, cwd_after="/tmp")
        if state.get("echo_token"):
            answer["tail"] = "token=%s" % os.environ.get("YANTRIK_AGENT_TOKEN")
        envelope(answer)
        raise SystemExit(0)
    if target == "shell" and action in ("new_agent", "send_to_agent", "stop_agent", "read_agent"):
        # The shell's answers, in the shapes `control_agents.rs` gives them: a sentence under
        # `said`, and for read_agent the session as text.
        child = args.get("agent", "pi:c-child1")
        answer = {"agent": child}
        if action == "new_agent":
            answer.update(mind=args.get("mind"), parent="pi:c-7f3a91", state="thinking",
                          said="Started `pi:c-child1` on pi, started by `pi:c-7f3a91`. It has none "
                               "of your grants.")
        elif action == "send_to_agent":
            answer.update(sent=True, said="Sent to `%s`; it is working on it now." % child)
        elif action == "stop_agent":
            answer.update(stopped=True, commands_killed=1, said="Stopped `%s`, and killed 1 command." % child)
        else:
            answer.update(state="done", turns=1,
                          text="%s · pi · \"write the changelog\" · done.\n\n── turn 1 ── asked: write "
                               "the changelog\nDone: 12 entries.\n[verified call] agent_run command=\"git "
                               "log\" — ok · exit 0" % child)
        if state.get("echo_token"):
            answer["said"] = answer["text"] = "token=%s" % os.environ.get("YANTRIK_AGENT_TOKEN")
        envelope(answer)
        raise SystemExit(0)
    if target == "shell" and action == "hand_off":
        # The shell's answer, in the shape `control_agents::Handed::answer` gives it.
        role = "the Reviewer (`deepseek:c-role1`, on deepseek)"
        answer = {"agent": "deepseek:c-role1", "role": "reviewer", "role_name": "Reviewer",
                  "mind": "deepseek", "reach": "editor, documents and notes · at most safe",
                  "said": "Handed to %s. It works on its own, in its own pane, within its reach." % role}
        if args.get("wait_seconds"):
            answer.update(done=True, ok=True, answer="Verdict — fix first.",
                          said="The Reviewer (`deepseek:c-role1`, on deepseek) answered:\n\nVerdict — fix first.")
        if state.get("echo_token"):
            answer["said"] = "token=%s" % os.environ.get("YANTRIK_AGENT_TOKEN")
        envelope(answer)
        raise SystemExit(0)
    envelope({"done": True})
    raise SystemExit(0)

die("unknown command %r" % argv)
'''


def load_mcp(fake, state_path, ceiling="standard", requester="", follow=False, wait=4, token=None):
    """A fresh copy of the real yos-mcp, pointed at the fake desktop.

    Reloaded per case because the module reads its ceiling and its wait out of the environment
    at import time, which is right for a server started once per session and inconvenient here.

    `token` is the agent token the harness would have started the bridge with, or None for a
    bridge that runs for nobody in particular — every case before 22.
    """
    os.environ["YOS_BIN"] = str(fake)
    os.environ["FAKE_YOS_STATE"] = str(state_path)
    if token is None:
        os.environ.pop("YANTRIK_AGENT_TOKEN", None)
    else:
        os.environ["YANTRIK_AGENT_TOKEN"] = token
    # `None` means the harness set no cap at all, which is the ordinary case and the one where
    # the desktop's own mode decides alone. An empty or absent variable and a set one are
    # genuinely different to the bridge, so the test has to be able to produce both.
    if ceiling is None:
        os.environ.pop("YOS_MCP_MAX_PERMISSION", None)
    else:
        os.environ["YOS_MCP_MAX_PERMISSION"] = ceiling
    os.environ["YOS_MCP_REQUESTER"] = requester
    # Off for every case but its own: bringing an app forward is one more `act` on the fake
    # desktop, and the cases below count exactly what ran.
    os.environ["YOS_MCP_FOLLOW"] = "1" if follow else "0"
    # Short, because several cases below wait the whole thing out. The shell's own 120s request
    # lifetime is not involved: the fake answers from a file.
    os.environ["YOS_MCP_APPROVAL_WAIT"] = str(wait)
    loader = SourceFileLoader("yosmcp_under_test", str(SOURCE))
    spec = importlib.util.spec_from_loader("yosmcp_under_test", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    # Not a production knob: the poll interval only decides how often the fake is asked.
    module.APPROVAL_POLL = 0.05
    return module


failures = []


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else "  -- " + str(detail)))
    if not ok:
        failures.append(name)


def act(module, app, action, args):
    return module.run_tool(module.BY_NAME["os_act"], {"app": app, "action": action, "args": args})


def serve(path, reply):
    """A unix socket that answers each JSON-RPC line with `reply(request)` — a dict holding
    `result` or `error` — for the cases that run the real `yos` rather than the fake. A connection
    that sends nothing is `yos`'s liveness probe, and is dropped."""
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(path))
    listener.listen(8)

    def run():
        while True:
            try:
                conn, _ = listener.accept()
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
                    if buf.strip():
                        asked = json.loads(buf)
                        answer = dict(reply(asked), jsonrpc="2.0", id=asked.get("id"))
                        conn.sendall((json.dumps(answer) + "\n").encode())
                except (OSError, ValueError):
                    continue

    threading.Thread(target=run, daemon=True).start()
    return listener


def case(tmp, name, answer="granted", machine_ceiling="sensitive", shell_down=False,
         ceiling="standard", requester="", mode="ask", rules=None, no_mode=False,
         poll_fails=0, poll_hang=0, wait=4, token=None, follow=False, **desktop):
    """A scratch desktop in a known mood, and a yos-mcp pointed at it.

    `no_mode` publishes a shell that says nothing about its mode — an older desktop, or one
    answering from a version that predates them. The bridge has to fall back to `ask`.

    `poll_fails` and `poll_hang` are how the desktop misbehaves once a card is UP: the first few
    approval polls refused, or every one of them slower than the bridge's budget for it.

    `token` starts the bridge as one of the person's agents. Anything else in `desktop` goes into
    the fake's state as it stands (`terminal_open`, `agent_running`, `echo_token`).
    """
    state_path = tmp / (name + ".json")
    body = {
        "answer": answer,
        "machine_ceiling": machine_ceiling,
        "shell_down": shell_down,
        "rules": rules or [],
        "poll_fails": poll_fails,
        "poll_hang": poll_hang,
    }
    body.update(desktop)
    if not no_mode:
        body["mode"] = mode
    fake = tmp / "yos"
    state_path.write_text(json.dumps(body), encoding="utf-8")
    module = load_mcp(fake, state_path, ceiling=ceiling, requester=requester, wait=wait,
                      token=token, follow=follow)
    return module, state_path


def handshake(module, name, version):
    """Drive a real MCP `initialize` through the server, as a client would.

    Through `main()` rather than by poking `_CLIENT`, because the thing being tested is that the
    handshake's `clientInfo` is read at all — that is precisely what was being thrown away.
    """
    message = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"clientInfo": {"name": name, "version": version}},
    })
    saved_in, saved_out = sys.stdin, sys.stdout
    sys.stdin = io.StringIO(message + "\n")
    sys.stdout = io.StringIO()
    try:
        module.main()
    finally:
        sys.stdin, sys.stdout = saved_in, saved_out


class Client:
    """A client on the other end of the bridge's stdio, for the questions about WHEN.

    `handshake` drives `main()` over a StringIO, which is enough to ask what a reply says. This
    asks when it arrives, over real pipes and in real time: a server that will not read its next
    request until the last one is answered looks exactly like one that will, right up until
    something is kept waiting. That is the whole of the 22 September defect — a card on the
    person's screen made the bridge deaf, and the client killed it for being dead.

    The process's own streams are swapped for the pipes while this is up, so nothing may print
    between `Client(...)` and `close()`; collect what you learned and check it afterwards.
    """

    def __init__(self, module):
        self.replies = []
        self.handed = 0
        self.arrived = threading.Event()
        self.noise = io.StringIO()
        client_read, server_write = os.pipe()
        server_read, client_write = os.pipe()
        self.to_server = os.fdopen(client_write, "w")
        self.from_server = os.fdopen(client_read, "r")
        self.server_in = os.fdopen(server_read, "r")
        self.server_out = os.fdopen(server_write, "w")
        self.saved = (sys.stdin, sys.stdout, sys.stderr)
        sys.stdin, sys.stdout, sys.stderr = self.server_in, self.server_out, self.noise
        self.server = threading.Thread(target=module.main, daemon=True)
        self.server.start()
        self.reader = threading.Thread(target=self._drain, daemon=True)
        self.reader.start()

    def _drain(self):
        for line in self.from_server:
            line = line.strip()
            if not line:
                continue
            try:
                self.replies.append(json.loads(line))
            except ValueError:
                continue
            self.arrived.set()

    def send(self, msg_id, method, **params):
        self.to_server.write(json.dumps({"jsonrpc": "2.0", "id": msg_id, "method": method,
                                         "params": params}) + "\n")
        self.to_server.flush()

    def take(self, timeout):
        """The next reply the server has not handed over yet, or None if it does not come."""
        deadline = time.monotonic() + timeout
        while True:
            if len(self.replies) > self.handed:
                self.handed += 1
                return self.replies[self.handed - 1]
            left = deadline - time.monotonic()
            if left <= 0:
                return None
            self.arrived.clear()
            self.arrived.wait(left)

    def close(self):
        """Close the client's end, let the server finish, and put the streams back."""
        self.to_server.close()
        self.server.join(60)
        sys.stdin, sys.stdout, sys.stderr = self.saved
        self.server_out.close()
        self.reader.join(5)
        return self.noise.getvalue()


def served(module, *messages):
    """Drive `main()` over a whole session of messages and hand back its replies, by id.

    Through the real read loop, so what is listed and what a tools/call returns — `_meta` and
    all — is what a client would receive. `main()` finishes every call before it returns.
    """
    lines = [json.dumps(dict({"jsonrpc": "2.0"}, **m)) for m in messages]
    saved_in, saved_out, saved_err = sys.stdin, sys.stdout, sys.stderr
    sys.stdin, sys.stdout, sys.stderr = io.StringIO("\n".join(lines) + "\n"), io.StringIO(), io.StringIO()
    try:
        module.main()
        out, err = sys.stdout.getvalue(), sys.stderr.getvalue()
    finally:
        sys.stdin, sys.stdout, sys.stderr = saved_in, saved_out, saved_err
    replies = {}
    for line in out.splitlines():
        if line.strip():
            reply = json.loads(line)
            replies[reply.get("id")] = reply
    return replies, out, err


def act_aloud(module, app, action, args):
    """`act`, with whatever the bridge said to stderr while it ran. Returns (text, error, log)."""
    saved = sys.stderr
    sys.stderr = io.StringIO()
    try:
        text, is_error = act(module, app, action, args)
        return text, is_error, sys.stderr.getvalue()
    finally:
        sys.stderr = saved


def poll_failures(noise):
    """The bridge's own account of the polls that did not come back."""
    return [line for line in noise.splitlines() if "approval poll failed" in line]


def read(state_path):
    # The fake writes atomically now; the retry is for the filesystem, not for the fake — a
    # rename is atomic on Linux and merely quick on a Windows-backed mount.
    for attempt in range(20):
        try:
            return json.loads(state_path.read_text(encoding="utf-8"))
        except (ValueError, OSError):
            if attempt == 19:
                raise
            time.sleep(0.05)


with tempfile.TemporaryDirectory() as d:
    tmp = pathlib.Path(d)
    fake = tmp / "yos"
    fake.write_text(FAKE_YOS, encoding="utf-8")
    fake.chmod(fake.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

    # 1. Below the ceiling: nothing is asked, and the action runs.
    module, state = case(tmp, "below", answer="granted")
    text, is_error = act(module, "calendar", "add_event", {"title": "Dentist", "date": "2026-10-02"})
    s = read(state)
    check("a standard action is not put in front of anybody", not s.get("requests"), s)
    check("a standard action runs", not is_error and len(s.get("acted", [])) == 1, text)
    check("and carries no grant, because none was minted",
          (s.get("acted") or [{}])[0].get("grant") is None, s.get("acted"))

    # 2. Above the ceiling and allowed: asked, bound, spent, run.
    module, state = case(tmp, "granted", answer="granted")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    req = (s.get("requests") or [{}])[0]
    check("a sensitive action asks the person", len(s.get("requests", [])) == 1, s)
    check("the card names the action and its grade",
          req.get("app") == "calendar" and req.get("action") == "delete_event"
          and req.get("grade") == "sensitive", req)
    check("the card carries the app's own sentence about the action",
          "not recoverable" in (str(req.get("purpose")) or "").lower(), req)
    check("with no client name, the card falls back to the mind the desktop says is answering",
          req.get("requester") == "Hermes Agent", req)
    check("the card is bound to the exact arguments",
          req.get("args_json") == {"id": "evt-3"}, req)
    check("the grant is spent exactly once", s.get("spent") == ["appr-1"], s)
    check("and only then does the action run",
          [a["action"] for a in s.get("acted", [])] == ["delete_event"], s)
    # Issue #116: the app's own dispatch spends the grant, so the bridge has to CARRY it — and
    # has to tell `yos` not to raise a card of its own, or a refusal would put up a second card
    # with nobody's name on it and a wait longer than this call's budget.
    check("the grant rides on the call, for the app's dispatch to spend",
          (s.get("acted") or [{}])[0].get("grant") == "appr-1", s.get("acted"))
    check("and yos is told the bridge does its own asking",
          all(a.get("no_ask") for a in s.get("acted", [])), s.get("acted"))
    check("the answer says a person allowed it",
          not is_error and "allowed this once" in text, text)

    # 3. The argument the app will receive is the argument the person approved — and for a
    # parameter the app publishes as text, that is the text.
    #
    # `yos act` used to parse every value back with json.loads, so a model sending {"id": "3"}
    # meant the app received 3. On 22 September that was how `dismiss id=67` reached the
    # notifications service as a number and came back "missing `id`". Values are read against
    # the published type now, on both sides of this boundary: whatever the CLI will send, the
    # card has to say and the grant has to bind, or the person approved one thing and the
    # machine did another.
    module, state = case(tmp, "coercion", answer="granted")
    text, is_error = act(module, "calendar", "delete_event", {"id": "3"})
    s = read(state)
    req = (s.get("requests") or [{}])[0]
    check("an id the app publishes as text is not turned into a number",
          req.get("args_json") == {"id": "3"}, req)
    check("and the call carries the same value",
          s.get("acted", [{}])[0].get("args") == {"id": "3"}, s.get("acted"))
    check("so the grant matches and the action runs", not is_error, text)

    # And the other half: a parameter the app publishes as a number is still a number, or the
    # rule would simply have moved the same failure to `duration_min=30`.
    module, state = case(tmp, "coercion-number", answer="granted")
    wanted = {"title": "Call", "date": "2026-10-02", "duration_min": 15}
    text, is_error = act(module, "calendar", "add_event", dict(wanted))
    s = read(state)
    check("a parameter the app publishes as a number arrives as one",
          s.get("acted", [{}])[0].get("args") == wanted, s.get("acted"))
    check("and the bridge predicts exactly that",
          module.effective_args({"app": "calendar", "action": "add_event",
                                 "args": dict(wanted)}) == wanted,
          module.effective_args({"app": "calendar", "action": "add_event",
                                 "args": dict(wanted)}))

    # 3b. The bridge's reading of a `key=value` value and the CLI's are one reading.
    #
    # This bridge predicts what `yos` will send so that the card, the grant and the call bind
    # the same bytes. It reaches `yos` by running it and cannot import it, so the rule is
    # written twice — and two copies drift silently in the direction nobody tests. Both are
    # driven through the same table here.
    module, _ = case(tmp, "read-value", ceiling=None)
    yos_loader = SourceFileLoader("yos_under_test", str(HERE / "yos"))
    yos_spec = importlib.util.spec_from_loader("yos_under_test", yos_loader)
    yos_module = importlib.util.module_from_spec(yos_spec)
    yos_loader.exec_module(yos_module)
    table = [("67", "string"), ("67", "number"), ("67", None), ('"67"', "string"),
             ("true", "boolean"), ("true", "string"), ("true", None),
             ("hello world", "string"), ("hello world", None), ("", "string"),
             ('{"a": 1}', "string"), ('{"a": 1}', "object"), ("null", "string"),
             ("2026-10-02", "string"), ("-3.5", "number"), ("[1, 2]", "string")]
    drifted = ["%r as %s: the CLI reads %r, this bridge %r"
               % (t, d, yos_module.read_value(t, d), module.read_value(t, d))
               for t, d in table if module.read_value(t, d) != yos_module.read_value(t, d)]
    check("the bridge reads a value exactly as the CLI will", not drifted, drifted)
    check("and that reading is the one the issue asked for",
          yos_module.read_value("67", "string") == "67"
          and yos_module.read_value("67", "number") == 67
          and yos_module.read_value('"67"', "number") == "67", None)

    # 3c. An enum value that looks like JSON stays the string the app published.
    #
    # An enum is published as `{"type": "string", "enum": [...]}`, and the CLI reads that JSON,
    # so `mode=1` reaches the app as the text "1". The bridge reads the RENDERED describe, where
    # the same parameter is spelled `mode: one of 1 | true | weekly` — words, not "string" — and
    # used to hand those words to `read_value`, which parsed "1" as a number and "true" as a
    # boolean: the card bound a value the app was never sent, and the grant with it.
    module, state = case(tmp, "coercion-enum", answer="granted")
    check("the bridge reads an enum parameter as the string the CLI reads it as",
          module.action_parameters("calendar", "repeat_event") == {"id": "string", "mode": "string"},
          module.action_parameters("calendar", "repeat_event"))
    text, is_error = act(module, "calendar", "repeat_event", {"id": "evt-3", "mode": "1"})
    s = read(state)
    req = (s.get("requests") or [{}])[0]
    check("an enum value that looks like a number is bound to the card as text",
          req.get("args_json") == {"id": "evt-3", "mode": "1"}, req)
    check("and reaches the app as the same text, so the grant matches",
          not is_error and (s.get("acted") or [{}])[0].get("args") == {"id": "evt-3", "mode": "1"},
          (text, s.get("acted")))
    text, is_error = act(module, "calendar", "repeat_event", {"id": "evt-3", "mode": "true"})
    s = read(state)
    check("and one that looks like a boolean stays a string too",
          not is_error and (s.get("acted") or [{}])[-1].get("args") == {"id": "evt-3", "mode": "true"},
          (text, s.get("acted")))

    # 4. Denied: nothing runs, and the mind is told not to ask again.
    module, state = case(tmp, "denied", answer="denied")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("a denial runs nothing", not s.get("acted"), s)
    # An answer, not a failure: unflagged, and REFUSED in its first word (see `run_tool`). A
    # client that counts isError results takes three of them as a dead server.
    check("a denial is an answer, and says REFUSED first", not is_error and text.startswith("REFUSED"), text)
    check("a denial says the person said no",
          "said no" in text and "do not ask again" in text.lower(), text)

    # 5. Nobody answered: nothing runs, and it does not read as a fault.
    module, state = case(tmp, "silent", answer="pending")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("an unanswered request runs nothing", not s.get("acted"), s)
    check("an unanswered request says the person did not answer",
          "did not answer" in text, text)
    check("an unanswered request does not say the machine failed",
          "timed out" not in text and "failed" not in text, text)

    # 6. Above the MACHINE's ceiling: the person is not asked at all.
    module, state = case(tmp, "machine", answer="granted", machine_ceiling="standard")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("nothing above the machine's own ceiling is put to the person",
          not s.get("requests"), s)
    check("and nothing runs", not s.get("acted"), s)
    check("the refusal names the machine's standing policy",
          "tool_permission" in text and "NOT asked" in text, text)

    # 7. No shell: say so, run nothing.
    module, state = case(tmp, "noshell", answer="granted", shell_down=True)
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("no shell means nothing runs", not s.get("acted"), s)
    check("no shell says the person could not be asked",
          "could not be asked" in text and "Nothing was run" in text, text)

    # 8. A grant that does not match what is being run authorises nothing.
    #
    # Driven by making the call carry different arguments from the card, which is the
    # argument-swap an attacker would attempt: get one thing approved, run another. The check
    # is the app's own dispatch's now (issue #116) — the fake models it — and the bridge's part
    # is to report the refusal as "did not go through" rather than as a fault.
    module, state = case(tmp, "swap", answer="granted")
    # The call's argv, not `act_pairs`: that also feeds the card, and a swap that moved both
    # sides together would be no swap at all.
    module.BY_NAME["os_act"]["argv"] = lambda a: ["act", a["app"], a["action"], "id=evt-99"]
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("a swapped argument spends no grant", not s.get("spent"), s)
    check("a swapped argument runs nothing", not s.get("acted"), s)
    check("a swapped argument is reported as not gone through",
          not is_error and text.startswith("REFUSED") and "could not be spent" in text, text)

    # 8b. And a refusal from the app's dispatch when the bridge asked nobody — the desktop's
    # mode as the app read it disagreed with what this bridge read — is a policy answer in the
    # app's own words, not "failed (exit 1)".
    module, state = case(tmp, "app-refuses", mode="auto", ceiling=None)
    module.BY_NAME["os_act"]["argv"] = lambda a: (
        ["act", a["app"], a["action"], "id=evt-3", "date=2026-10-09", "--grant", "never-minted"])
    text, is_error = act(module, "calendar", "move_event", {"id": "evt-3", "date": "2026-10-09"})
    s = read(state)
    check("an app's own GRANT refusal runs nothing", not s.get("acted"), s)
    check("and is reported as the desktop saying no, in its words",
          not is_error and text.startswith("REFUSED") and "GRANT:" in text
          and "failed (exit" not in text, text)

    # 9. The name on the card comes from the client's own handshake when it sent one.
    #
    # The first cards read "the mind on this desktop", which told the person nothing about who
    # wanted their calendar changed. Hermes declares itself on `initialize`; that is the name.
    module, state = case(tmp, "clientinfo", answer="granted")
    handshake(module, "Hermes Agent", "0.9.2")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    req = (read(state).get("requests") or [{}])[0]
    check("the card names the client that introduced itself on the MCP handshake",
          req.get("requester") == "Hermes Agent 0.9.2", req)

    # And an operator naming it explicitly outranks both.
    module, state = case(tmp, "override", answer="granted", requester="the tour script")
    handshake(module, "Hermes Agent", "0.9.2")
    act(module, "calendar", "delete_event", {"id": "evt-3"})
    req = (read(state).get("requests") or [{}])[0]
    check("an operator's YOS_MCP_REQUESTER outranks what the client called itself",
          req.get("requester") == "the tour script", req)

    # With nothing at all to go on it says so rather than inventing a name.
    module, _ = case(tmp, "anon", answer="granted")
    check("with no client name and no shell, the requester is honestly unnamed",
          module.requester_name(None) == "an unnamed caller", module.requester_name(None))

    # 10. The two clocks agree: this bridge must give up before the shell drops the request,
    # or it would report "no answer" for one the person had just allowed.
    module, _ = case(tmp, "clocks")
    check("the bridge waits less than the shell holds the request open",
          module.APPROVAL_WAIT < 120, module.APPROVAL_WAIT)
    check("a client is told how long one os_act can take",
          module.OS_ACT_MAX_SECONDS >= module.APPROVAL_WAIT + module.ACT_TIMEOUT,
          module.OS_ACT_MAX_SECONDS)

    # ── The modes ───────────────────────────────────────────────────────────────────────
    #
    # 11. Plan: reading is open, every change is refused, and the refusal asks for the plan
    # rather than reading as a fault. This is the mode a person picks when they want to see
    # what a mind INTENDS, so the one thing it must not do is sound broken.
    module, state = case(tmp, "plan-standard", mode="plan")
    text, is_error = act(module, "calendar", "add_event", {"title": "X", "date": "2026-10-02"})
    s = read(state)
    check("plan mode runs nothing, not even a standard action", not s.get("acted"), s)
    check("plan mode asks nobody", not s.get("requests"), s)
    check("plan mode says it is a setting and asks for the plan",
          "plan mode" in text and "WOULD do" in text and "not a failure" in text, text)

    module, state = case(tmp, "plan-safe", mode="plan")
    text, is_error = act(module, "calendar", "list_events", {})
    check("plan mode still runs a safe action", not is_error and read(state).get("acted"), text)

    # And the browser: reading a page is looking, typing into one is not.
    module, state = case(tmp, "plan-web", mode="plan")
    text, is_error = module.run_tool(module.BY_NAME["web_go"], {"url": "https://example.com/"})
    check("plan mode refuses a browser write",
          not is_error and text.startswith("REFUSED") and "plan mode" in text, text)
    check("and nothing reached the browser", not read(state).get("web"), read(state))
    text, is_error = module.run_tool(module.BY_NAME["web_text"], {})
    check("plan mode still lets the page be read", not is_error, text)

    # 12. Auto: a routine sensitive action runs unasked and is written down.
    #
    # `ceiling=None` from here on, because these cases are about the DESKTOP's mode and a
    # harness that sets no cap is the ordinary case. The cap gets its own cases at 15.
    module, state = case(tmp, "auto", mode="auto", answer="pending", ceiling=None)
    text, is_error = act(module, "calendar", "move_event", {"id": "evt-3", "date": "2026-10-09"})
    s = read(state)
    check("auto runs a sensitive action without asking", not s.get("requests"), s)
    check("and it actually runs", not is_error and len(s.get("acted", [])) == 1, text)
    check("an unasked run is reported to the shell's audit action",
          len(s.get("audited", [])) == 1, s.get("audited"))
    audited = (s.get("audited") or [{}])[0]
    check("the audit line carries the action, grade, mode, arguments and outcome",
          audited.get("app") == "calendar" and audited.get("action") == "move_event"
          and audited.get("grade") == "sensitive" and audited.get("mode") == "auto"
          and audited.get("args_json") == {"id": "evt-3", "date": "2026-10-09"}
          and audited.get("outcome") == "ok", audited)
    check("and the mind is told nobody was asked",
          "Nobody was asked" in text and "auto" in text, text)

    # 12b. And the defect this rule exists for: in `auto`, an action whose own published purpose
    # says it cannot be undone is asked about exactly as a `dangerous` one is.
    #
    # Found live on 21 September 2026. `calendar.delete_event` is graded `sensitive` and says
    # "It is not recoverable"; in `auto` it deleted an event with nobody asked, while the mode
    # menu was promising "You are still asked about the destructive ones". The grade ladder has
    # no rung for "cannot be undone" to sit on, so the app's own sentence decides too.
    module, state = case(tmp, "auto-unrecoverable", mode="auto", answer="granted", ceiling=None)
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("auto asks about an action the app says cannot be undone",
          len(s.get("requests", [])) == 1, s)
    check("and the card carries the sentence that caused it",
          "not recoverable" in str((s.get("requests") or [{}])[0].get("purpose")).lower(),
          s.get("requests"))
    check("and it runs only once the person has allowed it",
          not is_error and [a["action"] for a in s.get("acted", [])] == ["delete_event"], s)
    # The mind is told WHY, or the honest report it can make is "the desktop is in auto and it
    # asked me anyway", which reads as a fault and is the shape of a thing somebody works around.
    check("and the mind is told why it was asked in auto mode at all",
          "auto" in text and "cannot be undone" in text and "not a fault" in text, text)
    check("nothing was written into the unasked record, because somebody was asked",
          not s.get("audited"), s.get("audited"))

    # A `safe` action is never asked about, whatever its wording says. A read destroys nothing,
    # and a rule that turned looking into a card would be the fastest way to teach somebody that
    # cards are noise. (Nothing published on this OS today is both `safe` and matching; the
    # bridge's own predicate is what is being pinned here.)
    module, _ = case(tmp, "safe-wording", mode="auto", ceiling=None)
    check("a safe action is not asked about however its purpose is worded",
          module.decide("safe", "notes", "read", True, "auto", [], "dangerous") == ("run", False),
          module.decide("safe", "notes", "read", True, "auto", [], "dangerous"))

    # Bypass is the one mode this does not touch. It says "it does not ask" on a red
    # confirmation with a countdown, and a card after that would make the panel a lie — so the
    # action runs and the record is what the person gets instead.
    module, state = case(tmp, "bypass-unrecoverable", mode="bypass",
                         machine_ceiling="dangerous", ceiling=None)
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("bypass does not ask about it either, because bypass does not ask",
          not s.get("requests") and len(s.get("acted", [])) == 1, s)
    check("and it is written down instead", len(s.get("audited", [])) == 1, s.get("audited"))

    # And the two phrase lists are one list. The shell draws the card's red warning line and
    # refuses a session rule from `approvals::unrecoverable`; this decides whether there is a
    # card at all. Two readings of the same sentence that disagreed would be a desktop warning
    # about something it had already run.
    #
    # The list lives in the gate now (`gate::UNRECOVERABLE_PHRASES`), which the shell's
    # `approvals::unrecoverable` and every app's dispatch ask; the gate's tests publish it in
    # surface-vectors.json, so the two lists are compared as lists, in order.
    module, _ = case(tmp, "phrases", ceiling=None)
    rust = (HERE.parent.parent / "crates" / "yantrik-ipc-transport" / "src" / "gate.rs")
    body = rust.read_text(encoding="utf-8")
    start = body.index("pub const UNRECOVERABLE_PHRASES")
    in_rust = [p for p in module.UNRECOVERABLE_PHRASES
               if '"%s"' % p in body[start:body.index("];", start)]]
    check("every phrase this bridge matches on is one the gate matches on",
          len(in_rust) == len(module.UNRECOVERABLE_PHRASES),
          sorted(set(module.UNRECOVERABLE_PHRASES) - set(in_rust)))
    published = json.loads((HERE / "surface-vectors.json").read_text(encoding="utf-8"))
    check("and the list is the gate's, phrase for phrase and in order",
          list(module.UNRECOVERABLE_PHRASES) == published.get("phrases"),
          (module.UNRECOVERABLE_PHRASES, published.get("phrases")))
    misread = [v["purpose"] for v in published.get("purposes") or []
               if module.unrecoverable(v["purpose"]) != v["unrecoverable"]]
    check("and every sentence in the vectors is read the way the gate reads it",
          bool(published.get("purposes")) and not misread, misread)
    check("and the sentence Calendar actually publishes is one of them",
          module.unrecoverable("Take an event off the calendar. It is not recoverable")
          and not module.unrecoverable("Move a file or folder to recoverable Trash"), None)

    # 13. Bypass: even a dangerous action runs — but only up to the machine's own ceiling.
    module, state = case(tmp, "bypass", mode="bypass", machine_ceiling="dangerous", ceiling=None)
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("bypass asks nobody", not s.get("requests"), s)
    check("bypass runs it", not is_error and len(s.get("acted", [])) == 1, text)
    check("bypass writes it down anyway", len(s.get("audited", [])) == 1, s.get("audited"))

    module, state = case(tmp, "bypass-ceiling", mode="bypass", machine_ceiling="standard", ceiling=None)
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("bypass does NOT reach past the machine ceiling", not s.get("acted"), s)
    check("and the refusal names the standing policy, not the mode",
          "tool_permission" in text and "no mode changes it" in text, text)

    # 14. A session rule: the person said "stop asking me about this one".
    module, state = case(tmp, "rule", mode="ask", answer="pending", ceiling=None,
                         rules=[{"app": "calendar", "action": "move_event"}])
    text, is_error = act(module, "calendar", "move_event", {"id": "evt-9", "date": "2026-10-09"})
    s = read(state)
    check("a session rule covers the action with arguments nobody approved",
          not s.get("requests") and len(s.get("acted", [])) == 1, s)
    check("and it is recorded as a rule rather than as the mode",
          (s.get("audited") or [{}])[0].get("mode") == "rule", s.get("audited"))

    module, state = case(tmp, "rule-other", mode="ask", answer="pending",
                         rules=[{"app": "calendar", "action": "list_events"}])
    text, is_error = act(module, "calendar", "move_event", {"id": "evt-3", "date": "2026-10-09"})
    s = read(state)
    check("a rule for one action is not a rule for its neighbour",
          len(s.get("requests", [])) == 1 and not s.get("acted"), s)

    # 14b. And a rule NEVER covers an action the app says cannot be undone, in any mode.
    #
    # Two layers, and this is the second. The card refuses to OFFER one for such an action,
    # which is checked once, at the press; this is the table refusing to honour one, which is
    # checked on every call. They exist separately because an app can reword its own purpose
    # after a rule was made — and because the auto rule at 12b would be worthless if a rule the
    # card would never have made could answer the card it raises.
    for mode in ("ask", "auto"):
        module, state = case(tmp, "rule-unrecoverable-" + mode, mode=mode, answer="pending",
                             ceiling=None,
                             rules=[{"app": "calendar", "action": "delete_event"}])
        text, is_error = act(module, "calendar", "delete_event", {"id": "evt-9"})
        s = read(state)
        check("in `%s`, a rule does not cover what the app says cannot be undone" % mode,
              len(s.get("requests", [])) == 1 and not s.get("acted"), s)
        check("and nothing is recorded as having run under a rule (%s)" % mode,
              not s.get("audited"), s.get("audited"))

    # 15. YOS_MCP_MAX_PERMISSION can only ever be STRICTER than the desktop's mode.
    module, state = case(tmp, "cap-strict", mode="bypass", machine_ceiling="dangerous",
                         ceiling="standard", answer="pending")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("a stricter session cap turns a bypass run back into a question",
          len(s.get("requests", [])) == 1 and not s.get("acted"), s)

    module, state = case(tmp, "cap-loose", mode="ask", machine_ceiling="dangerous",
                         ceiling="dangerous", answer="pending")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("and a loose session cap cannot make an `ask` desktop stop asking",
          len(s.get("requests", [])) == 1 and not s.get("acted"), s)

    # 15b. With no cap set at all, the desktop's mode is the whole policy — and for a desktop in
    # `ask` mode that is exactly what the historical default of `standard` used to do, so an
    # existing deployment that simply stops setting the variable sees no change.
    module, state = case(tmp, "nocap", mode="ask", answer="pending", ceiling=None)
    text, is_error = act(module, "calendar", "add_event", {"title": "X", "date": "2026-10-02"})
    check("with no cap, an ask desktop still runs a standard action unasked",
          not is_error and not read(state).get("requests"), text)
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    check("and still asks about a sensitive one",
          len(read(state).get("requests", [])) == 1, read(state))

    # 15c. The taint rule is NOT a permission grade and no mode turns it off — not even bypass.
    #
    # A mode says how much the person trusts this mind; the taint says what this session has
    # already read. They are different questions, and a bypass that switched off the second one
    # would turn "do not ask me about things" into "carry my private state out to a web page".
    module, state = case(tmp, "taint-bypass", mode="bypass", machine_ceiling="dangerous",
                         ceiling=None)
    module.run_tool(module.BY_NAME["os_describe"], {"app": "calendar"})
    text, is_error = module.run_tool(module.BY_NAME["web_type"], {"ref": 1, "text": "secret"})
    check("bypass does not switch off the taint rule",
          not is_error and text.startswith("REFUSED") and "already read private state" in text, text)
    check("and nothing reached the browser", not read(state).get("web"), read(state))

    # 16. A desktop that will not say what mode it is in: fall back to `ask`, and say so.
    module, state = case(tmp, "nomode", no_mode=True, answer="pending")
    text, is_error = act(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("an unreadable mode still asks about a sensitive action",
          len(s.get("requests", [])) == 1, s)
    check("and the fallback is stated rather than assumed silently",
          "could not read the desktop's mind-mode" in text and "fell back to" in text, text)

    module, state = case(tmp, "nomode-standard", no_mode=True)
    text, is_error = act(module, "calendar", "add_event", {"title": "X", "date": "2026-10-02"})
    check("an unreadable mode does not block ordinary work",
          not is_error and read(state).get("acted"), text)

    # 17. The describe a mind reads is folded; the one the bridge parses for itself is not.
    #
    # `--fold` prints a large family of actions as signatures only, which is the whole saving —
    # and a folded family has no purpose lines, which is exactly what `action_detail` needs for
    # the card. The two must not converge.
    module, state = case(tmp, "fold")
    module.run_tool(module.BY_NAME["os_describe"], {"app": "calendar"})
    describes = read(state).get("describes") or []
    check("os_describe asks for the folded form",
          describes and describes[-1] == ["describe", "calendar", "--fold"], describes)

    module, state = case(tmp, "fold-actions")
    module.run_tool(module.BY_NAME["os_describe"], {"app": "calendar", "actions": "files_"})
    describes = read(state).get("describes") or []
    check("os_describe forwards an actions prefix",
          describes and describes[-1] == ["describe", "calendar", "--fold", "--actions", "files_"],
          describes)

    module, state = case(tmp, "fold-detail")
    module.action_detail("calendar", "delete_event")
    describes = read(state).get("describes") or []
    check("action_detail does NOT fold, because the card needs the purpose line",
          describes and describes[-1] == ["describe", "calendar"], describes)
    grade, purpose = module.action_detail("calendar", "delete_event")
    check("and it still reads the purpose the card shows",
          grade == "sensitive" and "not recoverable" in purpose.lower(), (grade, purpose))

    # 17b. The shell says what time it thinks it is (#207): `clock` as an object — date,
    # weekday, time, UTC offset and zone name — is what keeps a mind off `shell.agent_run
    # date`, which is graded sensitive and used to raise an approval card just to learn the
    # day. The bridge has to hand every key of it to the mind untouched, folded like the
    # rest of the state.
    module, state = case(tmp, "clock")
    text, is_error = module.run_tool(module.BY_NAME["os_describe"], {"app": "shell"})
    check("os_describe shell carries the clock the shell already knows, keys and all",
          not is_error
          and all(key in text for key in
                  ('"clock"', '"date"', '"weekday"', '"time"', '"utc_offset"', '"zone"'))
          and '"utc_offset": "-05:00"' in text
          and '"zone": "America/Chicago"' in text, text)

    # ── 18. The table, against the shell's own copy of it ───────────────────────────────
    #
    # `decide` in yos-mcp and `mind_mode::Modes::decide` in the shell are the same table
    # written twice. That is deliberate — the bridge deciding for itself costs one read of the
    # desktop per os_act instead of two — and the design note lists it as the top open item,
    # because two copies drift silently in the direction nobody tests.
    #
    # So the shell writes every combination out and this drives the bridge through all of them.
    # Changing `decide` on the Rust side without regenerating fails
    # `mind_mode_the_checked_in_vectors_are_what_decide_produces`; regenerating without changing
    # this side fails here. Neither can move alone.
    module, _ = case(tmp, "vectors")
    vectors_path = HERE / "mind-mode-vectors.json"
    try:
        document = json.loads(vectors_path.read_text(encoding="utf-8"))
        vectors = document.get("vectors") or []
    except (OSError, ValueError) as e:
        vectors = []
        check("the decision-table vectors are checked in", False, e)

    # A missing or emptied file must not pass by testing nothing, which is the usual way a
    # generated fixture quietly stops being a check.
    check("the decision-table vectors are checked in", len(vectors) > 100, len(vectors))
    named = set(v.get("expect") for v in vectors)
    check("every outcome they name is one this bridge can produce",
          named and named <= set(module.OUTCOMES), sorted(named - set(module.OUTCOMES)))

    drifted = []
    for vector in vectors:
        got = module.decide_outcome(vector)
        if got != vector.get("expect"):
            drifted.append("%s: the shell says %s, this bridge says %s"
                           % (vector.get("id"), vector.get("expect"), got))
    check("this bridge decides all %d of them the way the shell does" % len(vectors),
          not drifted,
          "\n     " + "\n     ".join(drifted[:12])
          + ("\n     (and %d more)" % (len(drifted) - 12) if len(drifted) > 12 else ""))

    # And the file covers what it says it covers. A vector set that had quietly lost its
    # bypass rows would agree with anything.
    check("the vectors cover all four modes",
          set(v.get("mode") for v in vectors) == {"plan", "ask", "auto", "bypass"},
          sorted(set(v.get("mode") for v in vectors)))
    check("every grade, and one this OS does not define",
          {"safe", "standard", "sensitive", "dangerous"} <= set(v.get("grade") for v in vectors)
          and any(v.get("expect") == "refuse_grade" for v in vectors), None)
    check("the browser tools and the harness cap as well as the shell's own table",
          set(v.get("layer") for v in vectors) == {"shell", "harness_cap", "browser"},
          sorted(set(v.get("layer") for v in vectors)))
    check("and each of the six outcomes actually occurs somewhere in them",
          named == set(module.OUTCOMES), sorted(set(module.OUTCOMES) - named))

    # The `unrecoverable` axis, and that it is a real axis rather than a field nobody varies.
    #
    # It was carried for a day as `recoverable`, expected to change nothing, and the file said
    # so. It decides the table now, so the check has to be the opposite one: both values are
    # present, and somewhere among them there is a pair identical in every other input whose
    # outcomes differ. A dimension that never changes an answer is a column, not a test.
    check("both values of `unrecoverable` are covered",
          {False, True} <= set(bool(v.get("unrecoverable")) for v in vectors),
          sorted(set(str(v.get("unrecoverable")) for v in vectors)))

    # Paired on the id with the `unrecoverable=` segment taken out, not on the fields: the two
    # halves of a pair are generated against DIFFERENT actions (`files.move` and
    # `calendar.delete_event`, so a vector reads like something a person could check on a real
    # machine), and their `rules` therefore differ in spelling while meaning the same shape.
    others = {}
    for v in vectors:
        key = "/".join(part for part in str(v.get("id")).split("/")
                       if not part.startswith("unrecoverable="))
        others.setdefault(key, {})[bool(v.get("unrecoverable"))] = v.get("expect")
    paired = [by for by in others.values() if len(by) == 2]
    check("every case is generated both ways, so the axis is a real one",
          len(paired) * 2 + 24 == len(vectors), (len(paired), len(vectors)))
    moved = [by for by in paired if by[False] != by[True]]
    check("and it changes the answer somewhere: %d cells turn on the app's own sentence"
          % len(moved), bool(moved), None)
    # The cell the defect was reported from, named rather than counted.
    auto_sensitive = [v for v in vectors
                      if v.get("layer") == "shell" and v.get("mode") == "auto"
                      and v.get("grade") == "sensitive" and v.get("ceiling") == "dangerous"
                      and not v.get("rules")]
    by_undo = {bool(v.get("unrecoverable")): v.get("expect") for v in auto_sensitive}
    check("auto runs a recoverable sensitive action and asks about one that cannot be undone",
          by_undo == {False: "run_logged", True: "ask"}, by_undo)

    # And the bridge agrees with every app's dispatch. `surface-vectors.json` is
    # `yantrik_ipc_transport::gate::decide` written out by the gate's own tests; each vector says
    # what a door that raises cards must do with the same inputs (`door`): ask exactly where the
    # dispatch would refuse for want of a grant, refuse where it refuses on the ceiling or the
    # machine is in plan. The bridge is such a door. If this and the mind-mode check above both
    # pass, the shell, the bridge and every app's dispatch tell a mind the same thing.
    module, _ = case(tmp, "surface-vectors")
    try:
        surface = json.loads((HERE / "surface-vectors.json").read_text(encoding="utf-8"))
        doors = surface.get("decide") or []
    except (OSError, ValueError) as e:
        doors = []
        check("the dispatch's vectors are checked in", False, e)
    check("the dispatch's vectors are checked in", len(doors) >= 640, len(doors))
    as_door = {"run": "run", "ask": "ask", "refuse_grade": "refuse", "refuse_ceiling": "refuse",
               "refuse_mode": "refuse"}
    drifted = []
    for v in doors:
        rules = [(v["app"], v["action"])] if v.get("session_rule") else []
        verdict, _ = module.decide(v["grade"], v["app"], v["action"],
                                   module.unrecoverable(v["purpose"]), v["mode"], rules,
                                   v["ceiling"])
        if as_door.get(verdict) != v.get("door"):
            drifted.append("%s: the dispatch says a door should %s, this bridge says %s"
                           % (v.get("id"), v.get("door"), verdict))
    check("this bridge does what every app's dispatch says a door should, on all %d" % len(doors),
          not drifted, "\n     " + "\n     ".join(drifted[:12]))
    # The bridge reaches the desktop only by running `yos`, which checks that the process behind
    # `app-shell.sock` is the shell before it writes anything there. That is only this bridge's
    # check too while it opens no socket of its own.
    source = SOURCE.read_text(encoding="utf-8")
    check("the bridge opens no socket itself, so yos's check of the shell's peer is its check",
          "socket.socket(" not in source and "AF_UNIX" not in source, None)
    check("and the one place the two differ is named in the file, not excused here",
          all(v.get("door") == "refuse" and v.get("outcome") == "allow" and v.get("mode") == "plan"
              for v in doors if v.get("note")) and any(v.get("note") for v in doors), None)

    # ── 19. A card on the screen does not make the bridge deaf ──────────────────────────
    #
    # 22 September 2026, and the whole of it. A `terminal.run` card went up at 06:48:18Z; the
    # bridge polled it every two seconds and, because it answered one request at a time, read
    # nothing else off its stdin while it did. 59 seconds in, Hermes' keepalive gave up on a
    # server that was merely busy, replaced the process, and the os_act died with it — so the
    # call never returned and Hermes waited out its own 300s timeout. The card was still on the
    # person's screen and nobody was left waiting for their answer.
    #
    # Driven through the real `main()` over real pipes, because nothing smaller can tell a server
    # that answers in order from one that answers at all.
    module, state = case(tmp, "concurrent", answer="pending", wait=4)
    client = Client(module)
    client.send(1, "tools/call", name="os_act",
                arguments={"app": "calendar", "action": "delete_event", "args": {"id": "evt-3"}})
    # Wait until the card is genuinely up, so the ping lands in the middle of the wait rather
    # than in front of it.
    card_up = False
    for _ in range(100):
        if read(state).get("requests"):
            card_up = True
            break
        time.sleep(0.05)
    client.send(2, "ping")
    ping = client.take(2.0)
    acted = client.take(20.0)
    noise = client.close()
    s = read(state)

    check("a card goes up before the ping is sent", card_up, s)
    check("the bridge answers a ping while it is waiting on a card",
          ping is not None and ping.get("id") == 2, ping)
    check("and a ping is answered as MCP defines it, not as an unknown method",
          ping is not None and ping.get("result") == {} and "error" not in ping, ping)
    check("the waiting call is still answered, after the ping",
          acted is not None and acted.get("id") == 1, acted)
    check("and it is the unanswered-card sentence, not a fault",
          acted is not None
          and "did not answer" in acted["result"]["content"][0]["text"], acted)
    check("nothing ran while nobody had answered", not s.get("acted"), s)

    # ── 20. A poll that fails does not throw the card away ──────────────────────────────
    #
    # The card is on somebody's screen and they are reaching for the mouse. One `yos` that takes
    # too long, one transient refusal, and the bridge used to return "the desktop stopped
    # answering" — abandoning a live question, and leaving behind a grant that nobody would ever
    # spend if they went on to press Allow.
    module, state = case(tmp, "poll-blips", answer="granted", poll_fails=3)
    text, is_error, noise = act_aloud(module, "calendar", "delete_event", {"id": "evt-3"})
    s = read(state)
    check("three failed polls do not abandon a card that is still up",
          not is_error and [a["action"] for a in s.get("acted", [])] == ["delete_event"], text)
    check("the grant is still spent exactly once", s.get("spent") == ["appr-1"], s)
    check("and the mind is told the person allowed it", "allowed this once" in text, text)
    check("every failed poll is written to stderr, with why",
          len(poll_failures(noise)) == 3 and "the desktop is busy" in noise,
          noise or "(nothing was logged)")

    # 20b. And a card nobody answers is still reported as unanswered, not as a desktop that
    # went away, when a poll failed somewhere in the middle of the wait.
    module, state = case(tmp, "poll-blip-silent", answer="pending", poll_fails=2, wait=2)
    started = time.monotonic()
    text, is_error, noise = act_aloud(module, "calendar", "delete_event", {"id": "evt-3"})
    elapsed = time.monotonic() - started
    check("a blip in the middle does not change what an unanswered card is called",
          "did not answer" in text and "stopped answering" not in text, text)
    check("and the wait still ends when it said it would (%.1fs of 2s)" % elapsed,
          elapsed < 2 + 1.5, elapsed)

    # ── 21. A desktop that never answers a poll at all ──────────────────────────────────
    #
    # Every poll slower than the budget the bridge has for it. Two things have to hold: the wait
    # ends when the mind was told it would — the poll's own subprocess timeout is cut to what is
    # left, or one slow poll carries the whole call past the deadline — and the ending says what
    # actually happened, which is NOT that the person did not answer. Nobody here knows that.
    module, state = case(tmp, "poll-silent", answer="granted", poll_hang=6, wait=2)
    module.SHELL_CALL_TIMEOUT = 6
    started = time.monotonic()
    text, is_error, noise = act_aloud(module, "calendar", "delete_event", {"id": "evt-3"})
    elapsed = time.monotonic() - started
    s = read(state)
    check("a wait nothing answers still ends on time (%.1fs of 2s)" % elapsed,
          elapsed < 2 + 1.5, elapsed)
    check("nothing runs on a guess", not s.get("acted") and not s.get("spent"), s)
    check("and the mind is told the desktop stopped answering, not that the person did not",
          text.startswith("REFUSED") and "stopped answering" in text and "never replied" in text
          and "away from the keyboard" not in text, text)
    check("with the reason on stderr", poll_failures(noise), noise or "(nothing was logged)")

    # The person can see what the mind is doing: one raise per change of app, never per action,
    # never for the shell, and never in the way of the action it follows.
    state = tmp / "follow.json"
    state.write_text(json.dumps({"answer": "granted", "machine_ceiling": "sensitive", "mode": "auto"}))
    module = load_mcp(fake, state, ceiling=None, follow=True)
    for title in ("One", "Two"):
        act(module, "calendar", "add_event", {"title": title, "date": "2026-10-02"})
    act(module, "shell", "open_app", {"name": "notes"})
    act(module, "calendar", "list_events", {})
    shown = [a["args"].get("name") for a in read(state).get("acted", []) if a.get("action") == "show_app"]
    check("an app is brought forward once when the mind moves to it, not once per action",
          shown == ["calendar"], shown)
    module._FOLLOWING[0] = "notes"   # the mind has been elsewhere since
    act(module, "calendar", "list_events", {})
    shown = [a["args"].get("name") for a in read(state).get("acted", []) if a.get("action") == "show_app"]
    check("and again when it comes back from another app", shown == ["calendar", "calendar"], shown)

    # ── 22. A bridge that runs as one of the person's agents ────────────────────────────
    #
    # design/agents-workspace-2026-09-23.md, decision 3. The harness starts this bridge with the
    # agent's token in YANTRIK_AGENT_TOKEN. Every act carries it beside the arguments (through
    # `yos`'s environment — never its command line, never the arguments, never the audit), a
    # command goes to the agent's own terminal rather than the person's, and the agent is offered
    # its terminal as tools of its own. Without a token none of that happens.
    TOKEN = "0123456789abcdef0123456789abcdef"
    LIST = {"id": 1, "method": "tools/list", "params": {}}
    COMMAND_TOOLS = ["run_command", "command_status", "command_input", "command_kill"]

    def leaks(state, *extra):
        """Every place the token must not be: argv, arguments, cards, audit lines, and `extra`."""
        s = read(state)
        places = {
            "a command line": [c["argv"] for c in s.get("carried", [])],
            "an action's arguments": [a.get("args") for a in s.get("acted", [])],
            "an approval card": s.get("requests", []),
            "the unasked-actions record": s.get("audited", []),
        }
        places.update({"what the mind was told (%d)" % n: text for n, text in enumerate(extra)})
        return sorted(where for where, what in places.items()
                      if TOKEN in json.dumps(what) or "--agent-token" in json.dumps(what))

    # 22a. No token: exactly as before. terminal.run is the person's Terminal, raised as ever.
    module, state = case(tmp, "notoken-terminal", answer="granted", ceiling=None,
                         terminal_open=True, follow=True)
    text, is_error = act(module, "terminal", "run", {"command": "ls"})
    s = read(state)
    check("without a token, terminal.run still types into the person's Terminal",
          not is_error and [(a["app"], a["action"]) for a in s.get("acted", [])
                            if a["action"] != "show_app"] == [("terminal", "run")], s.get("acted"))
    check("and the Terminal is brought forward as it always was",
          [a["args"].get("name") for a in s.get("acted", []) if a["action"] == "show_app"] == ["terminal"],
          s.get("acted"))
    check("and no act carries a token",
          s.get("carried") and all(c["env_token"] is None for c in s["carried"]), s.get("carried"))
    replies, _, _ = served(module, LIST, {"id": 2, "method": "tools/call",
                                          "params": {"name": "run_command", "arguments": {"command": "ls"}}})
    names = [t["name"] for t in replies[1]["result"]["tools"]]
    check("without a token the command tools are not listed",
          not set(COMMAND_TOOLS) & set(names) and "os_act" in names, names)
    check("and cannot be called", "no such tool" in json.dumps(replies[2].get("error")), replies[2])
    os_act_listed = [t for t in replies[1]["result"]["tools"] if t["name"] == "os_act"][0]
    check("and os_act is described exactly as it was",
          os_act_listed["description"] == module.BY_NAME["os_act"]["description"], None)

    # 22b. With a token, terminal.run is the agent's own terminal — asked about, carried and
    # recorded as what will actually run, `shell.agent_run` — and nothing is raised.
    module, state = case(tmp, "token-terminal", answer="granted", ceiling=None,
                         terminal_open=True, follow=True, token=TOKEN)
    text, is_error, noise = act_aloud(module, "terminal", "run", {"command": "ls -la"})
    s = read(state)
    req = (s.get("requests") or [{}])[0]
    check("with a token, terminal.run goes to the agent's own terminal",
          not is_error and [(a["app"], a["action"]) for a in s.get("acted", [])] == [("shell", "agent_run")]
          and s["acted"][0]["args"] == {"command": "ls -la"}, s.get("acted"))
    check("the card asks about shell.agent_run, sensitive, with the command and nothing else",
          (req.get("app"), req.get("action"), req.get("grade"), req.get("args_json"))
          == ("shell", "agent_run", "sensitive", {"command": "ls -la"}), req)
    check("the grant rides on the agent_run it was minted for",
          s["acted"][0].get("grant") == "appr-1" and s["acted"][0].get("no_ask"), s.get("acted"))
    check("no window is raised for it, and the Terminal is never even described",
          not any(a["action"] == "show_app" for a in s.get("acted", []))
          and not any(d[:2] == ["describe", "terminal"] for d in s.get("describes", [])),
          (s.get("acted"), s.get("describes")))
    check("every act — the card, its polls, the command — carries the token",
          s.get("carried") and all(c["env_token"] == TOKEN for c in s["carried"]),
          [(c["action"], c["env_token"]) for c in s.get("carried", [])])
    check("and it is nowhere a person or another user could read it", not leaks(state, text, noise),
          leaks(state, text, noise))
    check("the mind is told where it ran and how it ended",
          "not in the person's Terminal" in text and "exit code 0" in text and "allowed this once" in text,
          text)

    # 22c. The command tools, and their grades: run_command and command_input are sensitive, so
    # in `ask` they put a card up; command_status and command_kill are standard and do not.
    module, state = case(tmp, "token-grades", answer="granted", ceiling=None, token=TOKEN)
    for name, args in (("run_command", {"command": "make", "wait_seconds": 5}),
                       ("command_status", {"job": "job-1a2b", "wait_seconds": 0}),
                       ("command_input", {"job": "job-1a2b", "text": "y\n"}),
                       ("command_kill", {"job": "job-1a2b"})):
        module.call_tool(module.AGENT_BY_NAME[name], args)
    s = read(state)
    check("run_command and command_input ask; command_status and command_kill do not",
          [r.get("action") for r in s.get("requests", [])] == ["agent_run", "agent_input"],
          s.get("requests"))
    check("each runs as the shell's own action, with the shell's argument names",
          [(a["action"], a["args"]) for a in s.get("acted", [])] == [
              ("agent_run", {"command": "make", "wait": 5}), ("agent_job", {"job": "job-1a2b", "wait": 0}),
              ("agent_input", {"job": "job-1a2b", "text": "y\n"}), ("agent_kill", {"job": "job-1a2b"})],
          s.get("acted"))
    check("and the token travels with all of them and leaks into none",
          all(c["env_token"] == TOKEN for c in s.get("carried", [])) and not leaks(state), leaks(state))

    # 22d. In `auto` the command runs unasked and is written down — without the token.
    module, state = case(tmp, "token-auto", mode="auto", ceiling=None, token=TOKEN)
    text, is_error, meta = module.call_tool(module.AGENT_BY_NAME["run_command"],
                                            {"command": "make", "cwd": "/tmp"})
    s = read(state)
    audited = (s.get("audited") or [{}])[0]
    check("in auto, run_command runs unasked and lands in the record",
          not s.get("requests") and (audited.get("app"), audited.get("action"), audited.get("args_json"))
          == ("shell", "agent_run", {"command": "make", "cwd": "/tmp"}), s)
    check("and the record never holds the token", not leaks(state, text), leaks(state, text))
    check("the mind reads how it ended, and a client gets the shell's own answer",
          text.startswith("Nobody was asked") and "exit code 0 after 1.2 s, in /tmp." in text
          and meta and meta.get("exit_code") == 0 and meta.get("job") == "job-1a2b", (text, meta))

    # 22e. A command still running when its wait ran out says so, and how to follow it up.
    module, state = case(tmp, "token-running", mode="auto", ceiling=None, token=TOKEN,
                         agent_running=True)
    text, is_error, meta = module.call_tool(module.AGENT_BY_NAME["run_command"], {"command": "make"})
    check("a command still going answers with its job and the tools to follow it",
          not is_error and "still running after 2m 00s (job job-1a2b)" in text
          and "command_status waits for it again" in text and "command_kill stops it" in text
          and meta.get("running") is True, text)

    # 22f. Listed, called through the real read loop, answered with `_meta`, and scrubbed.
    module, state = case(tmp, "token-served", mode="auto", ceiling=None, token=TOKEN, echo_token=True)
    replies, out, err = served(module, LIST, {"id": 2, "method": "tools/call", "params": {
        "name": "run_command", "arguments": {"command": "env"}}})
    listed = {t["name"]: t for t in replies[1]["result"]["tools"]}
    check("with a token the command tools are listed", set(COMMAND_TOOLS) <= set(listed), sorted(listed))
    check("and os_act says where terminal.run now goes",
          "terminal of your own" in listed["os_act"]["description"], listed["os_act"]["description"])
    called = replies[2]["result"]
    check("a command tool's answer carries the shell's account under _meta",
          called.get("_meta", {}).get("yantrik/command", {}).get("exit_code") == 0, called)
    check("and a token the desktop echoed is scrubbed from everything the bridge sends",
          TOKEN not in out and TOKEN not in err and "token=[agent token]" in out, out)

    # 22g. An `agent_token` a mind puts among the arguments is dropped, not sent: the one that
    # counts rides beside them, from the bridge's own environment.
    module, state = case(tmp, "token-forged", mode="auto", ceiling=None, token=TOKEN)
    act(module, "shell", "agent_run", {"command": "ls", "agent_token": "f" * 32})
    s = read(state)
    check("a token among the arguments is never sent, shown or recorded",
          s["acted"][0]["args"] == {"command": "ls"} and "f" * 32 not in json.dumps(s)
          and all(c["env_token"] == TOKEN for c in s["carried"]), s)

    # 22h. A command tool is given as long as the command it waits for — more than `yos` gives
    # the same act, which is more than the shell waits.
    yos_loader = SourceFileLoader("yos_timeouts", str(HERE / "yos"))
    yos_spec = importlib.util.spec_from_loader("yos_timeouts", yos_loader)
    yos_timeouts = importlib.util.module_from_spec(yos_spec)
    yos_loader.exec_module(yos_timeouts)
    short = [w for w in (None, 0, 5, 120, 600)
             if module.agent_timeout(w) <= yos_timeouts.act_timeout(
                 "agent_run", {} if w is None else {"wait": w})]
    check("each wait is given longer here than yos gives it", not short, short)
    # And the clients allow for all of it: the card's wait as shipped (110 s, not this file's 4)
    # and the command's. The harness library's MCP client is DeepSeek's; pi's extension uses the
    # same numbers (harnesses/tests/test_pi_extension.py).
    shipped = module.agent_call_max_seconds(600) - module.APPROVAL_WAIT + 110
    lib_loader = SourceFileLoader("harness_lib", str(HERE.parent.parent / "harnesses" / "lib" / "yantrik_harness.py"))
    lib_spec = importlib.util.spec_from_loader("harness_lib", lib_loader)
    harness_lib = importlib.util.module_from_spec(lib_spec)
    lib_loader.exec_module(harness_lib)
    allowed = harness_lib.mcp_timeout("run_command", {"wait_seconds": 600})
    check("and the harness's MCP client waits out the longest such call (%ds of %ds)"
          % (allowed, shipped), allowed >= shipped, (allowed, shipped))
    seen = {}
    real_run = module.subprocess.run

    def spy(argv, **kw):
        if argv[1:4] == ["act", "shell", "agent_run"]:
            seen["timeout"] = kw.get("timeout")
        return real_run(argv, **kw)

    module.subprocess.run = spy
    try:
        module.call_tool(module.AGENT_BY_NAME["run_command"], {"command": "ls", "wait_seconds": 600})
    finally:
        module.subprocess.run = real_run
    check("and run_command's wait reaches the act's own timeout",
          seen.get("timeout") == 630, seen)

    # ── 23. Handing work to another agent ───────────────────────────────────────────────
    #
    # design/agents-workspace-2026-09-23.md, decision 1: `new_agent`, `send_to_agent`,
    # `stop_agent` and `read_agent` are the shell's, offered to a mind only when it runs as one of
    # the person's agents. The shell's grades stand (new_agent sensitive, send and stop standard,
    # read safe), which agent is asking rides beside the arguments and nowhere else, and reading
    # another agent's session counts as reading private state.
    AGENTS_TOOLS = ["new_agent", "send_to_agent", "stop_agent", "read_agent"]
    CHILD = "pi:c-child1"

    # 23a. Without a token none of them is listed or callable.
    module, state = case(tmp, "agents-notoken", ceiling=None)
    replies, _, _ = served(module, LIST, {"id": 2, "method": "tools/call", "params": {
        "name": "new_agent", "arguments": {"mind": "pi", "task": "x"}}})
    names = [t["name"] for t in replies[1]["result"]["tools"]]
    check("without a token the other-agent tools are not listed", not set(AGENTS_TOOLS) & set(names), names)
    check("and new_agent cannot be called", "no such tool" in json.dumps(replies[2].get("error")), replies[2])
    check("and nothing reached the desktop", not read(state).get("acted"), read(state))

    # 23b. With a token, in `ask`: new_agent puts a card up — shell.new_agent, sensitive, the mind
    # and the task and nothing else — and the other three run unasked, each as the shell's own
    # action with the shell's argument names.
    module, state = case(tmp, "agents-grades", answer="granted", ceiling=None, token=TOKEN)
    told = {}
    for name, args in (("new_agent", {"mind": "pi", "task": "write the changelog"}),
                       ("send_to_agent", {"agent": CHILD, "text": "and the release notes"}),
                       ("stop_agent", {"agent": CHILD}),
                       ("read_agent", {"agent": CHILD, "last": 2})):
        told[name] = module.call_tool(module.AGENT_BY_NAME[name], args)
    s = read(state)
    reqs = s.get("requests", [])
    check("new_agent asks; send_to_agent, stop_agent and read_agent do not",
          [r.get("action") for r in reqs] == ["new_agent"], reqs)
    check("the card is shell.new_agent, sensitive, with the mind and the task alone",
          reqs and (reqs[0].get("app"), reqs[0].get("grade"), reqs[0].get("args_json"))
          == ("shell", "sensitive", {"mind": "pi", "task": "write the changelog"}), reqs)
    check("each runs as the shell's own action, with the shell's argument names",
          [(a["action"], a["args"]) for a in s.get("acted", [])] == [
              ("new_agent", {"mind": "pi", "task": "write the changelog"}),
              ("send_to_agent", {"agent": CHILD, "text": "and the release notes"}),
              ("stop_agent", {"agent": CHILD}),
              ("read_agent", {"agent": CHILD, "last": 2})], s.get("acted"))
    check("the grant rides on the new_agent it was minted for, and only there",
          [a.get("grant") for a in s.get("acted", [])] == ["appr-1", None, None, None], s.get("acted"))
    check("every one carries the token, and it leaks into none of them",
          all(c["env_token"] == TOKEN for c in s.get("carried", [])) and not leaks(state, *[t[0] for t in told.values()]),
          leaks(state, *[t[0] for t in told.values()]))
    check("a mind reads the shell's sentences, not its JSON",
          "Started `pi:c-child1`" in told["new_agent"][0] and "none of your grants" in told["new_agent"][0]
          and told["stop_agent"][0].startswith("Stopped `pi:c-child1`")
          and not any(t[0].lstrip().startswith("{") for t in told.values()),
          {k: v[0] for k, v in told.items()})
    check("read_agent answers with the session as text",
          "── turn 1 ── asked: write the changelog" in told["read_agent"][0]
          and "exit 0" in told["read_agent"][0] and not told["read_agent"][1], told["read_agent"])

    # 23c. Reading another agent's session is reading private state: putting data into a page is
    # refused afterwards, as it is after os_describe.
    module, state = case(tmp, "agents-taint", mode="auto", ceiling=None, token=TOKEN)
    text, is_error = module.run_tool(module.BY_NAME["web_type"], {"ref": 1, "text": "hello"})
    check("before reading another agent, typing into a page is not refused by the taint",
          not text.startswith("REFUSED"), text)
    module.call_tool(module.AGENT_BY_NAME["read_agent"], {"agent": CHILD})
    text, is_error = module.run_tool(module.BY_NAME["web_type"], {"ref": 1, "text": "secret"})
    check("after read_agent, it is", text.startswith("REFUSED") and "read_agent" in text, text)

    # 23d. In `auto`, new_agent runs unasked and is written down — without the token; a token a
    # mind puts among the arguments is dropped; and one the desktop echoes back is scrubbed.
    module, state = case(tmp, "agents-auto", mode="auto", ceiling=None, token=TOKEN, echo_token=True)
    replies, out, err = served(module, {"id": 2, "method": "tools/call", "params": {
        "name": "new_agent", "arguments": {"mind": "pi", "task": "tidy", "agent_token": "f" * 32}}})
    text = replies[2]["result"]["content"][0]["text"]
    s = read(state)
    audited = (s.get("audited") or [{}])[0]
    check("in auto, new_agent runs unasked and lands in the record, as the shell's action",
          not s.get("requests") and (audited.get("app"), audited.get("action"), audited.get("args_json"))
          == ("shell", "new_agent", {"mind": "pi", "task": "tidy"}), s)
    check("a token among the arguments is never sent, shown or recorded",
          s["acted"][0]["args"] == {"mind": "pi", "task": "tidy"} and "f" * 32 not in json.dumps(s), s)
    check("and a token the desktop echoed is scrubbed from everything the bridge sends",
          TOKEN not in out and TOKEN not in err and "token=[agent token]" in text and not leaks(state), text)
    # Only the shell's own argument names go through: nothing a model adds rides along — not a
    # parent it names for itself, not a grant.
    module, state = case(tmp, "agents-extra", mode="auto", ceiling=None, token=TOKEN)
    module.call_tool(module.AGENT_BY_NAME["stop_agent"],
                     {"agent": CHILD, "parent": "pi:c-7f3a91", "grant": "appr-1"})
    s = read(state)
    check("an argument the shell does not take is not sent",
          [a["args"] for a in s.get("acted", [])] == [{"agent": CHILD}], s.get("acted"))

    # ── 24. Handing work to a role from the agent catalog ───────────────────────────────
    #
    # design/desk-and-mind-2026-09-23.md, section 5: `hand_off` is the shell's, offered only with a
    # token like the other-agent tools, `sensitive` like new_agent, and it may wait for the role's
    # answer — so the call is given that wait, and what comes back counts as reading private state.
    # An agent started as a role is held to its reach on every door: a refusal in the reach's words
    # is a policy answer, not a fault, and the person is never asked about an act it cannot reach.

    # 24a. Without a token it is not listed or callable.
    module, state = case(tmp, "handoff-notoken", ceiling=None)
    replies, _, _ = served(module, LIST, {"id": 2, "method": "tools/call", "params": {
        "name": "hand_off", "arguments": {"role": "reviewer", "task": "x"}}})
    names = [t["name"] for t in replies[1]["result"]["tools"]]
    check("without a token hand_off is not listed", "hand_off" not in names, names)
    check("and cannot be called", "no such tool" in json.dumps(replies[2].get("error")), replies[2])
    check("and nothing reached the desktop", not read(state).get("acted"), read(state))

    # 24b. With a token, in `ask`: a card for shell.hand_off, sensitive, with the shell's own
    # argument names and nothing else; the grant rides on it; the mind reads the shell's sentence.
    module, state = case(tmp, "handoff-ask", answer="granted", ceiling=None, token=TOKEN)
    replies, _, _ = served(module, LIST)
    check("with a token hand_off is listed",
          "hand_off" in [t["name"] for t in replies[1]["result"]["tools"]], replies[1])
    text, is_error, _ = module.call_tool(module.AGENT_BY_NAME["hand_off"], {
        "role": "reviewer", "task": "review the change", "context": "diff --git a/x b/x",
        "mind": "pi", "agent_token": "f" * 32})
    s = read(state)
    reqs = s.get("requests", [])
    check("hand_off asks, as new_agent does: shell.hand_off, sensitive",
          [(r.get("app"), r.get("action"), r.get("grade")) for r in reqs] == [("shell", "hand_off", "sensitive")], reqs)
    check("with the role, the task and the context, and nothing a model added",
          reqs and reqs[0].get("args_json") == {"role": "reviewer", "task": "review the change",
                                                  "context": "diff --git a/x b/x"}
          and [a["args"] for a in s.get("acted", [])] == [reqs[0].get("args_json")], s)
    check("the grant rides on the hand_off it was minted for",
          [a.get("grant") for a in s.get("acted", [])] == ["appr-1"], s.get("acted"))
    check("the mind reads the shell's sentence, not its JSON",
          not is_error and "Handed to the Reviewer (`deepseek:c-role1`, on deepseek)" in text
          and "{" not in text.split("\n\n")[-1], text)
    check("and the token travels beside it and leaks into nothing",
          all(c["env_token"] == TOKEN for c in s.get("carried", [])) and not leaks(state, text)
          and "f" * 32 not in json.dumps(s), leaks(state, text))

    # 24c. Told to wait, the call is given that wait and a margin — more than `yos` gives the same
    # act, which is more than the shell waits — and the harness's client allows for all of it. Not
    # told to, it is an ordinary act.
    module, state = case(tmp, "handoff-wait", mode="auto", ceiling=None, token=TOKEN)
    seen = []
    real_run = module.subprocess.run

    def spy(argv, **kw):
        if argv[1:4] == ["act", "shell", "hand_off"]:
            seen.append(kw.get("timeout"))
        return real_run(argv, **kw)

    module.subprocess.run = spy
    try:
        waited, _, _ = module.call_tool(module.AGENT_BY_NAME["hand_off"],
                                        {"role": "chair", "task": "weigh them", "wait_seconds": 240})
        module.call_tool(module.AGENT_BY_NAME["hand_off"], {"role": "scribe", "task": "sum up"})
    finally:
        module.subprocess.run = real_run
    check("a hand_off told to wait is given its wait and a margin; one not told, an act's minute",
          seen == [270, module.ACT_TIMEOUT], seen)
    check("and a wait is longer here than yos gives it",
          all(module.hand_off_timeout(w) > yos_timeouts.act_timeout("hand_off", {"wait_seconds": w})
              for w in (5, 240, 600)), None)
    allowed = harness_lib.mcp_timeout("hand_off", {"wait_seconds": 600})
    longest = module.OS_ACT_MAX_SECONDS - module.ACT_TIMEOUT + module.hand_off_timeout(600) \
        - module.APPROVAL_WAIT + 110
    check("and the harness's MCP client waits out the longest hand_off (%ds of %ds)" % (allowed, longest),
          allowed >= longest, (allowed, longest))
    check("the role's answer comes back as what it said",
          "The Reviewer (`deepseek:c-role1`, on deepseek) answered:\n\nVerdict — fix first." in waited, waited)

    # 24d. What a role hands back after a wait is another session's work: typing into a page is
    # refused afterwards. A hand_off that did not wait read nothing.
    module, state = case(tmp, "handoff-taint", mode="auto", ceiling=None, token=TOKEN)
    module.call_tool(module.AGENT_BY_NAME["hand_off"], {"role": "scribe", "task": "sum up"})
    text, _ = module.run_tool(module.BY_NAME["web_type"], {"ref": 1, "text": "hello"})
    check("a hand_off that did not wait does not taint", not text.startswith("REFUSED"), text)
    module.call_tool(module.AGENT_BY_NAME["hand_off"], {"role": "chair", "task": "weigh", "wait_seconds": 30})
    text, _ = module.run_tool(module.BY_NAME["web_type"], {"ref": 1, "text": "secret"})
    check("one that waited for the role's answer does", text.startswith("REFUSED") and "hand_off" in text, text)

    # 24e. An agent held to a role's reach: an act its door refuses is a policy answer naming the
    # role, not a failure; and an act it would have to ask about is refused by the shell before any
    # card goes up, in the shell's words.
    module, state = case(tmp, "handoff-reach-auto", mode="auto", ceiling=None, token=TOKEN, reach=True)
    text, is_error = act(module, "calendar", "add_event", {"title": "x", "date": "2026-10-02"})
    check("a door's reach refusal is a policy answer naming the role, not a fault",
          not is_error and text.startswith("REFUSED") and "outside the Reviewer's reach" in text, (text, is_error))
    module, state = case(tmp, "handoff-reach-ask", mode="ask", ceiling=None, token=TOKEN, reach=True)
    text, is_error = act(module, "calendar", "move_event", {"id": "evt-3", "date": "2026-10-03"})
    s = read(state)
    check("an act outside the reach is never put to the person",
          not s.get("requests") and not s.get("acted") and not is_error, s)
    check("and the mind hears the reach's own words, not a ceiling it could raise",
          "above the Reviewer's `safe` ceiling" in text and "could not be asked" not in text, text)

    # 24f. In `auto`, hand_off runs unasked and is written down — without the token.
    module, state = case(tmp, "handoff-auto", mode="auto", ceiling=None, token=TOKEN)
    module.call_tool(module.AGENT_BY_NAME["hand_off"], {"role": "reviewer", "task": "tidy"})
    s = read(state)
    audited = (s.get("audited") or [{}])[0]
    check("in auto, hand_off runs unasked and lands in the record, without the token",
          not s.get("requests") and (audited.get("app"), audited.get("action"), audited.get("args_json"))
          == ("shell", "hand_off", {"role": "reviewer", "task": "tidy"}) and not leaks(state), s)

    # 24g. A closed app that is in a role's reach says the role may open it (#195). The Planner
    # was told "Open it first" and then refused for trying. The door now lets a reach open the
    # apps it names, and this sentence — where the role reads it — says so, but only when the
    # file the shell publishes puts the app in the reach. The bridge reads that file for the
    # wording alone; the door still decides.
    digest = hashlib.sha256(TOKEN.encode("utf-8")).hexdigest()
    home = tmp / "home"
    (home / ".config" / "yantrik").mkdir(parents=True)
    (home / ".config" / "yantrik" / "agent-reach.json").write_text(json.dumps({"agents": [
        {"token_sha256": digest, "agent": "deepseek:c-role1", "role": "reviewer",
         "name": "Reviewer", "surfaces": ["editor", "documents", "notes.read_*"],
         "ceiling": "safe"},
    ]}), encoding="utf-8")
    module, state = case(tmp, "closed-in-reach", token=TOKEN, no_socket_for=["notes", "terminal"])
    saved_home = os.environ.get("HOME")
    os.environ["HOME"] = str(home)
    try:
        told, failed = module.run_tool(module.BY_NAME["os_describe"], {"app": "notes"})
        other, _ = module.run_tool(module.BY_NAME["os_describe"], {"app": "terminal"})
    finally:
        if saved_home is None:
            os.environ.pop("HOME", None)
        else:
            os.environ["HOME"] = saved_home
    check("a closed app within the reach says the role may open it",
          failed and "Open it first" in told
          and "Your reach names notes, so you may open it." in told, told)
    check("and one the reach does not name promises nothing, and still shows the way",
          "Open it first" in other and "reach" not in other, other)

    # 25. os_describe names the apps this machine declares, from their .desktop files — anybody's
    # as well as ours — and names none of its own. os_apps says a closed app is listed.
    module, state = case(tmp, "surfaces")
    apps_dir = tmp / "applications"
    apps_dir.mkdir()
    (apps_dir / "org.example.Howdy.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Howdy\nExec=/usr/bin/howdy\n"
        "X-Yantrik-Surface=howdy\nX-Yantrik-Purpose=say hello to someone, by name\n"
        "X-Yantrik-Aliases=hi;greeter;shell\n", encoding="utf-8")
    (apps_dir / "yantrik-system-monitor.desktop").write_text(
        (HERE.parent.parent / "apps" / "desktop-files" / "yantrik-system-monitor.desktop")
        .read_text(encoding="utf-8"), encoding="utf-8")
    (apps_dir / "vim.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim %F\n", encoding="utf-8")
    # An adapter's entry for an app this machine does not have: its Exec looks fine, but its
    # TryExec names a program that is nowhere, so nothing may offer it (#214).
    (apps_dir / "org.example.Ghost.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Ghost\nExec=/usr/bin/ghost-wrapper\n"
        "TryExec=no-such-program-anywhere\nX-Yantrik-Surface=ghost\n"
        "X-Yantrik-Purpose=an app whose program is not on this machine\n", encoding="utf-8")
    text = module.os_describe_text([str(apps_dir)])
    check("os_describe names each declared app with what it is for",
          "'howdy' (say hello to someone, by name)" in text
          and "'system-monitor' (CPU, memory, disk and processes)" in text, text)
    check("and still offers the desktop itself", "'shell' (the desktop" in text, text)
    check("and an app that declares nothing is not offered", "vim" not in text.lower(), text)
    check("and neither is an adapter for an app this machine does not have",
          "ghost" not in text.lower(), text)
    bare = module.os_describe_text([str(tmp / "nowhere")])
    check("with nothing declared it names no app at all — the old list is gone",
          not any(("'%s'" % name) in bare
                  for name in ("system-monitor", "weather", "network", "notes", "calendar", "email")),
          bare)
    saved_dirs = module.application_dirs
    module.application_dirs = lambda: [str(apps_dir)]
    try:
        listed = module.listing(module.BY_NAME["os_describe"])["description"]
    finally:
        module.application_dirs = saved_dirs
    check("tools/list publishes that description, read when the tools are listed",
          "'howdy' (say hello" in listed, listed)
    check("os_apps says a closed app is listed, marked as closed",
          "(closed)" in module.BY_NAME["os_apps"]["description"], module.BY_NAME["os_apps"]["description"])
    # One reading of the keys, however many scripts carry it: the bridge's copy agrees with
    # yos's — including hiding an entry whose TryExec program is not on this machine (#214),
    # which is why Ghost is in neither reading and the count stays two.
    loader = SourceFileLoader("yos_for_surfaces", str(HERE / "yos"))
    spec = importlib.util.spec_from_loader("yos_for_surfaces", loader)
    real_yos = importlib.util.module_from_spec(spec)
    loader.exec_module(real_yos)
    theirs = [{k: s[k] for k in ("id", "purpose", "aliases")}
              for s in real_yos.declared_surfaces([str(apps_dir)])]
    ours = module.declared_surfaces([str(apps_dir)])
    check("the bridge reads the .desktop keys exactly as yos does", ours == theirs and len(ours) == 2,
          (ours, theirs))

    # 26. What os_apps offers a mind to describe answers describe. The first live catalog run's
    # Red team read `harness` off the services line, called os_describe on it and spent a failed
    # call learning it was not a surface (#190). Run through the real `yos`, against sockets: the
    # harness host refusing anything but its own protocol, and a service that is a surface.
    runtime = tmp / "runtime"
    (runtime / "yantrik").mkdir(parents=True)
    listeners = [
        serve(runtime / "yantrik" / "harness.sock", lambda asked: {"error": {
            "code": -32000, "message": "unknown method `%s`; this service speaks: harness.attach, "
                                       "harness.poll, harness.chunk" % asked["method"]}}),
        serve(runtime / "yantrik" / "weather.sock", lambda asked: {"result": {
            "app": "weather", "summary": "Weather — 21°C and clear", "state": {}, "actions": []}}),
    ]
    saved_env = {k: os.environ.get(k) for k in ("XDG_RUNTIME_DIR", "XDG_DATA_DIRS", "XDG_DATA_HOME")}
    os.environ.update({"XDG_RUNTIME_DIR": str(runtime), "XDG_DATA_DIRS": str(tmp / "no-apps"),
                       "XDG_DATA_HOME": str(tmp / "no-apps")})
    try:
        module = load_mcp(HERE / "yos", tmp / "real-yos.json")
        listing, listing_failed = module.run_tool(module.BY_NAME["os_apps"], {})
        described, describe_failed = module.run_tool(module.BY_NAME["os_describe"], {"app": "harness"})
    finally:
        for key, value in saved_env.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
        for listener in listeners:
            listener.close()
    services = next((l for l in listing.splitlines() if l.startswith("Services answering:")), "")
    check("os_apps, through the real yos, lists a service that answers describe as answering",
          not listing_failed and "weather" in services, listing)
    check("and names no socket that does not: the harness host is not offered to describe",
          "harness" not in listing, listing)
    check("os_describe on it anyway says what it is and where to look, not only that it failed",
          describe_failed and "plumbing, not an app or a service" in described
          and "yos ls" in described, described)

print()
if failures:
    print("%d failed: %s" % (len(failures), ", ".join(failures)))
    sys.exit(1)
print("all checks passed")
