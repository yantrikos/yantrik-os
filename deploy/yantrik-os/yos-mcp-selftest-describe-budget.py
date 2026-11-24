#!/usr/bin/env python3
"""A mind can look at the desktop and still send its next request (#1388).

    python3 deploy/yantrik-os/yos-mcp-selftest-describe-budget.py          # also prints the size table
    python3 deploy/yantrik-os/yos-mcp-selftest-describe-budget.py --table  # the table only

Two things are held. (1) The folded `describe shell` of a representative state is at most
DESCRIBE_SHELL_BUDGET bytes, and nothing the person relies on for safety was cut to get there.
(2) No tool result a mind reads is bigger than the bridge's cap, and one cut says so and how to ask
for the rest. The state is a FIXTURE in the shape of the shell's (keys and fields from
`control.rs`'s describe), its lists sized as #319 measured a busy desktop (agents ~11 KB, catalog ~4.5 KB): it is not a capture of a real desktop, so the table is a measure of
the fold, and the VM's own number is for the core-flow run.
"""
import importlib.util
import json
import os
import sys
from importlib.machinery import SourceFileLoader

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yos_fold  # noqa: E402
import yos_mcp_cap  # noqa: E402

# Why 10,000 bytes (about 2,500 tokens): a free account's per-minute allowance (Groq free lists
# tpm 8,000 in `tiers.rs`) has to hold a mind's own prompt (6,700 tokens in the failing run) next
# to everything it has read. Something new in the shell's state pays for its room in this number.
DESCRIBE_SHELL_BUDGET = 10000

loader = SourceFileLoader("yos_budget", os.path.join(HERE, "yos"))
spec = importlib.util.spec_from_loader("yos_budget", loader)
yos = importlib.util.module_from_spec(spec)
loader.exec_module(yos)

FAILED = []


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else "  -- " + str(detail)))
    if not ok:
        FAILED.append(name)


SENTENCE = ("This account lists its models and answers; the person chose whether things marked private "
            "may go there, and a free tier that may train on prompts is never shared with, whatever is chosen.")


def representative_state():
    """3 AI accounts, 25 apps, the mode, approvals, the clock, windows, and the lists a busy desktop holds."""
    apps = [{"name": "app%d" % i, "opens": "app", "describe_as": "app%d" % i, "running": i % 3 == 0,
             "for": "What app %d is for, in the sentence its .desktop file declares for it." % i,
             "show_with": "os_act on shell with open_app name=app%d, then os_describe app%d" % (i, i)}
            for i in range(25)]
    accounts = [{"id": n, "name": n.title(), "models": "%d models" % (8 + i), "state": "ready",
                 "detail": SENTENCE, "local": n == "ollama", "sharing": "ask",
                 "private": SENTENCE, "private_lane": False} for i, n in enumerate(("groq", "openai", "ollama"))]
    return {
        "summary": "Yantrik — desktop screen", "screen": "desktop", "screen_id": 0, "locked": False,
        "private": False, "version": "0.9.1",
        "clock": {"date": "2026-11-24", "weekday": "Tuesday", "time": "03:41", "utc_offset": "+00:00", "zone": "UTC"},
        "desktop_in_front": True, "in_front": "desktop",
        "windows": [{"title": "Notes", "app": "notes", "focused": False}, {"title": "Files", "app": "files", "focused": True}],
        "companion_online": True, "companion_status": "ready", "thinking": False,
        "pending_approvals": [{"id": "g-1", "app": "notes", "action": "delete_note", "grade": "sensitive",
                               "why": "Deletes the note 'Plan' for good. The person decides on a card."}],
        "pending_questions": [],
        "tool_permission": "sensitive",
        "mind_mode": {"mode": "ask", "rules": [{"app": "notes", "action": "new_note", "for": "session"}]},
        "approvals_off_for_test": {"on": False, "expires_in_secs": None,
                                   "means": "Off: approvals work as the mode says. A test run turns this on so that nothing "
                                            "is ever put in front of the person."},
        "ai_accounts": {"summary": "3 accounts", "accounts": accounts},
        "apps": apps,
        "pinned": ["notes", "files", "calendar"],
        "minds": [{"id": "pi", "name": "Pi", "answering": True, "builtin": False, "tools": 30,
                   "detail": "Pi on Groq: its memory is its own session, running in the person's home on this machine."}],
        "harnesses": [{"id": h, "installed": True, "running": h == "pi",
                       "why": "Installed and its unit is %s; the config file was written when it was first set up." % h}
                      for h in ("pi", "hermes", "claude-code", "codex", "aider", "goose", "opencode")],
        "catalog": [{"role": "r%d" % i, "attached": True,
                     "may": "May read and write the notes, open files in the person's home, and run commands in its own terminal, "
                            "never outside the folder the person gave it."} for i in range(18)],
        "agents": [{"id": "pi:a%d" % i, "state": "idle", "task": "Summarise what changed in the repository since "
                    "the last release and write it up as a note the person can read in the morning."} for i in range(40)],
        "recipes": [{"id": "rcp%d" % i, "status": "waiting", "step": "Waiting for the person to answer a question about "
                     "which folder the report should be saved in before it goes on."} for i in range(9)],
        "mind_audit_recent": [{"app": "notes", "action": "new_note", "result": "ok",
                               "at": "03:%02d" % i} for i in range(40)],
        "services": [{"id": "svc%d" % i, "state": "running"} for i in range(30)],
        "vault": {"protected": False, "why": "This session signs in without a password, so there is nothing to lock the vault with."},
        "bond": "Partner-in-Crime", "memories": 412,
    }


def fold(state, detail=None):
    return yos.render_state(state, "shell", True, detail)


def size(v):
    return len(json.dumps(v, ensure_ascii=False).encode("utf-8"))


def table(state, out):
    """Bytes by top-level key: the fold before this change (every key whole, `apps` left out) and after."""
    after = json.loads(out[:out.rindex("}") + 1])
    rows = [(k, 0 if k == "apps" else size(state[k]), size(after[k]) if k in after else 0) for k in state]
    rows.sort(key=lambda r: -r[1])
    lines = ["| key | before | after |", "|---|---:|---:|"]
    lines += ["| %s | %d | %d |" % r for r in rows]
    lines.append("| **all keys, as printed** | %d | %d |" % (
        len(fold(state, None).encode("utf-8")) - sum(r[2] for r in rows) + sum(r[1] for r in rows), len(out.encode("utf-8"))))
    return "\n".join(lines), after


def main():
    state = representative_state()
    out = fold(state)
    text, after = table(state, out)
    print(text + "\n")
    if "--table" in sys.argv:
        return 0

    print("the folded describe shell holds to its budget")
    n = len(out.encode("utf-8"))
    check("%d bytes is at most %d" % (n, DESCRIBE_SHELL_BUDGET), n <= DESCRIBE_SHELL_BUDGET, n)
    check("and the fixture is big enough to mean it: unfolded it is over twice the budget",
          size(state) > 2 * DESCRIBE_SHELL_BUDGET, size(state))

    print("nothing the person relies on for safety was cut")
    for k in ("private", "locked", "screen", "clock", "windows", "in_front", "desktop_in_front",
              "pending_approvals", "pending_questions", "mind_mode", "tool_permission"):
        check("%s is whole" % k, after.get(k) == state[k], after.get(k))
    check("the approvals-off switch keeps its state", after["approvals_off_for_test"] == {"on": False, "expires_in_secs": None},
          after["approvals_off_for_test"])
    check("an account keeps its sharing choice and where it runs",
          all(a["sharing"] and "local" in a and "state" in a for a in after["ai_accounts"]["accounts"]), after["ai_accounts"])

    print("it says what was kept short, and `detail` reads one key in full")
    check("the last line names the keys and the way to ask",
          "kept short:" in out and "ai_accounts" in out.rsplit("(kept short:", 1)[1] and "detail=<key>" in out, out[-300:])
    whole = fold(state, "ai_accounts")
    check("detail=ai_accounts is that key, every sentence in it",
          json.loads(whole) == {"ai_accounts": state["ai_accounts"]}, whole[:200])
    check("detail=apps lists the apps, which the fold names and does not print",
          json.loads(fold(state, "apps")) == {"apps": state["apps"]})
    check("an unknown detail names the keys there are", "no key `nope`" in fold(state, "nope") and "ai_accounts" in fold(state, "nope"))
    check("a non-shell app is not folded", fold_other(state) == yos.render_state(state, "notes", True))
    argv = yos_mcp_cap.describe_argv({"app": "shell", "detail": "catalog"})
    check("os_describe detail becomes --detail, actions still wins",
          argv == ["describe", "shell", "--fold", "--detail", "catalog"]
          and yos_mcp_cap.describe_argv({"app": "shell", "actions": "files_", "detail": "x"})[3] == "--no-state", argv)

    print("the bridge's cap on any one result")
    cap = yos_mcp_cap.RESULT_CAP_BYTES
    small = "a line\n" * 10
    check("a result under the cap is returned as it is", yos_mcp_cap.cap_result(small, "os_apps", {}) == small)
    big = "".join("line %05d %s\n" % (i, "é" * 40) for i in range(2000))
    cut = yos_mcp_cap.cap_result(big, "os_describe", {"app": "shell"})
    check("a bigger one is within the cap, notes included", len(cut.encode("utf-8")) <= cap, len(cut.encode("utf-8")))
    body, _, last = cut.rpartition("\n")
    check("it is cut on a line boundary", all(l.startswith("line ") and l.endswith("é" * 40) for l in body.split("\n")), body[-80:])
    check("the last line says it was cut, by how much, and how to ask for the rest",
          "cut:" in last and str(len(big.encode("utf-8"))) in last and "detail=<one key" in last, last)
    check("another app is pointed at its actions family",
          "os_describe app=notes with actions=" in yos_mcp_cap.cap_result(big, "os_describe", {"app": "notes"}))
    check("any other tool is told to ask for less", "ask" in yos_mcp_cap.cap_result(big, "web_read", {}))
    one = "x" * (3 * cap)
    check("one line bigger than the cap is cut at the cap, not returned", len(yos_mcp_cap.cap_result(one, "t", {}).encode()) <= cap)

    mcp = SourceFileLoader("yosmcp_budget", os.path.join(HERE, "yos-mcp")).load_module()
    src = open(os.path.join(HERE, "yos-mcp"), encoding="utf-8").read()
    check("serve_call caps the one text every tool's answer goes through",
          'cap_result(scrub(text), tool["name"], arguments)' in src and src.count("cap_result(") == 1, "")
    check("and the bridge's own parsing reads yos directly: action_detail does not pass through serve_call",
          "cap_result" not in src.split("def action_detail", 1)[1].split("\ndef ", 1)[0])
    check("os_describe takes `detail`", "detail" in mcp.TOOLS_BY_NAME["os_describe"]["inputSchema"]["properties"]
          if hasattr(mcp, "TOOLS_BY_NAME") else any(t["name"] == "os_describe" and "detail" in t["inputSchema"]["properties"] for t in mcp.TOOLS))

    print("\n%d failed" % len(FAILED) if FAILED else "\nall passed")
    return 1 if FAILED else 0


def fold_other(state):
    return yos.render_state(state, "notes", True)


if __name__ == "__main__":
    sys.exit(main())
