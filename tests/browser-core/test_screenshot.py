#!/usr/bin/env python3
"""A picture of the page (#1152), against the live bench of test_live.py: a PNG path, of the
caller's own tab only, decoded here pixel by pixel.

    python3 tests/browser-core/test_screenshot.py

One browser and one site for the run, from test_live; skips without a browser, as it does.
"""

import os
import shutil
import struct
import tempfile
import time
import unittest
import zlib

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)


def png_pixel(path, x, y):
    """The (r, g, b) of one pixel of a PNG, decoded here (no image library on the test host):
    8-bit RGB or RGBA, no interlace, which is what the browser writes."""
    with open(path, "rb") as f:
        data = f.read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos, idat, width, height, channels = 8, b"", 0, 0, 0
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, color = struct.unpack(">IIBB", body[:10])
            assert depth == 8 and color in (2, 6), (depth, color)
            channels = 3 if color == 2 else 4
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = width * channels
    prev = bytearray(stride)
    for row in range(height):
        at = row * (stride + 1)
        filt, line = raw[at], bytearray(raw[at + 1:at + 1 + stride])
        for i in range(stride):
            a = line[i - channels] if i >= channels else 0
            b = prev[i]
            c = prev[i - channels] if i >= channels else 0
            if filt == 1:
                line[i] = (line[i] + a) & 255
            elif filt == 2:
                line[i] = (line[i] + b) & 255
            elif filt == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif filt == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        if row == y:
            return tuple(line[x * channels:x * channels + 3])
        prev = line
    raise AssertionError("pixel out of range")


class Screenshot(unittest.TestCase):
    """A picture of the page (FABLE.md 9 Nov, "got in the way" 2): a PNG path, of the caller's own
    tab only. A temp directory stands in for the home, so nothing lands in a real Pictures folder."""

    @classmethod
    def setUpClass(cls):
        cls.home = tempfile.mkdtemp(prefix="yb-shots-")
        cls.d = drv.Driver(Bench.d.sides.fixed, judge=False, file_rule=False, home=lambda: cls.home)
        cls.site = Bench.site

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.home, ignore_errors=True)

    def test_a_screenshot_is_a_png_of_the_viewport_saved_under_the_home(self):
        self.d.go(self.site + "/pages/red.html")
        out = self.d.screenshot()
        self.assertTrue(out["path"].startswith(os.path.join(self.home, "Pictures", "Screenshots", "Browser")), out)
        self.assertTrue(os.path.basename(out["path"]).startswith("Page "), out["path"])
        self.assertEqual(os.stat(out["path"]).st_mode & 0o777, 0o600)
        self.assertEqual(out["size"][0], self.d.read()["viewport"][0])
        self.assertEqual(png_pixel(out["path"], out["size"][0] // 2, out["size"][1] // 2), (255, 0, 0))
        self.assertIn("not the screen", out["of"])

    def test_each_tab_gets_its_own_picture_and_never_anothers(self):
        self.d.go(self.site + "/pages/red.html")
        red = self.d.tabs()["tabs"][-1]["tab"]
        self.d.go(self.site + "/pages/blue.html", new_tab=True)
        blue = self.d.tabs()["tabs"][-1]["tab"]
        self.assertNotEqual(red, blue)
        shot_red = self.d.screenshot(tab=red)
        shot_blue = self.d.screenshot(tab=blue)
        self.assertEqual(png_pixel(shot_red["path"], 10, 10), (255, 0, 0), "the red tab, though blue is in front")
        self.assertEqual(png_pixel(shot_blue["path"], 10, 10), (0, 0, 255))
        self.assertEqual(shot_red["tab"], red)
        self.assertNotEqual(shot_red["path"], shot_blue["path"], "two in one second get two names")
        self.d.close_tab(blue)

    def test_full_captures_the_whole_page(self):
        self.d.go(self.site + "/pages/blue.html")
        out = self.d.screenshot(full=True)
        self.assertTrue(out["full_page"])
        self.assertGreaterEqual(out["size"][1], 3000, out)

    def test_a_planted_link_at_the_next_name_is_not_followed(self):
        folder = os.path.join(self.home, "Pictures", "Screenshots", "Browser")
        os.makedirs(folder, exist_ok=True)
        victim = os.path.join(self.home, "victim.txt")
        with open(victim, "w") as f:
            f.write("keep me")
        stamp = time.strftime("%Y-%m-%d at %H.%M.%S")
        os.symlink(victim, os.path.join(folder, "Page %s.png" % stamp))
        self.d.go(self.site + "/pages/red.html")
        out = self.d.screenshot()
        with open(victim) as f:
            self.assertEqual(f.read(), "keep me")
        self.assertFalse(os.path.islink(out["path"]))


if __name__ == "__main__":
    unittest.main()
