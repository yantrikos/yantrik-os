#!/usr/bin/env python3
"""What a Mind View page drew before the service held the browser does not stay on screen (#1200).

    python3 tests/browser-core/test_held_pages.py

A real Chromium on a DevTools pipe (pipe_chrome.py), started with a `file:` page that embeds a
protected file, as the browser's UI could open one in the window before the shell's hand-over (an
omnibox entry, a restored session). Before the service holds it the page is drawn unfenced. Then
the service connects, and with NO call after that the page is gone from the browser: closed, never
reloaded behind the fence on a later touch. Skips (exit 0, says so) without a Chromium, as the other
live suites do.
"""

import base64
import os
import shutil
import sys
import tempfile
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [HERE, os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

try:
    from yantrik_browser import cdp, pipes  # noqa: E402
    import pipe_chrome  # noqa: E402
    from raw_cdp import Raw  # noqa: E402
    from test_screenshot import png_pixel  # noqa: E402
    from test_live import chrome  # noqa: E402
    import websocket  # noqa: F401,E402
    SUPPORTED = sys.platform.startswith("linux") and chrome() is not None
except ImportError:
    SUPPORTED = False

RED = (255, 0, 0)


@unittest.skipUnless(SUPPORTED, "needs Linux, google-chrome or chromium, and python3-websocket")
class PagesDrawnBeforeTheHold(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="yb-held-")
        self.addCleanup(shutil.rmtree, self.home, True)
        os.makedirs(os.path.join(self.home, ".ssh"))
        with open(os.path.join(self.home, ".ssh", "key.html"), "w") as f:
            f.write("<body style=\"margin:0;background:#f00\"><p>SECRET-KEY-MATERIAL</p>")
        self.page = os.path.join(self.home, "embed.html")
        with open(self.page, "w") as f:
            f.write("<title>Embedder</title><body style=\"margin:0\"><iframe src=\"file://%s/.ssh/key.html\" "
                    "style=\"position:absolute;border:0;left:0;top:0;width:300px;height:300px\"></iframe>" % self.home)
        self.procs = []
        self.addCleanup(lambda: [(p.kill(), p.wait()) for p in self.procs])

    def browser_with(self, *urls):
        """Chromium on a pipe, started on `urls`, and the service's holder of its stream."""
        log = open(os.path.join(self.home, "chrome-%d.log" % len(self.procs)), "wb")
        self.addCleanup(log.close)
        proc, mine = pipe_chrome.launch(chrome(), ["--headless=new", "--window-size=800,600",
                                                   "--user-data-dir=" + os.path.join(self.home, "profile%d" % len(self.procs)),
                                                   *urls], log)
        self.procs.append(proc)
        self.addCleanup(mine.close)
        holder = pipes.PipeHolder()
        holder.give(mine, proc.pid)
        return proc, holder

    def pages(self, raw):
        return [t for t in raw.call("Target.getTargets").get("targetInfos", []) if t["type"] == "page"]

    def drawn(self, raw):
        """Wait for the embedding page to have loaded, and the screenshot of it: the premise, that
        the protected file really is on screen before the hold."""
        end = time.monotonic() + 30
        while time.monotonic() < end:
            found = [t for t in self.pages(raw) if t["title"] == "Embedder"]
            if found:
                break
            time.sleep(0.2)
        else:
            self.fail("the embedding page never loaded")
        session = raw.call("Target.attachToTarget", {"targetId": found[0]["targetId"], "flatten": True})["sessionId"]
        time.sleep(0.5)
        png = base64.b64decode(raw.call("Page.captureScreenshot", {}, session)["data"])
        shot = os.path.join(self.home, "before.png")
        with open(shot, "wb") as f:
            f.write(png)
        self.assertEqual(png_pixel(shot, 150, 150), RED, "the page drew the protected file before any hold")
        raw.call("Target.detachFromTarget", {"sessionId": session})

    def hold(self, proc, holder):
        """The service takes the browser, and nothing else is asked of it."""
        browser = cdp.Browser(opener=holder.open, held_at_start=True)
        self.addCleanup(browser.close)
        browser.connect()
        return browser

    def targets_after(self, browser):
        return [t for t in browser.call("Target.getTargets").get("targetInfos", []) if t["type"] == "page"]

    def test_a_page_that_drew_a_protected_file_is_closed_when_the_service_holds_the_browser(self):
        proc, holder = self.browser_with("file://" + self.page)
        raw = Raw(holder)
        raw.call("Target.createTarget", {"url": "about:blank"})      # headless takes one url: the second tab is made here
        self.drawn(raw)
        blank_before = [t["targetId"] for t in self.pages(raw) if t["url"] == "about:blank"]
        raw.close()
        browser = self.hold(proc, holder)
        after = self.targets_after(browser)       # the first thing asked of the browser since the hold
        self.assertEqual([t["url"] for t in after], ["about:blank"], after)
        self.assertEqual([t["targetId"] for t in after], blank_before, "the untouched blank page is the same one")
        self.assertEqual({t.url for t in browser.pages()}, {"about:blank"})
        self.assertIsNone(proc.poll(), "the browser itself is still running")

    def test_when_the_only_page_is_one_that_drew_it_a_blank_page_keeps_the_browser_open(self):
        proc, holder = self.browser_with("file://" + self.page)
        raw = Raw(holder)
        self.drawn(raw)
        raw.close()
        browser = self.hold(proc, holder)
        time.sleep(1.0)                           # a browser with no window left would exit by now
        after = self.targets_after(browser)
        self.assertEqual([t["url"] for t in after], ["about:blank"], after)
        self.assertIsNone(proc.poll(), "closing the last page would have closed the browser")

    def test_a_page_that_cannot_be_closed_fails_the_connection_and_is_not_trusted(self):
        proc, holder = self.browser_with("file://" + self.page)
        raw = Raw(holder)
        raw.call("Target.createTarget", {"url": "about:blank"})
        self.drawn(raw)
        raw.close()
        browser = cdp.Browser(opener=holder.open, held_at_start=True)
        self.addCleanup(browser.close)
        real = browser.call

        def failing(method, *a, **k):
            if method == "Target.closeTarget":
                raise cdp.CdpError("no")
            return real(method, *a, **k)
        with mock.patch.object(browser, "call", failing):
            with self.assertRaises(cdp.CdpError):
                browser.connect()
        self.assertFalse(browser.alive, "a connection that could not clean the window is not used")


if __name__ == "__main__":
    unittest.main()
