#!/usr/bin/env python3
"""A reload that bypasses the cache, and find "+" (FABLE.md 9 Nov, "got in the way" 4 and 5), against
the live bench of test_live.py.

    python3 tests/browser-core/test_reload_find.py

One browser and one site for the run, from test_live; skips without a browser, as it does.
"""

import time
import unittest

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)


class ReloadAndFind(unittest.TestCase):
    """FABLE.md 9 Nov, "got in the way" 4 and 5: a reload must bypass the cache (or say it did
    not), and find "+" must find the "+" key."""

    @classmethod
    def setUpClass(cls):
        cls.d, cls.site = Bench.d, Bench.site

    def count(self):
        text = self.d.text()["text"]
        self.assertIn("count ", text)
        return int(text.split("count ")[1].split()[0])

    def test_reload_fetches_a_cached_script_again_and_says_so(self):
        self.d.go(self.site + "/pages/cached.html")
        first = self.count()
        # A plain reload (Chromium's own, without ignoreCache) takes the hour-cached script from
        # the cache: the count does not move. That is the bug of 9 Nov.
        t = self.d._tab()
        before = t.cache_hits
        self.d._call("Page.reload", {}, t)
        time.sleep(1.0)
        self.assertEqual(self.count(), first, "a plain reload served the cached script")
        self.assertGreater(t.cache_hits, before, "the cached response was seen as one")
        out = self.d.reload()
        self.assertEqual(self.count(), first + 1, "reload fetched the script from the server again")
        self.assertTrue(out["cache_bypassed"], out)
        self.assertEqual(out["from_cache"], 0)
        self.assertIn("bypassing the cache", out["did"])

    def test_find_a_single_punctuation_mark_finds_the_key_first(self):
        self.d.go(self.site + "/pages/plus.html")
        got = self.d.find("+")
        self.assertTrue(got["found"], got)
        self.assertEqual(got["found"][0]["name"], "+")
        self.assertEqual(got["found"][0]["role"], "button")
        names = [e["name"] for e in got["found"]]
        self.assertIn("+/-", names, "a name that holds the mark is still found, after the exact one")
        self.assertEqual(self.d.find("=")["found"][0]["name"], "=")

    def test_find_puts_the_exact_name_before_longer_ones(self):
        self.d.go(self.site + "/pages/plus.html")
        names = [e["name"] for e in self.d.find("Save")["found"]]
        self.assertEqual(names[:2], ["Save", "Save as draft"], names)
        self.assertEqual(len(names), 3)


if __name__ == "__main__":
    unittest.main()
