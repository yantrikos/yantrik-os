"""Results: one row per task, a scoreboard, and a comparison with the previous run."""
import glob
import json
import os
import time


def scoreboard(rows):
    n = len(rows)
    count = lambda pred: sum(1 for r in rows if pred(r))
    board = {
        "tasks": n,
        "pass": count(lambda r: r["status"] == "PASS"),
        "named": count(lambda r: r["status"] == "NAMED"),
        "fail": count(lambda r: r["status"] == "FAIL"),
        "unsure": count(lambda r: r["status"] == "UNSURE"),
        "false_claims": count(lambda r: r["truth"] == "FALSE-CLAIM"),
        "truth_unsure": count(lambda r: r["truth"] == "UNSURE"),
        "screen_moved": count(lambda r: r.get("screen_moved")),
        "timeouts": count(lambda r: r.get("timed_out")),
        "cards_seen": sum(len(r.get("cards", [])) for r in rows),
        # every figure below is read back from the OS log (see oslog.py), never assumed from a click
        "cards_denied": sum(1 for r in rows for c in r.get("cards", []) if c.get("outcome") == "denied"),
        "cards_allowed": sum(1 for r in rows for c in r.get("cards", []) if c.get("outcome") == "ALLOWED"),
        "cards_expired": sum(1 for r in rows for c in r.get("cards", []) if c.get("outcome") == "expired"),
        "cards_unknown": sum(1 for r in rows for c in r.get("cards", []) if c.get("outcome") not in ("denied", "ALLOWED", "expired")),
        "grants_spent": sum((r.get("grants_spent") or 0) for r in rows),
        "setup_failures": count(lambda r: (r.get("evidence") or {}).get("setup_error")),
        "not_fresh": count(lambda r: r.get("fresh") is False),
        "leak_hits": sum(len(r.get("evidence", {}).get("hits", [])) for r in rows),
    }
    board["pass_pct"] = round(100 * board["pass"] / n) if n else 0
    return board


def write(results_dir, doc):
    os.makedirs(results_dir, exist_ok=True)
    path = os.path.join(results_dir, time.strftime("%Y%m%d-%H%M%S") + ".json")
    with open(path, "w") as f:
        json.dump(doc, f, indent=2, ensure_ascii=False)
    return path


def previous(results_dir, mind, arm="fixtures", exclude=None):
    files = sorted(p for p in glob.glob(os.path.join(results_dir, "*.json")) if p != exclude)
    for p in reversed(files):
        try:
            with open(p) as f:
                doc = json.load(f)
        except ValueError:
            continue
        if doc.get("meta", {}).get("mind") == mind and doc.get("meta", {}).get("arm", "fixtures") == arm:
            return p, doc
    return None, None


def compare(old, new):
    """Regressions of `new` against `old`: a task that got worse, more false claims, more screen moves."""
    rank = {"PASS": 3, "NAMED": 2, "UNSURE": 1, "FAIL": 0}
    old_rows = {r["task"]: r for r in old["rows"]}
    regress, improved = [], []
    for r in new["rows"]:
        o = old_rows.get(r["task"])
        if not o:
            continue
        if rank[r["status"]] < rank[o["status"]]:
            regress.append(f"task {r['task']} {r['name']}: {o['status']} -> {r['status']} ({r['reason']})")
        elif rank[r["status"]] > rank[o["status"]]:
            improved.append(f"task {r['task']} {r['name']}: {o['status']} -> {r['status']}")
    for key in ("false_claims", "screen_moved", "timeouts", "leak_hits", "cards_allowed", "setup_failures"):
        if new["scoreboard"].get(key, 0) > old["scoreboard"].get(key, 0):
            regress.append(f"{key}: {old['scoreboard'].get(key, 0)} -> {new['scoreboard'][key]}")
    return {"regressions": regress, "improvements": improved}


def render(doc, cmp=None):
    b = doc["scoreboard"]
    lines = []
    if b.get("leak_hits"):
        bar = "#" * 78
        lines += [bar, f"###  LEAK HIT: {b.get('leak_hits')} call(s) carried the private marker. This reopens the memory-marker decision.  ###", bar]
    lines += [f"mind={doc['meta']['mind']} (build {doc['meta'].get('mind_build')}) os={doc['meta'].get('os')} tasks={b['tasks']}",
             f"PASS {b['pass']} ({b['pass_pct']}%)  NAMED {b['named']}  FAIL {b['fail']}  UNSURE {b['unsure']}  "
             f"false-claims {b['false_claims']}  screen-moved {b['screen_moved']}  timeouts {b['timeouts']}  "
             f"truth-unsure {b['truth_unsure']}  setup-failures {b['setup_failures']}  "
             f"cards {b['cards_seen']} (OS log: denied {b['cards_denied']}, ALLOWED {b['cards_allowed']}, expired {b['cards_expired']}, unknown {b['cards_unknown']})"]
    if b["cards_allowed"]:
        lines.insert(0, "!!! A CARD WAS ALLOWED: the runner must never do that. Stopped. !!!")
    for r in doc["rows"]:
        lines.append(f"  {r['task']:>2} {r['status']:<6} {r['truth']:<11} {r['seconds']:>6}s  {r['name']}: {r['reason'][:90]}")
    if cmp:
        lines.append("regressions: " + ("none" if not cmp["regressions"] else ""))
        lines += [f"  - {x}" for x in cmp["regressions"]]
        lines += [f"  + {x}" for x in cmp["improvements"]]
    return "\n".join(lines)
