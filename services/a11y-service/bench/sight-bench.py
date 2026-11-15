#!/usr/bin/env python3
"""Time the screen model against a window that says when it changed.

usage: sight-bench.py <a11y.sock> <sight-demo-app.py> <service pid>
Prints one JSON report on stdout. Reads no text out of any window but the demo's own.
"""
import json
import os
import socket
import subprocess
import sys
import threading
import time

SOCK, APP, PID = sys.argv[1], sys.argv[2], int(sys.argv[3])
TICK = os.sysconf("SC_CLK_TCK")


def now_ms():
    return int(time.time() * 1000)


class Rpc:
    """One connection, one JSON object per line."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(SOCK)
        self.file = self.sock.makefile("rwb")
        self.n = 0

    def call(self, method, params=None):
        self.n += 1
        line = json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}})
        started = time.perf_counter()
        self.file.write(line.encode() + b"\n")
        self.file.flush()
        reply = self.file.readline()
        took_ms = (time.perf_counter() - started) * 1000
        answer = json.loads(reply)
        if "error" in answer and answer["error"]:
            raise RuntimeError(f"{method}: {answer['error']}")
        return answer.get("result"), took_ms, len(reply)


def cpu_ms(pid):
    with open(f"/proc/{pid}/stat") as stat:
        fields = stat.read().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) * 1000 // TICK


def spread(values):
    values = sorted(values)
    pick = lambda q: round(values[min(len(values) - 1, int(len(values) * q))], 2)
    return {"n": len(values), "p50": pick(0.5), "p95": pick(0.95), "max": round(values[-1], 2)}


def timed(rpc, method, params, times):
    took, size = [], 0
    for _ in range(times):
        _, ms, size = rpc.call(method, params)
        took.append(ms)
    return {"ms": spread(took), "bytes": size}


events = []  # (event, received at ms)
stop = threading.Event()


def follow(seq):
    rpc = Rpc()
    while not stop.is_set():
        page, _, _ = rpc.call("a11y.since", {"seq": seq, "wait_ms": 2000})
        received = now_ms()
        for event in page["events"]:
            events.append((event, received))
        seq = page["next_seq"]


acts = []  # (name, ms)


def read_acts(pipe):
    for line in pipe:
        parts = line.split()
        if len(parts) == 3 and parts[0] == "ACT":
            acts.append((parts[1], int(parts[2])))


def act(name):
    acts.append((name, now_ms()))


def first_after(name, wanted, slack_ms=5):
    """The first event after act `name` that `wanted` accepts: (told lag, seen lag) in ms."""
    at = next((ms for act_name, ms in acts if act_name == name), None)
    if at is None:
        return None
    for event, received in events:
        if event["at_ms"] >= at - slack_ms and wanted(event):
            return {"told_after_ms": event["at_ms"] - at, "seen_after_ms": received - at}
    return {"told_after_ms": None, "seen_after_ms": None}


def title(event):
    return ((event.get("window") or {}).get("title") or "") if isinstance(event.get("window"), dict) else ""


def main():
    report = {}
    rpc = Rpc()
    start, _, _ = rpc.call("a11y.now")
    report["listening"] = start["listening"]
    report["windows_at_start"] = [f"{w['app']} ({w['role']})" for w in start["windows"]]

    # 1. Idle: what listening costs when nothing happens.
    before = cpu_ms(PID)
    time.sleep(15)
    report["idle"] = {"seconds": 15, "service_cpu_ms": cpu_ms(PID) - before}

    # 1b. The same quiet screen, watched the old way: asking for the windows ten times a second.
    before, asked, until = cpu_ms(PID), 0, time.time() + 15
    while time.time() < until:
        rpc.call("a11y.windows")
        asked += 1
        time.sleep(0.1)
    report["polling_10hz"] = {"seconds": 15, "asked": asked, "service_cpu_ms": cpu_ms(PID) - before}

    reader = threading.Thread(target=follow, args=(start["seq"],), daemon=True)
    reader.start()

    env = dict(os.environ)
    app = subprocess.Popen([sys.executable, APP], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)
    threading.Thread(target=read_acts, args=(app.stdout,), daemon=True).start()

    def tell(command, pause=0.8):
        app.stdin.write(command + "\n")
        app.stdin.flush()
        time.sleep(pause)

    time.sleep(2.5)
    # The text field has the focus when the window opens, so the button goes first.
    tell("focus-button")
    tell("focus-entry")
    tell("text Lunch at Rasa")
    tell("title sight-demo: Expenses (edited)")
    tell("dialog", 1.2)
    tell("close-dialog", 1.0)
    act("front-notes")
    subprocess.run(["wlrctl", "toplevel", "focus", "app_id:yantrik-notes"], check=False)
    time.sleep(1.0)
    act("front-demo")
    subprocess.run(["wlrctl", "toplevel", "focus", "title:sight-demo: Expenses (edited)"], check=False)
    time.sleep(1.0)

    # 2. The camera, for comparison: looking when asked.
    shown, _, _ = rpc.call("a11y.now")
    demo = next((w for w in shown["windows"] if w["title"].startswith("sight-demo: Expenses")), None)
    report["now"] = timed(rpc, "a11y.now", None, 300)
    report["windows_pull"] = timed(rpc, "a11y.windows", None, 20)
    if demo:
        report["describe_pull"] = timed(rpc, "a11y.describe", {"window": demo["id"]}, 10)
        listed, _, _ = rpc.call("a11y.windows")
        report["handle_is_stable"] = any(w["id"] == demo["id"] for w in listed["windows"])
        described, _, _ = rpc.call("a11y.describe", {"window": demo["id"]})
        report["describe_summary"] = described["summary"]
    report["front_now"] = (shown.get("front") or {}).get("title")
    report["focus_now"] = shown.get("focus")

    # 3. A firehose: one window changing its text as fast as its toolkit will announce it.
    counts_before = shown["counts"]
    cpu_before, events_before = cpu_ms(PID), len(events)
    tell("burst 2000", 0)
    deadline = time.time() + 30
    while time.time() < deadline and not any(name == "burst-end" for name, _ in acts):
        time.sleep(0.05)
    time.sleep(0.5)
    after, _, _ = rpc.call("a11y.now")
    span = [ms for name, ms in acts if name in ("burst-start", "burst-end")]
    report["burst"] = {
        "changes_made": 2000,
        "took_ms": span[1] - span[0] if len(span) == 2 else None,
        "signals_taken_in": after["counts"]["signals"] - counts_before["signals"],
        "signals_dropped": after["counts"]["signals_dropped"] - counts_before["signals_dropped"],
        "events_told": len(events) - events_before,
        "folded": after["counts"]["model"]["folded"] - counts_before["model"]["folded"],
        "service_cpu_ms": cpu_ms(PID) - cpu_before,
    }

    tell("quit", 1.5)
    stop.set()
    reader.join(timeout=4)
    app.wait(timeout=5)

    kind = lambda k: (lambda e: e["kind"] == k)
    report["latency_ms"] = {
        "window opened (from the toolkit mapping it)": first_after("mapped", lambda e: e["kind"] == "window_opened" and title(e).startswith("sight-demo: Expenses")),
        "focus moved to Save": first_after("focus-button", lambda e: e["kind"] == "focus_moved" and e["name"] == "Save"),
        "focus moved to the text field": first_after("focus-entry", lambda e: e["kind"] == "focus_moved" and e["role"] == "text"),
        "text typed": first_after("text", lambda e: e["kind"] == "content_changed" and "text" in e["what"]),
        "window retitled": first_after("title", kind("window_renamed")),
        "dialog opened": first_after("dialog", lambda e: e["kind"] == "window_opened" and "Confirm" in title(e)),
        "dialog closed": first_after("close-dialog", kind("window_closed")),
        "another app took the front (wlrctl)": first_after("front-notes", kind("front_changed")),
        "the demo took the front back (wlrctl)": first_after("front-demo", kind("front_changed")),
        "window closed": first_after("quit", lambda e: e["kind"] == "window_closed" and title(e).startswith("sight-demo: Expenses")),
    }
    report["acts"] = acts
    # The demo's own events, whole; anything else by kind and application only.
    demo_ids = {e["window"]["id"] for e, _ in events if e["kind"] == "window_opened" and "sight-demo" in e["window"]["title"]}
    mine = lambda e: "sight-demo" in json.dumps(e) or e.get("app") == "sight-demo-app.py" or (isinstance(e.get("window"), str) and e["window"] in demo_ids)
    report["events"] = [e if mine(e) else {"seq": e["seq"], "at_ms": e["at_ms"], "kind": e["kind"], "app": (e.get("window") or {}).get("app") if isinstance(e.get("window"), dict) else e.get("app")} for e, _ in events]
    report["counts_at_end"] = after["counts"]
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
