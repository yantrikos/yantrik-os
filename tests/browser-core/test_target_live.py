#!/usr/bin/env python3
"""The guard (#1085) against a real browser and the real /proc.

    python3 tests/browser-core/test_target_live.py

The browser and the site are test_live's. Skips (exit 0) without google-chrome or chromium.
"""

import functools
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [HERE, os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

import test_live  # noqa: E402
from yantrik_browser import cdp, driver as drv, target  # noqa: E402


def setUpModule():
    if test_live.chrome():
        test_live.Bench.start()


def tearDownModule():
    if test_live.chrome():
        test_live.Bench.stop()


def listening_on(port):
    for _ in range(150):
        try:
            return target.identify("http://127.0.0.1:%d" % port)
        except target.NoListener:
            time.sleep(0.1)
    raise AssertionError("nothing listened on %d" % port)


@unittest.skipUnless(test_live.chrome(), "needs google-chrome or chromium")
class WhichBrowser(unittest.TestCase):
    """The guard (#1085) against real Chromium and the real /proc. test_live's browser plays Mind
    View's: its profile directory is the one the guard is told is Mind View's. A second Chromium,
    with a profile of its own under a HOME of its own, is the person's."""

    @classmethod
    def setUpClass(cls):
        cls.home = tempfile.mkdtemp(prefix="yb-home-")
        cls.persons_port = test_live.free_port()
        cls.mind_view = test_live.Bench.tmp
        cls.persons_cdp = "http://127.0.0.1:%d" % cls.persons_port
        # The person's: the default profile, and arguments that claim to be Mind View's (a URL after `--`, which is an
        # address and not a flag).
        cls.persons = subprocess.Popen([
            test_live.chrome(), "--headless=new", "--no-first-run", "--no-default-browser-check",
            "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=%d" % cls.persons_port,
            "--remote-allow-origins=%s" % cls.persons_cdp, "--window-size=800,600",
            # (Chrome 136+ ignores the debugging port on its *default* directory, so the person's is
            # an explicit one here; a Chromium without that rule runs on the default.)
            "--user-data-dir=%s" % os.path.join(cls.home, ".config", "chromium"),
            "--", "about:blank?--user-data-dir=%s" % cls.mind_view],
            env=dict(os.environ, HOME=cls.home, XDG_CONFIG_HOME=os.path.join(cls.home, ".config")),
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        cls.ready = listening_on(cls.persons_port)

    @classmethod
    def tearDownClass(cls):
        cls.persons.kill()
        cls.persons.wait()
        shutil.rmtree(cls.home, ignore_errors=True)

    def driver(self, person, address):
        identify = functools.partial(target.identify, mind_profile=self.mind_view)
        guard = target.Guard(is_person=lambda: person)
        d = drv.Driver(cdp.Browser(address, identify=identify), judge=False, guard=guard)
        self.addCleanup(d.browser.close)
        return d

    def test_the_spoofing_arguments_are_really_in_the_persons_command_line(self):
        with open("/proc/%d/cmdline" % self.persons.pid, "rb") as f:
            self.assertIn(b"--user-data-dir=" + self.mind_view.encode(), f.read())

    def test_real_mind_view_like_chromium_is_identified_by_what_it_holds(self):
        got = target.identify(test_live.Bench.cdp, mind_profile=self.mind_view)
        self.assertTrue(got.mind_view, got)

    def test_real_chromium_on_its_default_profile_is_not_despite_its_arguments(self):
        got = target.identify(self.persons_cdp, mind_profile=self.mind_view)
        self.assertFalse(got.mind_view, got)

    def test_a_mind_is_let_into_mind_views_browser(self):
        self.assertTrue(self.driver(False, test_live.Bench.cdp).tabs()["open"])

    def test_a_mind_is_refused_the_persons_and_nothing_is_done_to_it(self):
        before = self.driver(True, self.persons_cdp).tabs()["tabs"]
        d = self.driver(False, self.persons_cdp)
        for call in (lambda: d.go(test_live.Bench.site + "/pages/other.html"), d.tabs, d.read, d.text):
            with self.assertRaises(drv.PageRefused) as got:
                call()
            self.assertIn("Nothing was done", str(got.exception))
        self.assertEqual(self.driver(True, self.persons_cdp).tabs()["tabs"], before)

    def test_the_person_is_let_into_theirs_and_refused_mind_views(self):
        self.assertTrue(self.driver(True, self.persons_cdp).tabs()["open"])
        with self.assertRaises(drv.PageRefused):
            self.driver(True, test_live.Bench.cdp).read()

    def test_a_process_that_retitles_itself_to_claim_mind_views_profile_is_not_it(self):
        """What Chromium does to its own command line, done on purpose: one space-joined string."""
        port = test_live.free_port()
        code = textwrap.dedent("""
            import ctypes, socket, sys, time
            start, end = (int(x) for x in open("/proc/self/stat").read().rpartition(")")[2].split()[45:47])
            text = (" ".join(["chromium", "--remote-debugging-port=%s", "--user-data-dir=%s"]) % (sys.argv[1], sys.argv[2])).encode()
            room = end - start
            ctypes.memmove(start, text[:room - 1].ljust(room - 1, b" ") + bytes([0]), room)
            s = socket.socket(); s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(1)
            time.sleep(60)
        """)
        child = subprocess.Popen([sys.executable, "-c", code, str(port), self.mind_view] + ["x" * 200])
        self.addCleanup(lambda: (child.kill(), child.wait()))
        got = listening_on(port)
        with open("/proc/%d/cmdline" % child.pid, "rb") as f:
            said = f.read()
        self.assertIn(b"--user-data-dir=" + self.mind_view.encode(), said)
        self.assertNotIn(b"\x00--user-data-dir", said)
        self.assertFalse(target.identify("http://127.0.0.1:%d" % port, mind_profile=self.mind_view).mind_view)
        self.assertEqual(got.pids, (child.pid,))


if __name__ == "__main__":
    unittest.main()
