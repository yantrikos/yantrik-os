#!/usr/bin/env python3
"""The bridge asks what the shell's table says is always asked (#1379).

    python3 deploy/yantrik-os/yos-mcp-selftest-always-asked.py

`guard_act` read only the action's own sentence, so a browser `go` (graded `standard`, no sentence)
ran in `ask` with no card and was refused by the browser's dispatch. It now also reads
`always_asked(app, action)`, the one table, shipped beside the bridge as `yos_always_asked.py`.
The desktop is faked at the seams `guard_act` reads (the shell's state, the card, the app's
description); the decision and the argv are the bridge's own.
"""
import importlib.util
import os
import subprocess
import sys
import tempfile
from importlib.machinery import SourceFileLoader

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
loader = SourceFileLoader("yosmcp_always_asked", os.path.join(HERE, "yos-mcp"))
spec = importlib.util.spec_from_loader("yosmcp_always_asked", loader)
mcp = importlib.util.module_from_spec(spec)
loader.exec_module(mcp)

FAILED = []
URL = "https://example.com/"
NO_SENTENCE = "Does a thing. It can be run at any time."


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else "  -- " + str(detail)))
    if not ok:
        FAILED.append(name)


def read(path):
    with open(path, "rb") as fh:
        return fh.read()


def run(mode, app, action, args, purpose=NO_SENTENCE):
    """guard_act in `mode`, on an app whose description says `purpose`. Returns what it returned
    and the cards put up (each the arguments `ask_the_person` was given) and the audit calls."""
    cards, calls = [], []

    def ask(app_, action_, grade, purpose_, bound, requester, must_ask=False):
        cards.append({"app": app_, "action": action_, "args": bound, "must_ask": must_ask})
        return True, "", "grant-1"

    def call(name, payload, timeout=None):
        calls.append(name)
        return {"status": "not_needed"}, None

    saved = {k: getattr(mcp, k) for k in ("shell_state", "mind_mode", "machine_ceiling", "standing_rules",
             "action_detail", "carries_private", "ask_the_person", "shell_call")}
    mcp.shell_state = lambda: {}
    mcp.mind_mode = lambda state: (mode, [])
    mcp.machine_ceiling = lambda state: "dangerous"
    mcp.standing_rules = lambda state: []
    mcp.action_detail = lambda a, b: ("standard", purpose)
    mcp.carries_private = lambda *a, **k: ""
    mcp.ask_the_person = ask
    mcp.shell_call = call
    try:
        out = mcp.guard_act({"app": app, "action": action, "args": dict(args)})
    finally:
        for k, v in saved.items():
            setattr(mcp, k, v)
    return out, cards, calls


for mode in ("ask", "auto", "bypass"):
    for name in ("browser", "chromium", "app-browser", "App Browser"):
        (refusal, note, audit, grant), cards, _ = run(mode, name, "go", {"url": URL})
        check("browser go as %r in %s asks, carries the address and returns the grant" % (name, mode),
              refusal is None and len(cards) == 1 and grant == "grant-1"
              and URL in str(cards[0]["args"]), (refusal, cards, grant))
    (refusal, _, _, grant), cards, _ = run(mode, "browser", "click", {"selector": "a"})
    check("browser click in %s asks too" % mode, len(cards) == 1 and grant == "grant-1", cards)
    (refusal, _, audit, grant), cards, _ = run(mode, "browser", "read", {})
    check("browser read in %s runs with no card" % mode, not cards and refusal is None and grant is None, cards)
    (refusal, _, audit, grant), cards, _ = run(mode, "shell", "send_mail", {"to": "x"})
    check("shell send_mail in %s asks although its description lacks the sentence" % mode,
          len(cards) == 1 and grant == "grant-1", cards)

(refusal, _, audit, grant), cards, _ = run("bypass_all", "browser", "go", {"url": URL})
check("bypass_all: browser go raises no card, runs and is audited",
      refusal is None and not cards and grant is None and audit and audit["mode"] == "bypass_all"
      and audit["action"] == "go", (refusal, cards, audit))
(refusal, _, audit, grant), cards, _ = run("bypass_all", "shell", "send_mail", {"to": "x"})
check("bypass_all: send_mail raises no card and is audited", not cards and refusal is None and audit, (cards, audit))
(refusal, _, audit, grant), cards, _ = run("bypass_all", "browser", "read", {})
check("bypass_all: a silent read is neither asked nor audited", not cards and audit is None, (cards, audit))
(refusal, _, _, _), cards, _ = run("ask", "browser", "go", {"url": URL}, purpose="x. " + mcp.ALWAYS_ASKS)
check("the description's own sentence still asks", len(cards) == 1, cards)

# One table: the copy the bridge ships with is the SDK's file, byte for byte.
check("yos_always_asked.py is a byte copy of the SDK's always_asked.py",
      read(os.path.join(HERE, "yos_always_asked.py"))
      == read(os.path.join(ROOT, "sdk", "python", "yantrik_surface", "always_asked.py")))
for script in ("build-release.sh", "deploy-to-vm.sh"):
    check("%s ships yos_always_asked.py" % script, "yos_always_asked.py" in open(os.path.join(HERE, script)).read())
check("the bridge holds no second list of the shell's always-asked actions",
      "allow_background_research" not in open(os.path.join(HERE, "yos-mcp")).read())
sys.exit(1 if FAILED else 0)
