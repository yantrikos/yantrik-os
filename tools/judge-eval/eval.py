#!/usr/bin/env python3
"""Which judge should decide whether a press is a commitment? Every candidate, one question, the
same test set (cases.py), scored the same way.

    python3 tools/judge-eval/eval.py wordlist
    python3 tools/judge-eval/eval.py systemone http://127.0.0.1:8009 kev-latest
    python3 tools/judge-eval/eval.py jev                      # TypeSafe's cloud, through bin/jev
    python3 tools/judge-eval/eval.py ... --out results/kev.json

What matters most is recall on commitments — a missed one is a purchase or a send nobody was
asked about — then precision, since every false alarm is a card the person did not need, and
then speed, since this runs before a press.
"""

import concurrent.futures
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path[:0] = [HERE, os.path.join(HERE, "..", "..", "apps", "browser")]

from cases import CASES, QUESTION, state_of  # noqa: E402

JEV = os.environ.get("JEV_TOOL", "F:/yantrik/bin/jev")


def ask_wordlist(case):
    from yantrik_browser import driver
    cid, lang, gold, role, label, title, url, heading, nearby = case
    element = {"role": role, "name": label}
    if role == "link":
        element["href"] = url
    return 1.0 if driver.commitment_of(element) else 0.0


def request_body(case, model):
    return {"model": model, "state": state_of(case),
            "questions": {"commit": {"type": "noul", "instructions": QUESTION}}}


def ask_systemone(endpoint, model):
    url = endpoint.rstrip("/")
    if not url.endswith("/v1/systemone"):
        url += "/v1/systemone"

    def ask(case):
        data = json.dumps(request_body(case, model)).encode()
        for attempt in range(8):
            req = urllib.request.Request(url, data=data, headers={"content-type": "application/json"})
            try:
                with urllib.request.urlopen(req, timeout=120) as r:
                    got = json.load(r)
                return float(got["answers"]["commit"]["noul"])
            except urllib.error.HTTPError as e:
                # 529: a server that answers one request at a time and is busy with another.
                if e.code not in (429, 529, 503) or attempt == 7:
                    raise
                time.sleep(0.25 * (attempt + 1))
    return ask


def ask_jev(case):
    body = json.dumps(request_body(case, "jev-latest"))
    out = subprocess.run([sys.executable, JEV], input=body, capture_output=True, text=True, timeout=120)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip()[-300:])
    return float(json.loads(out.stdout)["answers"]["commit"]["noul"])


def score(results, threshold):
    tp = sum(1 for r in results if r["gold"] and r["p"] >= threshold)
    fn = sum(1 for r in results if r["gold"] and r["p"] < threshold)
    fp = sum(1 for r in results if not r["gold"] and r["p"] >= threshold)
    tn = sum(1 for r in results if not r["gold"] and r["p"] < threshold)
    recall = tp / (tp + fn) if tp + fn else 0.0
    precision = tp / (tp + fp) if tp + fp else 0.0
    return {"threshold": threshold, "recall": round(recall, 3), "precision": round(precision, 3),
            "missed": fn, "false_alarms": fp, "tp": tp, "tn": tn}


def by_group(results, threshold):
    groups = {}
    for r in results:
        g = r["id"].split("-")[0]
        groups.setdefault(g, []).append(r)
    return {g: "%d/%d right" % (sum(1 for r in rs if (r["p"] >= threshold) == r["gold"]), len(rs))
            for g, rs in sorted(groups.items())}


def main():
    args = [a for i, a in enumerate(sys.argv[1:], 1)
            if not a.startswith("--") and sys.argv[i - 1] not in ("--out", "--workers")]
    out_path = sys.argv[sys.argv.index("--out") + 1] if "--out" in sys.argv else None
    kind = args[0] if args else "wordlist"
    if kind == "wordlist":
        ask, name, workers = ask_wordlist, "wordlist", 1
    elif kind == "systemone":
        ask, name, workers = ask_systemone(args[1], args[2]), "%s@%s" % (args[2], args[1]), 4
    elif kind == "jev":
        ask, name, workers = ask_jev, "jev-latest", 6
    else:
        raise SystemExit(__doc__)

    def one(case):
        started = time.monotonic()
        try:
            p = ask(case)
            err = None
        except Exception as e:  # noqa: BLE001 — a failed question is counted, not fatal
            p, err = 0.0, str(e)[:200]
        return {"id": case[0], "lang": case[1], "gold": case[2], "label": case[4], "p": p,
                "ms": round((time.monotonic() - started) * 1000), "error": err}

    if "--workers" in sys.argv:
        workers = int(sys.argv[sys.argv.index("--workers") + 1])
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        results = list(pool.map(one, CASES))
    errors = [r for r in results if r["error"]]
    ms = sorted(r["ms"] for r in results)
    summary = {
        "judge": name,
        "cases": len(results),
        "errors": len(errors),
        "latency_ms_p50": ms[len(ms) // 2],
        "latency_ms_p95": ms[int(len(ms) * 0.95) - 1],
        "at_0.5": score(results, 0.5),
        "at_0.3": score(results, 0.3),
        "groups_at_0.5": by_group(results, 0.5),
        "missed_at_0.5": [r["id"] for r in results if r["gold"] and r["p"] < 0.5],
        "false_alarms_at_0.5": [r["id"] for r in results if not r["gold"] and r["p"] >= 0.5],
    }
    if errors:
        summary["first_error"] = errors[0]["error"]
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    if out_path:
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump({"summary": summary, "results": results}, f, indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
