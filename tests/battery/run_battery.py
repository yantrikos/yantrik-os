#!/usr/bin/env python3
"""Yantrik OS end-to-end battery. Runs ON a test machine, as root or the person, with no LLM and no tokens.

  run_battery.py --mind mind                 run all 19 tasks with the Yantrik Mind answering
  run_battery.py --mind hermes --tasks 2,4   a subset, Hermes answering
  run_battery.py --snapshot-baseline         save the current sample world as the baseline (once)
  run_battery.py --list                      the tasks

It refuses to run unless /etc/yantrik/test-machine exists (root's, containing 'yantrik-test-machine')."""
import argparse
import datetime
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from battery import adapters, checks, facts, leak, lens, report, tasks as T, truth, ui, world  # noqa: E402
from battery import machine as mach  # noqa: E402
from battery.verdict import Verdict  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))


def log(msg):
    print(time.strftime("%H:%M:%S"), msg, flush=True)


def read_before(w):
    t = w.today()
    return {"events": w.events(t - datetime.timedelta(days=1), t + datetime.timedelta(days=21)),
            "notes": w.notes(), "desktop": w.desktop(), "apt": w.apt_history_size(), "dnd": w.dnd()}


def os_versions(m):
    def line(path, key):
        try:
            for l in open(path):
                if l.startswith(key):
                    return l.split("=", 1)[1].strip()
        except OSError:
            return None
    return {"os": line("/opt/yantrik/BUILD", "version="), "mind": line("/opt/yantrik-mind/BUILD", "commit=")}


def restart_hermes(m):
    m.run(["systemctl", "--user", "restart", "hermes-gateway"], timeout=60)
    time.sleep(15)


def run_leak_task(m, w, adapter, task, args):
    marker, v, a, b = leak.run_leak(m, w, adapter, log=log, timeout=min(int(task.timeout * args.timeout_scale), args.max_task_seconds), wait_quiet=not args.no_wait_quiet)
    cards = (a.get("cards") or []) + (b.get("cards") or [])
    row = {"task": task.id, "name": task.name, "seconds": round((a.get("seconds") or 0) + (b.get("seconds") or 0), 1),
           "timed_out": bool(a.get("timed_out") or b.get("timed_out")), "fresh": (b.get("fresh") or {}).get("ok"),
           "fresh_detail": (b.get("fresh") or {}).get("detail"), "cards": cards, "reply": (b.get("reply") or "")[:600],
           "tools": [t for r in (a, b) for t in (r.get("trace") or {}).get("tools", [])],
           "refusals": [t for r in (a, b) for t in (r.get("trace") or {}).get("refusals", [])],
           "screen_before": a.get("screen_before"), "screen_after": b.get("screen_after") or a.get("screen_after"),
           "status": v.status, "reason": v.reason, "evidence": v.evidence}
    row["screen_moved"] = checks.screen_moved(row["screen_before"], row["screen_after"])
    row["truth"] = "-"
    return row, b


def run_task(m, w, adapter, task, args):
    if task.special == "leak":
        return run_leak_task(m, w, adapter, task, args)
    before = read_before(w)
    res = lens.run_turn(m, adapter, task.prompt, fresh=not task.same_chat, timeout=min(int(task.timeout * args.timeout_scale), args.max_task_seconds),
                        stop_after=task.stop_after, log=log, wait_quiet=not args.no_wait_quiet)
    row = {"task": task.id, "name": task.name, "seconds": res.get("seconds", 0), "timed_out": bool(res.get("timed_out")),
           "fresh": (res.get("fresh") or {}).get("ok") if not task.same_chat else None,
           "fresh_detail": (res.get("fresh") or {}).get("detail"), "cards": res.get("cards", []),
           "reply": (res.get("reply") or "")[:1200], "tools": (res.get("trace") or {}).get("tools", []),
           "refusals": (res.get("trace") or {}).get("refusals", []), "screen_before": res.get("screen_before"),
           "screen_after": res.get("screen_after")}
    row["screen_moved"] = checks.screen_moved(res.get("screen_before"), res.get("screen_after"))
    if res.get("setup_error"):
        v = Verdict("FAIL", "setup: " + res["setup_error"], {"setup_error": True})
    elif res.get("timed_out"):
        v = Verdict("FAIL", f"the turn did not finish within {task.timeout}s", {"timed_out": True})
    else:
        try:
            v = task.verify(T.Ctx(w, res, before))
        except Exception as e:  # a broken verifier is a FAIL with its reason, never a crash of the run
            v = Verdict("FAIL", f"verifier error: {e!r}", {})
    row.update(status=v.status, reason=v.reason, evidence=v.evidence,
               truth=truth.check(res.get("reply"), v.status, v.evidence) if not (res.get("setup_error") or res.get("timed_out")) else "-")
    return row, res


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mind", choices=["mind", "hermes"])
    ap.add_argument("--tasks", help="comma separated task ids (default: all)")
    ap.add_argument("--user", default="tester")
    ap.add_argument("--state-dir", default=world.STATE)
    ap.add_argument("--results-dir", default=os.path.join(HERE, "results"))
    ap.add_argument("--snapshot-baseline", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--no-reset", action="store_true", help="do not reset the sample world first (debugging only)")
    ap.add_argument("--no-pick", action="store_true", help="do not try to select the mind; require it to be answering")
    ap.add_argument("--no-wait-quiet", action="store_true", help="refuse instead of waiting out 03:00-04:30")
    ap.add_argument("--max-task-seconds", type=int, default=1800)
    ap.add_argument("--timeout-scale", type=float, default=None, help="multiply every task timeout (default 1 for the Mind, 2 for Hermes, which is slower)")
    ap.add_argument("--compare", nargs="?", const="previous", help="compare with a results file (default: the previous run of this mind)")
    a = ap.parse_args(argv)

    if a.list:
        for t in T.TASKS:
            print(f"{t.id:>2} {t.name}: {t.prompt}")
        return 0
    m = mach.Machine(a.user)
    try:
        m.assert_test_machine()
    except mach.NotATestMachine as e:
        print("REFUSED:", e, file=sys.stderr)
        return 2
    if a.snapshot_baseline:
        print("baseline saved to", world.snapshot(m, a.state_dir))
        return 0
    if not a.mind:
        ap.error("--mind is required")

    a.timeout_scale = a.timeout_scale or (2.0 if a.mind == "hermes" else 1.0)
    w = facts.World(m)
    adapter = adapters.adapter_for(a.mind, m)
    if a.mind not in m.shell_state()["answering"]:
        if a.no_pick:
            print(f"REFUSED: {a.mind} is not the answering mind", file=sys.stderr)
            return 2
        ok, why = ui.pick_mind(m, a.mind, {})
        log(f"select mind {a.mind}: {why}")
        if not ok:
            return 2
    changes = [] if a.no_reset else world.reset(m, a.state_dir)
    for c in changes:
        log("world: " + c)
    ids = [int(x) for x in a.tasks.split(",")] if a.tasks else [t.id for t in T.TASKS]
    order = [t for t in T.TASKS if t.id == 19 and 19 in ids] + [t for t in T.TASKS if t.id in ids and t.id != 19]
    rows, canary_failed = [], False
    for task in order:
        log(f"task {task.id}: {task.name}")
        row, res = run_task(m, w, adapter, task, a)
        rows.append(row)
        log(f"  -> {row['status']} {row['truth']} {row['seconds']}s {row['reason'][:100]}")
        if task.id == 19 and row["status"] != "PASS":
            log("the canary failed: the harness is broken, not the OS; stopping")
            canary_failed = True
            break
        if a.mind == "hermes" and (res.get("asked_clarify") or res.get("timed_out")):
            restart_hermes(m)
    rows.sort(key=lambda r: r["task"])
    doc = {"meta": {"when": time.strftime("%Y-%m-%dT%H:%M:%S"), "mind": a.mind, "host": os.uname().nodename,
                    **os_versions(m)}, "world_changes": changes, "rows": rows, "scoreboard": report.scoreboard(rows)}
    path = report.write(a.results_dir, doc)
    cmp = None
    if a.compare:
        oldp, old = (a.compare, json.load(open(a.compare))) if a.compare != "previous" else report.previous(a.results_dir, a.mind, path)
        if old:
            cmp = report.compare(old, doc)
            doc["compared_with"] = oldp
            doc["comparison"] = cmp
            json.dump(doc, open(path, "w"), indent=2, ensure_ascii=False)
        else:
            log("--compare: no previous run of this mind")
    print(report.render(doc, cmp))
    print("results:", path)
    if canary_failed:
        return 3
    if doc["scoreboard"].get("leak_hits"):
        return 4
    return 1 if cmp and cmp["regressions"] else 0


if __name__ == "__main__":
    sys.exit(main())
