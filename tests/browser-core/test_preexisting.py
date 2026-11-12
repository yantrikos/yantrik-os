#!/usr/bin/env python3
"""What was open before the surface connected, and the person's own browser (#1170 re-reviews: C, H; #1200).

    python3 tests/browser-core/test_preexisting.py

Its own browser and profile, over its own pipe: the shared bench's connection fences every window from
the moment it connects, so these are made while nothing is connected, as a restart of the surface
or a lost connection leaves them. Skips without a browser, as test_live does.
"""

import os
import shutil
import struct
import tempfile
import time
import unittest
import zlib

from test_live import Bench, chrome
from raw_cdp import Raw
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)


class Second(Bench):
    """A browser of its own, which the service has not yet connected to."""
    service_first = False


def red_png(path, size=400):
    """A solid red PNG, written here: a picture a leak would show as red."""
    row = b"\x00" + b"\xff\x00\x00" * size

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(row * size)) + chunk(b"IEND", b""))


@unittest.skipUnless(chrome(), "no browser on this machine")
class Preexisting(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="yb-home-")
        self.outside = tempfile.mkdtemp(prefix="yb-outside-")
        os.makedirs(os.path.join(self.home, ".ssh"))
        os.makedirs(os.path.join(self.home, "site"))
        with open(os.path.join(self.home, ".ssh", "key.html"), "w") as f:
            f.write("<body style=\"margin:0;background:#f00\"><p>SECRET-KEY-MATERIAL-1234</p>")
        red_png(os.path.join(self.home, ".ssh", "key.png"))
        with open(os.path.join(self.home, "site", "opener.html"), "w") as f:
            f.write("<p>opener</p>")
        Second.start()
        self.addCleanup(self.stop)
        self.browser = Second.d.sides.fixed
        self.browser.close()               # nothing is connected: nothing fences what follows
        self.raw = Raw(Second.holder)
        self.addCleanup(self.raw.close)

    def stop(self):
        Second.stop()
        shutil.rmtree(self.home, ignore_errors=True)
        shutil.rmtree(self.outside, ignore_errors=True)

    def opener(self):
        """A local page, open in a bare session, from which a script can open windows."""
        _, session = self.raw.open("file://%s/site/opener.html" % self.home)
        time.sleep(1.0)
        return session

    def after_the_connection(self):
        """What the surface finds when it connects: every page target that existed before it is
        closed (adopt.py, #1200), the opener and any window it opened, so nothing drawn unfenced
        stays on screen. Returns the driver and the pages left."""
        driver = drv.Driver(self.browser, judge=False, file_rule=False, home=lambda: self.home)
        self.browser.connect()
        return driver, [t for t in self.browser.pages()]

    def assertNothingOpenedBeforeSurvives(self, pages):
        self.assertEqual([t for t in pages if t.opened_by], [], "a window opened before the connection stayed")
        self.assertFalse([t for t in pages if "opener.html" in str(t.url)], "the opener stayed")

    def test_a_window_open_before_the_connection_is_closed_when_it_connects(self):
        self.raw.evaluate(self.opener(), (
            "var x = window.open(''); x.document.write('<iframe src=\"file://%s/.ssh/key.html\" "
            "style=\"position:absolute;border:0;left:0;top:0;width:300px;height:300px\"></iframe>'); "
            "x.document.close()") % self.home, gesture=True)
        time.sleep(1.0)
        _, pages = self.after_the_connection()
        self.assertNothingOpenedBeforeSurvives(pages)

    def test_an_image_a_popup_loaded_while_the_connection_was_down_is_closed_when_it_connects(self):
        """H: an `about:blank` window, an `<img>` its opener put in it during the gap: no frame holds it."""
        session = self.opener()
        self.raw.evaluate(session, (
            "window.x = window.open(''); window.t = setInterval(() => { var i = x.document.createElement('img'); "
            "i.src = 'file://%s/.ssh/key.png?' + Date.now(); i.onload = () => clearInterval(t); "
            "x.document.body.appendChild(i); }, 300); 1") % self.home, gesture=True)
        for _ in range(30):
            if self.raw.evaluate(session, "x.document.images.length && x.document.images[0].naturalWidth") == 400:
                break
            time.sleep(0.2)
        else:
            self.fail("the picture did not load in the window: the probe is broken")
        _, pages = self.after_the_connection()
        self.assertNothingOpenedBeforeSurvives(pages)


if __name__ == "__main__":
    unittest.main()
