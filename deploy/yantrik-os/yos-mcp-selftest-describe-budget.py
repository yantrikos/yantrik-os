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
import re
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
SEND_CHAT_MESSAGE_DESCRIPTION = 1500  # in the `send_` family on the machine, so it is not one of the 62


# The folded state's keys as #1389's build printed them on the machine (22,746 bytes in all, 9,125 of it
# state): the eight the issue (#1396) lists, in bytes. The rest of the 9,125 is the other ~30 keys.
MACHINE_FOLDED_KEYS = {"agents": 586, "ai_accounts": 535, "catalog": 504, "recipes": 403, "mind_panel": 384,
                       "notifications": 369, "harnesses": 351, "mind_mode": 313}
MACHINE_FOLDED_STATE_BYTES, MACHINE_FOLDED_TOTAL_BYTES = 9125, 22746
# What this PR's fold printed of the machine's state (test VM, review of 2e0f663c): the fixture is pinned to it.
MACHINE_AFTER_STATE_BYTES = 3899  # (unused: the calibration above pins the state)


def question_vocabulary():
    """The fields the four question actions take: `asks.rs`'s FIELDS and `step_1`..`step_<STEP_FIELDS>`,
    read from the source, so the fixture's signatures are the machine's and cannot drift from it."""
    src = open(os.path.join(HERE, "..", "..", "crates", "yantrik-companion-tools", "src", "asks.rs"), encoding="utf-8").read()
    block = src.split("pub const FIELDS: &[(&str, &str)] = &[", 1)[1].split("\n];", 1)[0]
    fields = re.findall(r'\(\s*"([a-z_]+)",\s*"((?:[^"\\]|\\.)*)"\s*\)', block)
    steps = int(re.search(r"pub const STEP_FIELDS: usize = (\d+);", src).group(1))
    return fields + [("step_%d" % i, "One step of a recipe, in words, quoted") for i in range(1, steps + 1)]


# The machine's 11 families (test VM, 24 Nov, #1397's review): their member counts. All but `capture` mix grades,
# so on the machine each prints as one line grouped by grade; nearly every action settles "on return".
FAMILIES = {"open": 10, "send": 4, "read": 4, "close": 8, "show": 7, "set": 14, "files": 21, "capture": 4,
            "record": 4, "agent": 6, "workshop": 5}
FAMILY_WORDS = ("lens", "today_view", "app_window", "capture_area", "note_pad", "list_items", "page_up", "view_mode",
                "item_detail", "panel_side", "card_stack", "tab_next")
QUESTIONS = ("delete_for_good", "change_item", "plan_work", "send_chat_message")


def question_parameters():
    """What `control_questions.rs` publishes: every field of the vocabulary, a string, none required."""
    return {"required": [], "properties": {k: {"type": "string", "description": d} for k, d in question_vocabulary()}}


def filler_parameters(i):
    """The parameters of the machine's other actions, in four shapes: a few required, a few named optionals, a count."""
    def spec(names):
        return {n: {"type": "string", "description": "the " + n} for n in names}
    shapes = ((["id", "kind"], ["note", "when", "where", "limit"]), (["target"], ["note", "when"]),
              ([], ["app"]), (["id", "text"], ["limit"]))
    required, optional = shapes[i % 4]
    return {"required": required, "properties": spec(required + optional)}


def machine_actions():
    """62 actions not in any family (no four share a word before the first underscore), plus the
    families the shell already folds (open_ send_ read_ close_ show_ set_ files_ capture_), each with the
    grade marks and a first sentence. Descriptions run as long as the machine's."""
    small = (48363 - sum(BIG_DESCRIPTIONS.values())) // 54
    names = list(BIG_DESCRIPTIONS) + ["act%02d_the_whole_thing" % i if i % 2 else "do%02d_the_whole_task" % i for i in range(54)]
    acts = []
    for i, n in enumerate(names):
        size = BIG_DESCRIPTIONS.get(n, small)
        first = "What %s does for the person, in one plain sentence." % n.replace("_", " ")
        body = words(max(size - len(first) - 1, 0), i)
        acts.append({"name": n, "permission": "sensitive" if i % 5 == 0 else "safe",
                     "settles": "later" if i % 7 == 0 else "on return",
                     "description": first + " " + body,
                     "parameters": question_parameters() if n in QUESTIONS else filler_parameters(i)})
    acts.append({"name": "send_chat_message", "permission": "sensitive", "settles": "later",
                 "description": "Asks the person to allow a message to go. " + words(SEND_CHAT_MESSAGE_DESCRIPTION, 3),
                 "parameters": question_parameters()})
    for fam, count in FAMILIES.items():
        members = count - 1 if fam == "send" else count  # send_chat_message is the fourth `send_`
        for j in range(members):
            grade = "safe" if fam == "capture" else ("safe", "standard", "sensitive")[(j * 7 + len(fam)) % 3 if j % 4 else 0]
            params = ({"name": {"type": "string"}}, {}, {"app": {"type": "string"}, "window": {"type": "string"}},
                      {"path": {"type": "string"}, "limit": {"type": "integer"}, "after": {"type": "string"}})[j % 4]
            acts.append({"name": "%s_%s" % (fam, FAMILY_WORDS[j % len(FAMILY_WORDS)] + ("" if j < len(FAMILY_WORDS) else str(j))),
                         "permission": grade, "settles": "later" if j % 11 == 5 else "on return",
                         "description": "Does one of the %s things. It takes what it is given." % fam,
                         "parameters": {"required": ["name"] if j % 4 == 0 else [], "properties": params}})
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
                 "private": SENTENCE, "private_lane": False} for i, n in enumerate(("groq", "openai", "ollama", "anthropic"))]
    return {
        "summary": "Yantrik — desktop screen", "screen": "desktop", "screen_id": 0, "locked": False,
        "private": False, "version": "0.9.1",
        "clock": {"date": "2026-11-24", "weekday": "Tuesday", "time": "03:41", "utc_offset": "+00:00", "zone": "UTC"},
        "desktop_in_front": True, "in_front": "desktop",
        "windows": [{"title": t.title(), "app": t, "focused": t == "files"}
                    for t in ("notes", "files", "calendar", "mail", "browser", "terminal", "music", "photos", "maps", "clock")],
        "companion_online": True, "companion_status": "ready", "thinking": False,
        "pending_approvals": [{"id": "g-%d" % i, "app": "notes", "action": "delete_note", "grade": "sensitive",
                               "why": "Deletes the note 'Plan' for good. The person decides on a card."} for i in (1, 2, 3)],
        "pending_questions": [{"id": "q-1", "tool": "delete_for_good", "grade": "sensitive",
                              "text": "Delete the note 'Plan' for good? The person decides on a card; nothing is deleted until then."}],
        "tool_permission": "sensitive",
        "mind_mode": {"mode": "ask", "rules": [{"app": "notes", "action": a, "for": "session"} for a in
                                       ("new_note", "edit_note", "pin_note", "move_note", "tag_note")]},
        "approvals_off_for_test": {"on": False, "expires_in_secs": None,
                                   "means": "Off: approvals work as the mode says. A test run turns this on so that nothing "
                                            "is ever put in front of the person."},
        "ai_accounts": {"summary": "4 accounts", "accounts": accounts},
        "apps": apps,
        "pinned": ["notes", "files", "calendar", "mail", "browser", "terminal", "music", "photos"],
        "minds": [{"id": "pi", "name": "Pi", "answering": True, "builtin": False, "tools": 30,
                   "detail": "Pi on Groq: its memory is its own session, running in the person's home on this machine."}],
        "harnesses": rows(5, 8, 80, "h"),
        "catalog": rows(18, 12, 80, "r"),
        "agents": rows(40, 14, 100, "pi:a"),
        "recipes": rows(9, 9, 100, "rcp"),
        "problems": rows(9, 2, 80, "p"),
        "notifications": rows(9, 9, 60, "n"),
        "dock": rows(8, 2, 50, "d"),
        "proofs": rows(4, 3, 24, "pf"),
        "mind_view": rows(9, 1, 40, "mv"),
        "workroom": rows(5, 5, 100, "w"),
        "mind_audit_recent": [{"app": "notes", "action": "new_note", "result": "ok",
                               "at": "03:%02d" % i} for i in range(40)],
        "services": [{"id": "svc%d" % i, "state": "running"} for i in range(30)],
        "vault": {"protected": False, "why": "This session signs in without a password, so there is nothing to lock the vault with."},
        "bond": "Partner-in-Crime", "memories": 412,
        # The keys the machine's describe also carries (control.rs), at sizes that bring the folded
        # state to its 9,125 bytes: `mind_panel` 384, and the small ones the issue says add up.
        "mind_panel": {"open": False, "tab": "agents", "selected": None, "filter": "", "sort": "recent",
                       "items": rows(1, 3, 24, "mp"), "hint": words(160, 5)},
        "failed_launches": [{"app": "app%d" % i, "why": words(60, i), "at": "03:1%d" % i} for i in range(3)],
        "mind_preference": {"mind": "pi", "why": words(90, 2), "can_change": True},
        "companion": {"name": "Yantrik", "model": "groq/llama", "language": "en", "voice": False,
                      "note": words(110, 7)},
        "network": {"online": True, "ssid": "home", "ip": "10.0.0.2", "vpn": False, "note": words(80, 4)},
        "battery": {"percent": 88, "charging": True, "minutes_left": None, "note": words(70, 6)},
        "storage": {"free_gb": 41, "total_gb": 120, "warn": False, "note": words(90, 9)},
        "volume": {"level": 40, "muted": False, "output": "speakers", "note": words(60, 8)},
        "updates": {"available": True, "version": "0.9.2", "channel": "stable", "note": words(120, 3)},
        "installer": {"state": "idle", "last": words(80, 1), "queue": 0},
        "files": {"home": "/home/p", "recent": [words(40, i) for i in range(5)], "note": words(70, 2)},
        "backup": {"last": "2026-11-23", "ok": True, "where": words(50, 3), "note": words(90, 5)},
        "focus": {"on": False, "until": None, "note": words(100, 11)},
        "display": {"scale": 1.0, "width": 1920, "height": 1080, "dark": True, "note": words(110, 13)},
        "keyboard": {"layout": "us", "shortcuts": True, "note": words(120, 14)},
        "locale": {"language": "en", "region": "IN", "time_format": "24h", "note": words(70, 12)},
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


def size(v):
    return len(json.dumps(v, ensure_ascii=False).encode("utf-8"))


def shortened_1389(k, v):
    """What #1389's fold printed of a key: its first row, strings clipped at 24 (the lists), or the
    explanatory sentences dropped, and no count: the rules of that round, kept here to measure against."""
    tight = (yos_fold.TIGHT_ROWS, yos_fold.TIGHT_CLIP) if k in yos_fold.TIGHT_KEYS else (yos_fold.ROWS_SHOWN, yos_fold.CLIP_AT)
    if k in yos_fold.DROP_FIELDS or k in yos_fold.CLIP_KEYS:
        drop = [d for d in yos_fold.DROP_FIELDS.get(k, ()) if d != "models"]
        return yos_fold._shorten(v, drop, k in yos_fold.CLIP_KEYS, *tight)
    return v


def state_bytes_1389(state):
    return sum(len(("  %s: %s" % (json.dumps(k), json.dumps(shortened_1389(k, v)))).encode()) + 2
               for k, v in state.items() if k not in yos_fold.LEFT_OUT)


def line_1389(a):
    """An action's line as #1389 printed it: every parameter in the signature, the sentence in what room is left."""
    params = a.get("parameters") or {}
    props, required = params.get("properties") or {}, set(params.get("required") or [])
    names = [n for n in props if n in required] + [n for n in props if n not in required]
    head = "  act: %s(%s)  [%s, settles %s]" % (a["name"], ", ".join(n if n in required else n + "?" for n in names),
                                                a["permission"], a["settles"])
    room = yos_fold.LINE_AT - len(head) - 3
    sentence = yos.first_sentence(a["description"])
    return head if not sentence or room < 12 else "%s — %s" % (head, sentence[:room - 1].rstrip() + "…" if len(sentence) > room else sentence)


def actions_bytes_1389(actions):
    """Standalone actions as #1389 printed them; a family on one line with every parameter of every member."""
    fam = {n for n in ("open", "send", "read", "close", "show", "set", "files", "capture")}
    lines, seen = [], set()
    for a in actions:
        head = a["name"].split("_", 1)[0]
        if head in fam:
            if head not in seen:
                seen.add(head)
                members = [m for m in actions if m["name"].split("_", 1)[0] == head]
                lines.append("  %s_* — %d actions, signatures only; each in full: actions=%s_" % (head, len(members), head))
                lines.extend("  act: %s(%s)  [%s, settles %s]" % (m["name"], ", ".join((m.get("parameters") or {}).get("properties", {})),
                                                                  m["permission"], m["settles"]) for m in members)
        else:
            lines.append(line_1389(a))
    return len(("\n" + "\n".join(lines)).encode("utf-8"))


def parse_state(text):
    """The state object a describe printed: from its first `{` to the `}` in column zero."""
    return json.loads(text[text.index("{"):text.index("\n}") + 2])


def table(state, actions, full):
    """Markdown: the machine's bytes (#1389's build, measured), the fixture under #1389's fold, the fixture now."""
    after = parse_state(full)
    state_after = len(full[:full.index("\n}") + 2].encode("utf-8"))
    tail_after = len(full[full.index("\n}") + 2:].encode("utf-8"))
    state_before, actions_before = state_bytes_1389(state), actions_bytes_1389(actions)
    lines = ["| part of the folded `describe shell` | machine before | fixture before | fixture after |",
             "|---|---:|---:|---:|"]
    lines += ["| `%s` | %d | %d | %d |" % (k, b, size(shortened_1389(k, state[k])), size(after[k]))
              for k, b in MACHINE_FOLDED_KEYS.items()]
    lines.append("| **state, all %d keys** | %d | %d | %d |" % (len(state) - 1, MACHINE_FOLDED_STATE_BYTES, state_before, state_after))
    lines.append("| **actions after the state** | %d | %d | %d |" % (
        MACHINE_FOLDED_TOTAL_BYTES - MACHINE_FOLDED_STATE_BYTES, actions_before, tail_after))
    lines.append("| **everything `os_describe shell` returns** | %d | %d | %d |" % (
        MACHINE_FOLDED_TOTAL_BYTES, state_before + actions_before, len(full.encode("utf-8"))))
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
    check("and at most %d, which leaves the machine room to differ from the fixture" % (DESCRIBE_SHELL_BUDGET - 500),
          n <= DESCRIBE_SHELL_BUDGET - 500, n)
    unfolded = size(state) + sum(len(render_whole(a).encode()) for a in actions)
    check("and the fixture is the machine's size: unfolded it is over 7 times the budget",
          unfolded > 7 * DESCRIBE_SHELL_BUDGET, unfolded)
    off = [k for k, b in MACHINE_FOLDED_KEYS.items() if abs(size(shortened_1389(k, state[k])) - b) > 0.15 * b]
    check("the fixture's big keys are within 15% of the machine's byte counts as #1389 printed them", not off, off)
    st_1389 = state_bytes_1389(state)
    check("and its %d keys fold, under #1389's rules, to the machine's %d bytes of state (within 10%%): %d"
          % (len(state), MACHINE_FOLDED_STATE_BYTES, st_1389), abs(st_1389 - MACHINE_FOLDED_STATE_BYTES) < 0.1 * MACHINE_FOLDED_STATE_BYTES)
    before = st_1389 + actions_bytes_1389(actions)
    check("so under #1389's rules the fixture is over the budget, as the machine was (%d against %d)"
          % (before, MACHINE_FOLDED_TOTAL_BYTES), before > DESCRIBE_SHELL_BUDGET and abs(before - MACHINE_FOLDED_TOTAL_BYTES) < 0.2 * MACHINE_FOLDED_TOTAL_BYTES)
    check("62 actions outside any family, descriptions as long as the machine's",
          sum(1 for a in actions if a["name"] in BIG_DESCRIPTIONS or a["name"][:2] in ("do", "ac")) == 62
          and len(actions[0]["description"]) >= 5000)

    print("actions: a signature, the grade marks and the first sentence, one line of about 120 characters at most")
    lines = [l for l in out.split("\n") if l.startswith("  act: ")]
    check("62 one-line actions", len(lines) == 62, len(lines))
    check("none is over 120 characters", max(len(l) for l in lines) <= 120, max(len(l) for l in lines))
    check("each keeps its grade marks", all("[sensitive" in l or "[safe" in l or "[standard" in l for l in lines))
    check("`settles` is said once in the legend, and a line says its own only when it differs",
          "settles on return unless a line says otherwise" in out and sum("settles later" in l for l in lines) > 0
          and not any("settles on return" in l for l in lines), [l for l in lines if "on return" in l][:2])
    check("the full description is not in the line", "does for the person" not in "".join(
        l for l in lines if "delete_for_good" in l) or len(lines[0]) <= 120)
    short = describe_text(state, [{"name": "ask", "permission": "safe", "settles": "now",
                                   "description": "Asks. A second sentence follows.", "parameters": {}}] * 20)
    check("a short first sentence is whole and the second is left for `actions=<name>`",
          "[safe] — Asks." in short and "second sentence" not in short, short[-200:])
    one = describe_text(state, actions, actions="delete_for_good")
    check("`actions=<name>` prints that one action, its whole description",
          one.startswith(yos.SIGNATURES_ONLY) and len(one) > 5000 and "act: delete_for_good(" in one
          and "plan_work" not in one, one[:200])
    fam = describe_text(state, actions, actions="open_")
    check("`actions=<family>_` still prints the family", "open_lens" in fam and "other actions of shell" in fam)

    # The fixture is calibrated to the machine at round 2's settings (#1397's review of 5ec9e94d: 10,792 bytes on the
    # test VM; state 3,897, the 62 one-line actions 3,782 at 57 characters on average, 10 family lines 2,755).
    saved = (yos_fold.LINE_AT, yos_fold.HINT_AT, yos_fold.BARE_AT, yos_fold.FOLD_KEY_AT)
    yos_fold.LINE_AT, yos_fold.HINT_AT, yos_fold.BARE_AT, yos_fold.FOLD_KEY_AT = 56, 10, 10 ** 9, 100
    try:
        r2 = describe_text(state, actions)
    finally:
        yos_fold.LINE_AT, yos_fold.HINT_AT, yos_fold.BARE_AT, yos_fold.FOLD_KEY_AT = saved
    r2_state = len(r2[:r2.index("\n}") + 2].encode("utf-8"))
    r2_lines = [l for l in r2.split("\n") if l.startswith("  act: ")]
    r2_fams = [l for l in r2.split("\n") if "_* " in l and not l.startswith("  act") and not l.startswith("  capture_*")]
    for what, got, want in (("state", r2_state, 3897), ("62 one-line actions", sum(len(l) + 1 for l in r2_lines), 3782),
                            ("10 family lines", sum(len(l) + 1 for l in r2_fams), 2755), ("everything", len(r2.encode("utf-8")), 10792)):
        check("calibrated to the machine at round 2's settings: %s %d against %d (within 3%%)" % (what, got, want),
              abs(got - want) <= 0.03 * want, got)
    st_after = len(out[:out.index("\n}") + 2].encode("utf-8"))
    heads = [l.split(" ")[2] for l in out.split("\n") if l.startswith("  ") and "_* " in l.split(" — ")[0] + " "]
    check("11 families: %s" % ", ".join(h.rstrip("*").rstrip("_") for h in heads),
          sorted(h.rstrip("*").rstrip("_") for h in heads) == sorted(FAMILIES), heads)
    for fam, n in FAMILIES.items():
        got = sum(1 for a in actions if a["name"].split("_", 1)[0] == fam)
        check("%s_ has %d members%s" % (fam, n, "" if fam == "capture" else ", of mixed grades"),
              got == n and (len({a["permission"] for a in actions if a["name"].split("_", 1)[0] == fam}) > 1) == (fam != "capture"), got)

    print("signatures: the required parameters, then a count of the optional ones")
    vocab = question_vocabulary()
    check("the question actions take the card vocabulary of asks.rs, %d fields (48+ named, and step_1..step_12)" % len(vocab),
          len(vocab) >= 60 and ("step_12", "One step of a recipe, in words, quoted") in vocab and ("tool", "The companion tool that is asking") in vocab)
    for q in QUESTIONS:
        a = next(x for x in actions if x["name"] == q)
        check("%s has every one of the %d fields, none required" % (q, len(vocab)),
              list(a["parameters"]["properties"]) == [k for k, _ in vocab] and not a["parameters"]["required"])
    for q in QUESTIONS[:3]:
        line = next(l for l in lines if l.startswith("  act: %s(" % q))
        check("%s is one line of at most 120 characters: %r" % (q, line[:60] + "…"),
              len(line) <= 120 and "+%d optional (actions=%s)" % (len(vocab), q) in line and "step_1" not in line, len(line))
    sent = next(l for l in out.split("\n") if l.startswith("  send_* "))
    check("send_chat_message, in the `send_` family, is counted, not listed, and the line is short",
          "send_chat_message" in sent and "step_1" not in sent and "optional" not in sent and len(sent) <= 160, sent)
    sends = [a for a in actions if a["name"].split("_", 1)[0] == "send"]
    check("and with its arguments, a short family line counts the vocabulary: send_chat_message(+%d optional)" % len(vocab),
          "send_chat_message(+%d optional)" % len(vocab) in yos_fold.render_family("send", sends, "on return", bare_at=10 ** 9))
    fam_line = next(l for l in out.split("\n") if l.startswith("  open_* "))
    opens = [a for a in actions if a["name"].split("_", 1)[0] == "open"]
    with_args = yos_fold.render_family("open", opens, "on return", bare_at=10 ** 9)
    check("a family line names a member's few optional parameters, as a line does",
          "open_lens(name)" in with_args and "open_app_window(app?, window?)" in with_args, with_args)
    check("a family line past %d characters names its members without arguments, and `actions=open_` has the rest"
          % yos_fold.BARE_AT, len(with_args) > yos_fold.BARE_AT and "(" not in fam_line.split(" — ", 1)[1]
          and "open_app_window" in fam_line and len(fam_line) < len(with_args), fam_line)
    check("a mixed family is one line, its members grouped by grade", "[safe] " in fam_line and "[sensitive] " in fam_line
          and "[standard] " in fam_line and fam_line.startswith("  open_* (10) — "), fam_line)
    check("every one of the machine's 11 families is a line, none a line a member",
          sum(1 for l in out.split("\n") if l.startswith("  ") and "_* " in l.split(" — ")[0] + " ") == len(FAMILIES)
          and not any(l.startswith("    act:") for l in out.split("\n")))
    ex = next(l for l in lines if "(id, kind, " in l)
    check("a parameter the call must have is named, then the rest are counted past %d" % yos_fold.NAMED_AT, "(id, kind, +4 optional)" in ex, ex)
    two = {"name": "open_lens", "parameters": {"required": [], "properties": {"app": {}, "x": {}}}}
    check("a few optional parameters are named with a ?, as `open_lens(app?, x?)`", yos_fold.signature(two) == "open_lens(app?, x?)")
    check("past a few, or when the names run long, they are counted", yos_fold.signature({"name": "n", "parameters": {
        "properties": {k: {} for k in "abcd"}}}) == "n(+4 optional)" and "+2 optional" in yos_fold.signature({"name": "n", "parameters": {
            "properties": {"a" * 20: {}, "b" * 20: {}}}}))
    check("an action with nothing optional has no count", yos_fold.signature({"name": "x", "parameters": {
        "required": ["a"], "properties": {"a": {}}}}) == "x(a)")
    check("nor one with nothing at all", yos_fold.signature({"name": "x", "parameters": {}}) == "x()")
    whole_q = describe_text(state, actions, actions="plan_work")
    check("`actions=plan_work` still prints every field and what it means",
          all(("         %s?: string" % k) in whole_q for k, _ in vocab) and "The companion tool that is asking" in whole_q)

    print("a row shown keeps its id and name whole")
    longid, longname = "agent:" + "i" * 24, "Name " + "n" * 26
    shown = yos_fold._shorten([{"id": longid, "name": longname}], (), True, yos_fold.TIGHT_ROWS, yos_fold.TIGHT_CLIP)
    check("a shown row's id and name are not clipped (TIGHT_CLIP is 24)",
          shown == [{"id": longid, "name": longname}], shown)
    nested = yos_fold._shorten({"id": longid, "name": longname, "text": "t" * 200, "rows": [{"id": longid}]}, (), True, 1, 24)
    check("whatever the depth, and a long text beside them is", nested["id"] == longid and nested["name"] == longname
          and nested["rows"] == [{"id": longid}] and len(nested["text"]) == 25, nested)

    print("a key that is still big is folded to a count")
    big_state = dict(state, mystery={"a": words(200, 1)}, rowset=[{"v": words(50, i)} for i in range(5)])
    got = yos_fold.fold_shell(big_state)[0]
    check("an object over %d bytes is `N fields`, a list `N rows`" % yos_fold.FOLD_KEY_AT,
          got["mystery"] == "1 field" and got["rowset"] == "5 rows", (got["mystery"], got["rowset"]))
    check("and detail=<key> prints it whole", json.loads(fold(big_state, "rowset"))["rowset"] == big_state["rowset"])
    check("the safety keys are never counted, whatever their size",
          all(yos_fold.fold_shell(dict(state, **{k: [{"x": "y" * 300}] * 3}))[0][k] != "3 rows" for k in yos_fold.NEVER_COUNTED if k in state))

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
