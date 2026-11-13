#!/usr/bin/env python3
"""Yantrik OS end-to-end battery. Runs ON a test machine, as root, with no LLM and no tokens.

  run_battery.py --mind mind                 run all 20 tasks with the Yantrik Mind answering
  run_battery.py --mind hermes --tasks 2,4   a subset, Hermes answering
  run_battery.py --snapshot-baseline         save the current sample world as the baseline (once)
  run_battery.py --list                      the tasks

It refuses unless /etc/yantrik/no-person-data exists (the marker #1080 defines: root's regular file, not group or
world writable, in a root-owned directory) and --user is a test account.

Exit codes: 0 clean; 1 regression against --compare; 2 refused or setup failed; 3 the canary failed; 4 the leak
test found its marker; 5 a card was ALLOWED (never happens by design); 6 false claims, setup failures or a leak
test that could not inspect anything; 7 the runner crashed (partial results are written); 8 below --min-pass."""
import argparse
import datetime
import glob
import json
import os
import sys
import time
import traceback

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from battery import adapters, arms, checks, facts, leak, lens, report, tasks as T, truth, ui, world  # noqa: E402
from battery import machine as mach  # noqa: E402
from battery.verdict import Verdict  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))


def log(msg):
    print(time.strftime("%H:%M:%S"), msg, flush=True)


def read_before(w):
    t = w.today()
    return {"events": w.events(t - datetime.timedelta(days=1), t + datetime.timedelta(days=21)),
            "notes": w.notes(), "desktop": w.desktop(), "apt": w.apt_history_size(), "dnd": w.dnd(),
            "csv": sorted(glob.glob(f"{w.home}/*.csv"))}


def os_versions(m):
    def line(path, key):
        try:
            with open(path) as f:
                for l in f:
                    if l.startswith(key):
                        return l.split("=", 1)[1].strip()
        except OSError:
            return None
    return {"os": line("/opt/yantrik/BUILD", "version="), "mind_build": line("/opt/yantrik-mind/BUILD", "commit=")}


def restart_hermes(m):
    m.run(["systemctl", "--user", "restart", "hermes-gateway"], timeout=60)
    time.sleep(15)


def base_row(task, res):
    cards = res.get("cards", [])
    return {"task": task.id, "name": task.name, "seconds": res.get("seconds", 0), "timed_out": bool(res.get("timed_out")),
            "cards": cards, "grants_spent": res.get("grants_spent"), "reply": (res.get("reply") or "")[:1200],
            "tools": (res.get("trace") or {}).get("tools", []), "refusals": (res.get("trace") or {}).get("refusals", []),
            "journal": (res.get("trace") or {}).get("journal", [])}


def run_leak_task(m, w, adapter, task, args):
    marker, v, a, b = leak.run_leak(m, w, adapter, log=log, timeout=min(int(task.timeout * args.timeout_scale), args.max_task_seconds),
                                    wait_quiet=not args.no_wait_quiet)
    res = {"cards": (a.get("cards") or []) + (b.get("cards") or []), "seconds": round((a.get("seconds") or 0) + (b.get("seconds") or 0), 1),
           "timed_out": bool(a.get("timed_out") or b.get("timed_out")), "reply": b.get("reply"),
           "grants_spent": (a.get("grants_spent") or 0) + (b.get("grants_spent") or 0),
           "trace": {"tools": [t for r in (a, b) for t in (r.get("trace") or {}).get("tools", [])],
                     "refusals": [t for r in (a, b) for t in (r.get("trace") or {}).get("refusals", [])],
                     "journal": ((b.get("trace") or {}).get("journal") or [])}}
    row = base_row(task, res)
    row.update(fresh=(b.get("fresh") or {}).get("ok"), fresh_detail=(b.get("fresh") or {}).get("detail"),
               screen_before=a.get("screen_before"), screen_after=b.get("screen_after") or a.get("screen_after"),
               status=v.status, reason=v.reason, evidence=v.evidence, truth="-")
    row["screen_moved"] = checks.screen_moved(row["screen_before"], row["screen_after"])
    return row, b


def run_task(m, w, adapter, task, args):
    if task.special == "leak":
        return run_leak_task(m, w, adapter, task, args)
    try:
        before = read_before(w)
    except Exception as e:  # the world could not be read: a FAIL row with the reason, never a dead run
        return {**base_row(task, {}), "status": "FAIL", "reason": f"setup: could not read the world before the task: {e!r}",
                "evidence": {"setup_error": True}, "truth": "-", "fresh": None, "screen_moved": None}, {}
    res = lens.run_turn(m, adapter, task.prompt, fresh=not task.same_chat, timeout=min(int(task.timeout * args.timeout_scale), args.max_task_seconds),
                        stop_after=task.stop_after, log=log, wait_quiet=not args.no_wait_quiet)
    row = base_row(task, res)
    row.update(fresh=(res.get("fresh") or {}).get("ok") if not task.same_chat else None, fresh_detail=(res.get("fresh") or {}).get("detail"),
               screen_before=res.get("screen_before"), screen_after=res.get("screen_after"))
    row["screen_moved"] = checks.screen_moved(res.get("screen_before"), res.get("screen_after"))
    if res.get("setup_error"):
        v = Verdict("FAIL", "setup: " + res["setup_error"], {"setup_error": True})
    elif res.get("timed_out"):
        v = Verdict("FAIL", f"the turn did not finish within the timeout", {"timed_out": True})
    else:
        try:
            v = task.verify(T.Ctx(w, res, before))
        except Exception as e:  # a broken verifier is a FAIL with its reason, never a crash of the run
            v = Verdict("FAIL", f"verifier error: {e!r}", {"verifier_error": True})
    row.update(status=v.status, reason=v.reason, evidence=v.evidence,
               truth=truth.check(res.get("reply"), v.status, v.evidence) if not (res.get("setup_error") or res.get("timed_out")) else "-")
    return row, res


def exit_code(doc, cmp, canary_failed, min_pass):
    b = doc["scoreboard"]
    if b["cards_allowed"]:
        return 5
    if b["leak_hits"]:
        return 4
    if doc.get("crashed"):
        return 7
    if canary_failed:
        return 3
    leak_row = next((r for r in doc["rows"] if r["task"] == 20), None)
    if b["false_claims"] or b["setup_failures"] or (leak_row and leak_row["status"] not in ("PASS", "NAMED")):
        return 6
    if cmp and cmp["regressions"]:
        return 1
    if min_pass is not None and b["pass_pct"] < min_pass:
        return 8
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mind", choices=["mind", "hermes"])
    ap.add_argument("--tasks", help="comma separated task ids (default: all)")
    ap.add_argument("--user", default="tester")
    ap.add_argument("--allow-user", action="store_true", help="accept a --user that is not named like a test account")
    ap.add_argument("--arm", choices=["fixtures", "clean"], default="fixtures",
                    help="clean: refuse unless the mind's memory holds none of the earlier tests' fixtures (a fresh install)")
    ap.add_argument("--state-dir", default=world.STATE)
    ap.add_argument("--results-dir", default=os.path.join(HERE, "results"))
    ap.add_argument("--snapshot-baseline", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--no-reset", action="store_true", help="do not reset the sample world first (debugging only)")
    ap.add_argument("--no-pick", action="store_true", help="do not try to select the mind; require it to be answering")
    ap.add_argument("--no-wait-quiet", action="store_true", help="refuse instead of waiting out 03:00-04:30")
    ap.add_argument("--max-task-seconds", type=int, default=1800)
    ap.add_argument("--timeout-scale", type=float, default=None, help="multiply every task timeout (default 1 for the Mind, 2 for Hermes)")
    ap.add_argument("--min-pass", type=int, default=None, help="exit 8 when fewer than this percent of tasks PASS")
    ap.add_argument("--compare", nargs="?", const="previous", help="compare with a results file (default: the previous run of this mind and arm)")
    a = ap.parse_args(argv)

    if a.list:
        for t in sorted(T.TASKS, key=lambda t: t.id):
            print(f"{t.id:>2} {t.name}: {t.prompt}")
        return 0
    m = mach.Machine(a.user)
    try:
        m.assert_test_machine(a.allow_user)
        m.require_root()
    except mach.NotATestMachine as e:
        print("REFUSED:", e, file=sys.stderr)
        return 2
    if a.snapshot_baseline:
        print("baseline saved to", world.snapshot(m, a.state_dir))
        return 0
    if not a.mind:
        ap.error("--mind is required")
    a.timeout_scale = a.timeout_scale or (2.0 if a.mind == "hermes" else 1.0)
    if a.arm == "clean":
        bad = arms.fixtures_present(m, a.mind)
        if bad:
            print("REFUSED: --arm clean needs a fresh install; " + "; ".join(f"{p} {why}" for p, why in bad), file=sys.stderr)
            return 2
    w = facts.World(m)
    adapter = adapters.adapter_for(a.mind, m)
    ok, why = adapter.readable()
    if not ok:
        print(f"REFUSED: cannot read the {a.mind} adapter's evidence ({why}); the battery would pass blind", file=sys.stderr)
        return 2
    if a.mind not in m.shell_state()["answering"]:
        if a.no_pick:
            print(f"REFUSED: {a.mind} is not the answering mind", file=sys.stderr)
            return 2
        ok, why = ui.pick_mind(m, a.mind, {})
        log(f"select mind {a.mind}: {why}")
        if not ok:
            return 2
    try:
        changes = [] if a.no_reset else world.reset(m, a.state_dir)
    except Exception as e:
        print("REFUSED: could not reset the sample world:", repr(e), file=sys.stderr)
        return 2
    for c in changes:
        log("world: " + c)
    ids = [int(x) for x in a.tasks.split(",")] if a.tasks else [t.id for t in T.TASKS]
    order = [t for t in T.TASKS if t.id == 19 and 19 in ids] + [t for t in sorted(T.TASKS, key=lambda t: t.id) if t.id in ids and t.id != 19]
    rows, canary_failed, crashed = [], False, None
    meta = {"when": time.strftime("%Y-%m-%dT%H:%M:%S"), "mind": a.mind, "arm": a.arm, "host": os.uname().nodename, **os_versions(m)}
    try:
        for task in order:
            log(f"task {task.id}: {task.name}")
            row, res = run_task(m, w, adapter, task, a)
            rows.append(row)
            log(f"  -> {row['status']} {row['truth']} {row['seconds']}s {row['reason'][:100]}")
            if any(c.get("outcome") == "ALLOWED" for c in row["cards"]):
                log("A CARD WAS ALLOWED. Stopping the run.")
                break
            if task.id == 19 and row["status"] != "PASS":
                log("the canary failed: the harness is broken, not the OS; stopping")
                canary_failed = True
                break
            if a.mind == "hermes" and (res.get("asked_clarify") or res.get("timed_out")):
                restart_hermes(m)
    except Exception:
        crashed = traceback.format_exc()
        log("the runner crashed; writing partial results")
    rows.sort(key=lambda r: r["task"])
    doc = {"meta": meta, "world_changes": changes, "rows": rows, "scoreboard": report.scoreboard(rows)}
    if crashed:
        doc["crashed"] = crashed
    path = report.write(a.results_dir, doc)
    cmp = None
    if a.compare:
        if a.compare == "previous":
            oldp, old = report.previous(a.results_dir, a.mind, a.arm, path)
        else:
            oldp = a.compare
            with open(oldp) as f:
                old = json.load(f)
        if old:
            cmp = report.compare(old, doc)
            doc["compared_with"], doc["comparison"] = oldp, cmp
            with open(path, "w") as f:
                json.dump(doc, f, indent=2, ensure_ascii=False)
        else:
            log("--compare: no previous run of this mind and arm")
    print(report.render(doc, cmp))
    print("results:", path)
    return exit_code(doc, cmp, canary_failed, a.min_pass)


if __name__ == "__main__":
    sys.exit(main())
