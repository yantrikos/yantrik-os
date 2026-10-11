"""A stop asked for by SIGTERM always ends the program, however the signal's timing falls.

A handler that called `threading.Event.set()` could land while the main thread was inside
`Event.wait()` holding the event's lock, and wait on that lock for ever: the LibreOffice adapter's
clean-stop test timed out in CI (5 Oct 2026) after the handler had already been moved before the
bind. `StopOnSignal`'s handler only sets a flag. These tests send the stop at the worst moment
many times over, and keep any handler from going back to taking a lock.
"""

import os
import re
import signal
import subprocess
import sys
import textwrap
import unittest

import support

from yantrik_surface import StopOnSignal

LOOP = textwrap.dedent("""
    import sys
    sys.path.insert(0, %(package)r)
    from yantrik_surface import StopOnSignal
    stop = StopOnSignal()
    print("ready", flush=True)
    while not stop.wait(0.0005):   # a tick this short keeps the loop inside wait() nearly always
        pass
    print("stopped", flush=True)
""")


class StopOnSignalTest(unittest.TestCase):
    def test_a_signal_sets_the_flag_and_restore_puts_back_the_old_handler(self):
        before = signal.getsignal(signal.SIGTERM)
        stop = StopOnSignal((signal.SIGTERM,))
        try:
            self.assertFalse(stop.wait(0))
            os.kill(os.getpid(), signal.SIGTERM)
            self.assertTrue(stop.wait(1.0))
        finally:
            stop.restore()
        self.assertEqual(signal.getsignal(signal.SIGTERM), before)

    def test_a_stop_sent_into_a_busy_wait_loop_always_ends_it(self):
        code = LOOP % {"package": support.PACKAGE_ROOT}
        for _ in range(40):
            program = subprocess.Popen([sys.executable, "-c", code], stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
            self.assertEqual(program.stdout.readline().strip(), "ready")
            program.send_signal(signal.SIGTERM)
            try:
                out, err = program.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                program.kill()
                program.communicate()
                self.fail("a SIGTERM did not end the loop: the handler is waiting on a lock")
            self.assertEqual(program.returncode, 0, err)
            self.assertIn("stopped", out)

    def test_no_signal_handler_in_the_repo_sets_a_threading_event(self):
        # A handler installed as a lambda that calls an event's set() is the shape that deadlocked.
        shape = re.compile(r"signal\.signal\([^)]*lambda[^:]*:\s*\w+\.set\(\)")
        found = []
        for top in ("adapters", "apps", "sdk", "templates", "harnesses", "docs"):
            for root, _, files in os.walk(os.path.join(support.REPO, top)):
                for name in files:
                    if not name.endswith(".py"):
                        continue
                    path = os.path.join(root, name)
                    with open(path, encoding="utf-8", errors="replace") as f:
                        for number, line in enumerate(f, 1):
                            if shape.search(line):
                                found.append("%s:%d" % (os.path.relpath(path, support.REPO), number))
        self.assertEqual(found, [], "use yantrik_surface.StopOnSignal instead")


if __name__ == "__main__":
    unittest.main()
