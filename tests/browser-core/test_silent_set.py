#!/usr/bin/env python3
"""Every browser action graded `safe` is named in the silent set, so a new one cannot land
unnamed and run unasked while the gate's fail-closed rule exempts `safe` (#164, L1).

    python3 tests/browser-core/test_silent_set.py

Reads the service's source; needs no browser.
"""

import os
import re
import sys
import unittest

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "sdk", "python"))
from yantrik_surface import always_asked  # noqa: E402


class SilentSet(unittest.TestCase):
    def test_every_safe_action_is_in_browser_silent(self):
        src = open(os.path.join(ROOT, "apps", "browser", "yantrik_browser", "service.py"), encoding="utf-8").read()
        safe = re.findall(r'@s\.action\(grade="safe"[^\n]*\)\s*\n\s*def (\w+)', src)
        self.assertGreaterEqual(len(safe), 5, "the pattern found the safe actions: %s" % safe)
        self.assertEqual([n for n in safe if n not in always_asked.BROWSER_SILENT], [], "safe but not named silent")


if __name__ == "__main__":
    unittest.main()
