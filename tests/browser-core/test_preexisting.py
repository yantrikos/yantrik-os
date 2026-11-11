#!/usr/bin/env python3
"""A window that was open before the surface connected (#1170 re-review, C): it loaded with no
fence on it, and a file it drew then must not be in a screenshot taken through the surface.

    python3 tests/browser-core/test_preexisting.py

Its own browser, own profile and own port: the shared bench's connection fences every window from
the moment it connects, so this one is made while nothing is connected, as a restart of the surface
or a lost connection leaves it. Skips without a browser, as test_live does.
"""

import json
import os
import shutil
import tempfile
import time
import unittest
import urllib.request

from test_live import Bench, chrome
from test_screenshot import png_pixel
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)

RED = (255, 0, 0)


class Second(Bench):
    """A browser of its own."""


def raw_session(address, script, home):
    """A bare DevTools client, with no fence: opens a local page and runs `script` in it as a click."""
    from websocket import create_connection
    with urllib.request.urlopen(address + "/json/version", timeout=3) as r:
        ws = create_connection(json.load(r)["webSocketDebuggerUrl"], origin=address, suppress_origin=False)
    ids = iter(range(1, 100))

    def call(method, params=None, session=None):
        mid = next(ids)
        ws.send(json.dumps({"id": mid, "method": method, "params": params or {},
                            **({"sessionId": session} if session else {})}))
        while True:
            msg = json.loads(ws.recv())
            if msg.get("id") == mid:
                return msg.get("result", {})
    try:
        target = call("Target.createTarget", {"url": "file://%s/site/opener.html" % home})["targetId"]
        session = call("Target.attachToTarget", {"targetId": target, "flatten": True})["sessionId"]
        time.sleep(1.0)
        call("Runtime.evaluate", {"expression": script, "userGesture": True}, session)
        time.sleep(1.0)
    finally:
        ws.close()


@unittest.skipUnless(chrome(), "no browser on this machine")
class PreexistingWindow(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="yb-home-")
        os.makedirs(os.path.join(self.home, ".ssh"))
        os.makedirs(os.path.join(self.home, "site"))
        with open(os.path.join(self.home, ".ssh", "key.html"), "w") as f:
            f.write("<body style=\"margin:0;background:#f00\"><p>SECRET-KEY-MATERIAL-1234</p>")
        with open(os.path.join(self.home, "site", "opener.html"), "w") as f:
            f.write("<p>opener</p>")
        Second.start()
        self.addCleanup(self.stop)

    def stop(self):
        Second.stop()
        shutil.rmtree(self.home, ignore_errors=True)

    def test_a_window_open_before_the_connection_is_loaded_again_behind_the_fence(self):
        Second.d.sides.fixed.close()               # nothing is connected: nothing fences what follows
        script = ("var x = window.open(''); x.document.write('<iframe src=\"file://%s/.ssh/key.html\" "
                  "style=\"position:absolute;border:0;left:0;top:0;width:300px;height:300px\"></iframe>'); "
                  "x.document.close()" % self.home)
        raw_session(Second.cdp, script, self.home)
        d = drv.Driver(Second.d.sides.fixed, judge=False, file_rule=False, home=lambda: self.home)
        browser = Second.d.sides.fixed
        browser.connect()
        popup = next(t for t in browser.pages() if t.opened_by)
        shot = d.screenshot(popup.id)
        self.assertNotEqual(png_pixel(shot["path"], 150, 150), RED, "the window drew a hidden file")
        self.assertTrue(popup.ready and not popup.born)


if __name__ == "__main__":
    unittest.main()
