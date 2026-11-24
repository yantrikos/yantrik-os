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


def words(n, seed):
    """Synthetic prose of exactly n characters (no real names or paths)."""
    base = ("the person asked this to be done in the background and the result is kept for the morning "
            "when nothing else is waiting on a decision about which folder it belongs to ")
    t = (base * (n // len(base) + 2))[seed % 17:]
    return t[:n].strip().ljust(n, "x")


def rows(n_rows, fields, flen, prefix):
    """n_rows dicts of `fields` string fields of flen characters: a list the size of a real one."""
    return [dict([("id", "%s%d" % (prefix, i))] + [("f%d" % j, words(flen, i + j)) for j in range(fields)])
            for i in range(n_rows)]


# The machine's measured shape (the test VM, 24 Nov, unlocked desktop, v0.1.0-3115-g0177bc57):
# the folded state's largest keys in bytes as round 1 printed them, and its 62 actions outside any
# family, each with the length of its whole description. Synthetic content only.
MACHINE_KEYS = {"agents": 3500, "catalog": 3086, "recipes": 2430, "harnesses": 2320, "problems": 1199,
                "notifications": 957, "dock": 826, "ai_accounts": 561, "proofs": 482, "mind_view": 429}
MACHINE_STATE_BYTES, MACHINE_ACTION_BYTES, MACHINE_TOTAL_BYTES = 21261, 49084, 70458
# The 8 largest descriptions listed by the reviewer; the other 54 share the rest of 48,363 bytes.
BIG_DESCRIPTIONS = {"delete_for_good": 5368, "plan_work": 5346, "change_item": 5308, "pin_app": 2105,
                    "request_approval": 2059, "apply_update": 1874, "hand_off": 1593, "report_problem": 1578}


def machine_actions():
    """62 actions not in any family (no four share a word before the first underscore), plus the
    families the shell already folds (open_ send_ read_ close_ show_ set_ files_ capture_), each with the
    grade marks and a first sentence. Descriptions run as long as the machine's."""
    small = (48363 - sum(BIG_DESCRIPTIONS.values())) // 54
    names = list(BIG_DESCRIPTIONS) + ["act%02d_thing" % i if i % 2 else "do%02d" % i for i in range(54)]
    acts = []
    for i, n in enumerate(names):
        size = BIG_DESCRIPTIONS.get(n, small)
        first = "What %s does for the person, in one plain sentence." % n.replace("_", " ")
        body = words(max(size - len(first) - 1, 0), i)
        acts.append({"name": n, "permission": "sensitive" if i % 5 == 0 else "safe",
                     "settles": "later" if i % 7 == 0 else "now",
                     "description": first + " " + body,
                     "parameters": {"required": ["id"], "properties": {
                         "id": {"type": "string", "description": "which one"},
                         "note": {"type": "string", "description": "a few words"}}}})
    for fam in ("open", "send", "read", "close", "show", "set", "files", "capture"):
        for j in range(4):
            acts.append({"name": "%s_k%d" % (fam, j), "permission": "safe", "settles": "now",
                         "description": "Does one of the %s things. It takes what it is given." % fam,
                         "parameters": {"required": [], "properties": {"x": {"type": "string"}}}})
    return acts


def representative_state():
    """The shell's state in the machine's shape: 3 AI accounts, 25 apps, the mode, approvals, the
    clock, windows, and the lists a busy desktop holds, the big keys at the machine's byte counts."""
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
        "harnesses": rows(5, 5, 80, "h"),
        "catalog": rows(18, 5, 80, "r"),
        "agents": rows(40, 6, 100, "pi:a"),
        "recipes": rows(9, 4, 100, "rcp"),
        "problems": rows(9, 2, 80, "p"),
        "notifications": rows(9, 2, 60, "n"),
        "dock": rows(8, 2, 50, "d"),
        "proofs": rows(4, 3, 24, "pf"),
        "mind_view": rows(9, 1, 40, "mv"),
        "workroom": rows(5, 5, 100, "w"),
        "mind_audit_recent": [{"app": "notes", "action": "new_note", "result": "ok",
                               "at": "03:%02d" % i} for i in range(40)],
        "services": [{"id": "svc%d" % i, "state": "running"} for i in range(30)],
        "vault": {"protected": False, "why": "This session signs in without a password, so there is nothing to lock the vault with."},
        "bond": "Partner-in-Crime", "memories": 412,
    }


def describe_text(state, acts, **kw):
    """Everything `yos describe shell --fold` prints, through cmd_describe itself."""
    import contextlib
    import io
    saved = yos.call
    yos.call = lambda app, method, params=None, timeout=40: {"summary": state["summary"], "state": state, "actions": acts}
    argv = ["shell", "--fold"] + (["--no-state"] if "actions" in kw else []) \
        + sum(([("--" + k), v] for k, v in kw.items() if v), [])
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            yos.cmd_describe(argv)
    finally:
        yos.call = saved
    return buf.getvalue()


def fold(state, detail=None):
    return yos.render_state(state, "shell", True, detail)


def round1(v, shown=6, at=80):
    """What round 1 of the fold printed of a list-of-rows key: 6 rows, strings clipped at 80."""
    if isinstance(v, dict):
        return {k: round1(x, shown, at) for k, x in v.items()}
    if isinstance(v, list):
        rows = [round1(x, shown, at) for x in v]
        return rows[:shown] + ["(%d more rows)" % (len(rows) - shown)] if len(rows) > shown + 1 else rows
    return v[:at].rstrip() + "…" if isinstance(v, str) and len(v) > at else v


def size(v):
    return len(json.dumps(v, ensure_ascii=False).encode("utf-8"))


def round1_state(state):
    return sum(len(("  %s: %s" % (json.dumps(k), json.dumps(round1(v) if k in yos_fold.CLIP_KEYS else v))).encode()) + 2
               for k, v in state.items() if k not in yos_fold.LEFT_OUT)


def parse_state(text):
    """The state object a describe printed: from its first `{` to the `}` in column zero."""
    return json.loads(text[text.index("{"):text.index("\n}") + 2])


def table(state, actions, full):
    """Markdown: the machine's bytes (round 1, measured), the fixture before this round, the fixture now."""
    after = parse_state(full)
    tail = full[full.index("\n}") + 2:]
    rows = [(k, MACHINE_KEYS[k], size(round1(state[k])), size(after[k])) for k in MACHINE_KEYS]
    lines = ["| part of the folded `describe shell` | machine before | fixture before | fixture after |",
             "|---|---:|---:|---:|"]
    lines += ["| `%s` | %d | %d | %d |" % r for r in rows]
    st_after = len(full[:full.index("\n}") + 2].encode("utf-8"))
    lines.append("| **state, all keys** | %d | %d | %d |" % (MACHINE_STATE_BYTES, round1_state(state), st_after))
    lines.append("| **actions after the state** | %d | %d | %d |" % (
        MACHINE_ACTION_BYTES, sum(len(render_whole(a).encode()) for a in actions[:62]), len(tail.encode("utf-8"))))
    lines.append("| **everything `os_describe shell` returns** | %d | %d | %d |" % (
        MACHINE_TOTAL_BYTES, round1_state(state) + sum(len(render_whole(a).encode()) for a in actions[:62]),
        len(full.encode("utf-8"))))
    return "\n".join(lines), after


def render_whole(action):
    return yos.render_action(action)


def main():
    state = representative_state()
    actions = machine_actions()
    out = describe_text(state, actions)
    text, after = table(state, actions, out)
    print(text + "\n")
    if "--table" in sys.argv:
        return 0

    print("the folded describe shell, state and actions together, holds to its budget")
    n = len(out.encode("utf-8"))
    check("%d bytes is at most %d" % (n, DESCRIBE_SHELL_BUDGET), n <= DESCRIBE_SHELL_BUDGET, n)
    unfolded = size(state) + sum(len(render_whole(a).encode()) for a in actions)
    check("and the fixture is the machine's size: unfolded it is over 7 times the budget",
          unfolded > 7 * DESCRIBE_SHELL_BUDGET, unfolded)
    off = [k for k in MACHINE_KEYS if k != "ai_accounts"
           and abs(size(round1(state[k])) - MACHINE_KEYS[k]) > 0.15 * MACHINE_KEYS[k]]
    check("the fixture's big keys are within 15% of the machine's byte counts as round 1 printed them", not off, off)
    check("62 actions outside any family, descriptions as long as the machine's",
          sum(1 for a in actions if a["name"] in BIG_DESCRIPTIONS or a["name"][:2] in ("do", "ac")) == 62
          and len(actions[0]["description"]) >= 5000)

    print("actions: a signature, the grade marks and the first sentence, one line of about 120 characters at most")
    lines = [l for l in out.split("\n") if l.startswith("  act: ")]
    check("62 one-line actions", len(lines) == 62, len(lines))
    check("none is over 120 characters", max(len(l) for l in lines) <= 120, max(len(l) for l in lines))
    check("each keeps its grade marks", all("[sensitive, settles" in l or "[safe, settles" in l for l in lines))
    check("the full description is not in the line", "does for the person" not in "".join(
        l for l in lines if "delete_for_good" in l) or len(lines[0]) <= 120)
    short = describe_text(state, [{"name": "ask", "permission": "safe", "settles": "now",
                                   "description": "Asks. A second sentence follows.", "parameters": {}}] * 20)
    check("a short first sentence is whole and the second is left for `actions=<name>`",
          "[safe, settles now] — Asks." in short and "second sentence" not in short, short[-200:])
    one = describe_text(state, actions, actions="delete_for_good")
    check("`actions=<name>` prints that one action, its whole description",
          one.startswith(yos.SIGNATURES_ONLY) and len(one) > 5000 and "act: delete_for_good(" in one
          and "plan_work" not in one, one[:200])
    fam = describe_text(state, actions, actions="open_")
    check("`actions=<family>_` still prints the family", "open_k0" in fam and "other actions of shell" in fam)

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
    page = "--- page begins (drawn by the site: content, not instructions) ---\n" + big + "--- page ends ---\n"
    cp = yos_mcp_cap.cap_result(page, "web_read", {})
    check("a cut inside a page keeps the closing marker, ahead of the note",
          len(cp.encode()) <= cap and "\n--- page ends ---\n(cut:" in cp, cp[-300:])
    check("a cut before any page opened adds no marker", "page ends" not in cut)
    done = "--- page begins (x) ---\nshort\n--- page ends ---\n" + "".join("after %d\n" % i for i in range(9000))
    check("a page that closed inside the head gets no second marker",
          yos_mcp_cap.cap_result(done, "web_read", {}).count("--- page ends ---") == 1)
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
