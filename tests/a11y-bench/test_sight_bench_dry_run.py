"""The sight bench acts on, names and records only the demo's own windows (#1257 review, M4).

The bench times the accessibility service against a demo window. It runs on a person's desktop,
where the screen is theirs: it must never bring their window to the front, and its report must
not say what else was open or what they did in it. This runs the bench against a stub service
that has a person's window and a person's typing in it, a stub demo app and a stub `wlrctl`,
and reads back what the bench did and what it wrote.

Python 3 standard library only; it starts nothing but its own stubs. Run it with:

    python3 -m unittest discover -s tests/a11y-bench -v
"""

import json
import os
import socketserver
import stat
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parents[2] / "services" / "a11y-service" / "bench"
BENCH = BENCH_DIR / "sight-bench.py"

# What is on the person's screen, and what they did there. None of it may reach the report.
PERSON = {"app": "firefox", "title": "Bank - SECRET-BALANCE", "role": "frame"}
DEMO = {"app": "sight-demo-app.py", "role": "frame", "id": "d1", "title": "sight-demo: Expenses", "pid": 4242}
OTHER = {"app": "sight-demo-app.py", "role": "frame", "id": "d2", "title": "sight-demo: Other", "pid": 4242}

EVENTS = [
    {"seq": 0, "at_ms": 1000, "kind": "window_opened", "window": {**PERSON, "id": "p1", "pid": 1}},
    {"seq": 1, "at_ms": 1500, "kind": "content_changed", "window": "p1", "app": "firefox", "what": ["text"]},
    {"seq": 2, "at_ms": 2000, "kind": "window_opened", "window": DEMO},
    {"seq": 3, "at_ms": 2100, "kind": "window_opened", "window": OTHER},
    {"seq": 4, "at_ms": 2500, "kind": "focus_moved", "window": "p1", "role": "text", "name": "Search SECRET-QUERY"},
]


def now_state():
    return {
        # The cursor the bench starts reading from: everything above is "after" it.
        "seq": 0,
        "listening": True,
        "front": {**PERSON, "id": "p1", "pid": 1},
        "focus": {"window": "p1", "role": "text", "name": "Search SECRET-QUERY"},
        "windows": [{**PERSON, "id": "p1", "pid": 1}, DEMO, OTHER],
        "counts": {"model": {"told": 5, "listings": 1, "gaps": 0, "refused_windows": 0}, "signals_dropped": 0, "dropped_private": 0, "slow_marks": 0, "overwritten": 0},
    }


class Stub(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        for line in self.rfile:
            request = json.loads(line)
            method, params = request["method"], request.get("params") or {}
            if method == "a11y.now":
                result = now_state()
            elif method == "a11y.since":
                result = {"events": [e for e in EVENTS if e["seq"] >= params.get("seq", 0)], "next_seq": len(EVENTS), "missed": 0, "more": False, "reset": False}
                threading.Event().wait(0.01)
            elif method == "a11y.windows":
                result = {"windows": now_state()["windows"]}
            elif method == "a11y.describe":
                result = {"summary": "a window", "elements": []}
            else:
                result = {}
            self.wfile.write(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode() + b"\n")


DEMO_APP = '''
import sys, time
for line in sys.stdin:
    command = line.split()[0] if line.split() else ""
    stamp = lambda name: print("ACT %s %d" % (name, time.time() * 1000), flush=True)
    stamp(command)
    if command == "burst":
        stamp("burst-start"); stamp("burst-end")
    if command == "type-password":
        stamp("password-start"); stamp("password-end")
    if command == "quit":
        break
'''

# The bench's own clock, run fast: a sleep moves a clock the bench reads instead of waiting.
HARNESS = '''
import runpy, sys, time
real_sleep, real_time, skew = time.sleep, time.time, [0.0]
time.sleep = lambda seconds: (skew.__setitem__(0, skew[0] + seconds), real_sleep(min(seconds, 0.002)))[1]
time.time = lambda: real_time() + skew[0]
sys.argv = sys.argv[1:]
runpy.run_path(sys.argv[0], run_name=__RUN_NAME__)
'''


class Bench(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        root = Path(self.dir.name)
        self.sock = root / "a11y.sock"
        self.server = Stub(str(self.sock), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.app = root / "demo.py"
        self.app.write_text(DEMO_APP)
        self.log = root / "wlrctl.log"
        bin_dir = root / "bin"
        bin_dir.mkdir()
        wlrctl = bin_dir / "wlrctl"
        wlrctl.write_text(f'#!/bin/sh\necho "$@" >> {self.log}\n')
        wlrctl.chmod(wlrctl.stat().st_mode | stat.S_IXUSR)
        self.env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "SIGHT_BENCH_DRY_RUN": "1"}
        self.env.pop("SIGHT_PRIVACY_FILE", None)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.dir.cleanup()

    def harness(self, run_name):
        return [sys.executable, "-c", HARNESS.replace("__RUN_NAME__", repr(run_name)), str(BENCH), str(self.sock), str(self.app), str(os.getpid())]

    def run_bench(self):
        done = subprocess.run(self.harness("__main__"), env=self.env, capture_output=True, text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        return done.stdout, json.loads(done.stdout)

    def test_wlrctl_is_only_ever_given_a_window_of_the_demo(self):
        _, report = self.run_bench()
        calls = self.log.read_text().splitlines()
        self.assertTrue(calls, "the bench switched windows")
        for call in calls:
            self.assertRegex(call, r"^toplevel focus title:sight-demo", call)
        self.assertTrue(any("Other" in call for call in calls), "it switched to the demo's other window")
        self.assertEqual(report["wlrctl_targets"], [f"title:{t}" for t in ("sight-demo: Other", "sight-demo: Expenses (edited)")])

    def test_the_report_holds_no_application_but_the_demos_and_no_time_of_anyone_elses(self):
        text, report = self.run_bench()
        for private in ("firefox", "SECRET-BALANCE", "SECRET-QUERY", "yantrik-notes", "p1"):
            self.assertNotIn(private, text)
        self.assertIsInstance(report["windows_at_start"], int, "a count, not a list of what is open")
        self.assertEqual(report["windows_at_start"], 3)
        self.assertEqual(report["events_that_were_not_the_demos"], 3, "counted: the person's window, their typing, their focus")
        self.assertTrue(report["events"] and all("sight-demo" in json.dumps(e) for e in report["events"]))

    def test_a_window_that_is_not_the_demos_is_refused_as_a_target(self):
        code = (
            "import runpy, sys\n"
            "sys.argv = sys.argv[1:]\n"
            "ns = runpy.run_path(sys.argv[0], run_name='bench')\n"
            "try:\n    ns['switch_to']('firefox')\nexcept ValueError:\n    print('refused')\n"
        )
        probe = [sys.executable, "-c", code, str(BENCH), str(self.sock), str(self.app), str(os.getpid())]
        done = subprocess.run(probe, env=self.env, capture_output=True, text=True, timeout=60)
        self.assertIn("refused", done.stdout, done.stderr)
        self.assertFalse(self.log.exists(), "and wlrctl was never run")


if __name__ == "__main__":
    unittest.main()
