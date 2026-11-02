#!/usr/bin/env python3
"""The guard (#1085) against a real browser and the real /proc.

    python3 tests/browser-core/test_target_live.py

The browser and the site are test_live's. Skips (exit 0) without google-chrome or chromium.
"""

import os
import sys
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


@unittest.skipUnless(test_live.chrome(), "needs google-chrome or chromium")
class WhichBrowser(unittest.TestCase):
    """The guard (#1085) against the real browser and the real /proc: a Mind is let into the
    browser whose profile is Mind View's and into no other, and a refused go leaves the page alone."""

    def driver(self, person, mind_view):
        guard = target.Guard(is_person=lambda: person, profile=lambda: mind_view)
        d = drv.Driver(cdp.Browser(test_live.Bench.cdp), judge=False, guard=guard)
        self.addCleanup(d.browser.close)
        return d

    def test_a_mind_is_let_into_the_browser_that_has_mind_views_profile(self):
        self.assertTrue(self.driver(False, test_live.Bench.tmp).tabs()["open"])

    def test_a_mind_is_refused_any_other_and_the_page_is_left_alone(self):
        d = self.driver(False, os.path.join(test_live.Bench.tmp, "not-this-one"))
        before = test_live.Bench.d.tabs()["tabs"]
        with self.assertRaises(drv.PageRefused):
            d.go(test_live.Bench.site + "/pages/other.html")
        with self.assertRaises(drv.PageRefused):
            d.tabs()
        self.assertEqual(test_live.Bench.d.tabs()["tabs"], before)

    def test_the_person_is_refused_mind_views_browser(self):
        with self.assertRaises(drv.PageRefused):
            self.driver(True, test_live.Bench.tmp).read()


if __name__ == "__main__":
    unittest.main()
