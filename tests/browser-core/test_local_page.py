#!/usr/bin/env python3
"""`go` and file: addresses (#1068): a Mind may open a page of its own from the person's home,
and nothing else on disk. No browser is needed: the shell's `files_stat` is stood in for, and the
browser is a stub that records where it was sent.

    python3 tests/browser-core/test_local_page.py
"""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import driver as drv, local_page  # noqa: E402
from yantrik_surface import files  # noqa: E402

drv.hands.settle = lambda *a, **k: (0.0, "")   # no page to wait for

HOME = "/home/p"
ANSWERS = {
    "/home/p/Sites/index.html": {"exists": True, "kind": "file"},
    "/home/p/Sites": {"exists": True, "kind": "directory"},
    "/home/p/.ssh/id_ed25519": {"exists": "unknown", "reason": "protected"},
    "/home/p/Sites/link.html": {"exists": "unknown", "reason": "outside"},  # a link out of the home
    "/etc/passwd": {"exists": "unknown", "reason": "outside"},
}


def stat(asked):
    return ANSWERS.get(asked, {"exists": False, "reason": "not_found"})


class FakeTab:
    def __init__(self, url="about:blank"):
        self.url, self.id, self.session = url, "t1", "s1"


class FakeBrowser:
    def __init__(self):
        self.tab_now = FakeTab()
        self.sent = []

    def tab(self, _tab=None):
        return self.tab_now

    def call(self, method, params=None, session=None):
        self.sent.append((method, params))
        if method == "Page.navigate":
            self.tab_now.url = params["url"]
        return {}

    def pages(self):
        return [self.tab_now]

    def new_tab(self, url):
        self.sent.append(("new_tab", url))
        return self.tab_now


class Driver(drv.Driver):
    """The real `go` and `_tab`, with the shell's answer stood in and no page to wait for."""

    def __init__(self):
        super().__init__(FakeBrowser(), judge=False)

    def _allow_file(self, url):
        try:
            return local_page.check(url, stat)
        except local_page.LocalRefused as e:
            raise drv.Refused(str(e)) from None

    def _act(self, tab, doing, fn, **_):
        fn()
        return {"did": doing}

    def _run(self, *a, **k):
        return {"text": ""}

    def _consequence(self, *a, **k):
        return {"did": "new tab"}


class TestLocalPage(unittest.TestCase):
    def test_a_file_in_the_home_opens(self):
        d = Driver()
        self.assertEqual(d.go("file:///home/p/Sites/index.html"), {"did": "went to file:///home/p/Sites/index.html"})
        self.assertEqual(d.browser.sent, [("Page.navigate", {"url": "file:///home/p/Sites/index.html"})])
        d = Driver()
        d.go("file:///home/p/Sites/index.html?x=1#top", new_tab=True)
        self.assertEqual(d.browser.sent[0][0], "new_tab")

    def test_outside_the_roots_is_refused_and_nothing_is_sent(self):
        for url, why in [
            ("file:///etc/passwd", "is outside"),
            ("file:///home/p/.ssh/id_ed25519", "is protected"),
            ("file:///home/p/Sites/link.html", "is outside"),          # a symlink escape
            ("file:///home/p/Sites", "is not a file"),                  # no folder listings
            ("file:///home/p/gone.html", "there is no file"),
            ("file:///home/p/Sites/../.ssh/id_ed25519", "`..`"),        # traversal
            ("file://other-host/home/p/Sites/index.html", "another host"),
            ("file:home/p/Sites/index.html", "full path"),
        ]:
            d = Driver()
            with self.assertRaises(drv.Refused) as caught:
                d.go(url)
            self.assertIn(why, str(caught.exception), url)
            self.assertEqual(d.browser.sent, [], url)

    def test_other_schemes_are_still_not_pages(self):
        with self.assertRaises(drv.Refused):
            Driver().go("ftp://example.com/x")

    def test_a_link_from_the_page_to_a_file_outside_the_roots_is_left(self):
        d = Driver()
        d.browser.tab_now.url = "file:///etc/passwd"
        with self.assertRaises(drv.Refused) as caught:
            d.text()
        self.assertIn("taken off it", str(caught.exception))
        self.assertEqual(d.browser.sent[-1], ("Page.navigate", {"url": "about:blank"}))

    def test_the_check_is_the_one_every_agents_file_meets(self):
        # No second rule: the verdict is `files.may_read_file`'s, asked for any caller.
        src = open(os.path.join(ROOT, "apps", "browser", "yantrik_browser", "local_page.py"), encoding="utf-8").read()
        self.assertIn("files.may_read_file(path, stat, anyone=True)", src)
        with self.assertRaises(files.Refusal):
            files.may_read_file("/etc/passwd", stat, anyone=True)


if __name__ == "__main__":
    unittest.main()
