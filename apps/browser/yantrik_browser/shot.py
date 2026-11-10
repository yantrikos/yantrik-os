"""`screenshot`: a PNG of this tab's page, saved under the home, answered with its path (#1152; a
part of driver.py, moved out of it).

`Page.captureScreenshot` runs on the tab's own DevTools session and is drawn by the renderer from
the page alone: it cannot hold the screen, the browser's chrome, another tab or another window —
and for a mind the session is in Mind View's browser, so the person's is out of reach by
construction. The file goes beside the shell's own captures, named as they are, and is made new
and never through a link.
"""

import base64
import errno
import os
import struct
import time

from .cdp import short
from .refusals import Refused

SHOTS_DIR = ("Pictures", "Screenshots", "Browser")   # under the home, beside the shell's own captures
# What an answer carries of the picture itself, when asked (`inline`): the MCP bridge runs as the
# Mind and cannot read the file, so the bytes travel in the answer — bounded, since an answer is
# one line on a socket, and a vision model takes a few MB and 8000 pixels a side at most.
INLINE_LIMIT = 2 * 1024 * 1024
INLINE_SIDE = 8000


class ShotMixin:
    """The screenshot methods of `Driver`. They rely on the driver's `home`, `held`, `_tab` and `_call`."""

    def _save_png(self, data):
        """Write a capture under ~/Pictures/Screenshots/Browser as `Page <date> at <time>.png`,
        the shell's own naming (capture/names.rs), and like it never through a link: the file is
        made new (O_EXCL | O_NOFOLLOW, 0600), and a name already taken gets a (2), (3)…"""
        folder = os.path.join(self.home(), *SHOTS_DIR)
        os.makedirs(folder, mode=0o700, exist_ok=True)
        stamp = time.strftime("%Y-%m-%d at %H.%M.%S")
        for n in range(1, 100):
            name = "Page %s%s.png" % (stamp, "" if n == 1 else " (%d)" % n)
            path = os.path.join(folder, name)
            try:
                fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            except OSError as e:
                if e.errno in (errno.EEXIST, errno.ELOOP):
                    continue
                raise Refused("the picture could not be saved in %s: %s" % (folder, e.strerror)) from None
            with os.fdopen(fd, "wb") as f:
                f.write(data)
            return path
        raise Refused("the picture could not be saved: too many taken this second")

    @staticmethod
    def _inline(out, data, width, height):
        """The picture itself on the answer, base64 PNG, for a caller that can look at it — within
        the caps, or `png_withheld` says why not (the file is there either way)."""
        if len(data) > INLINE_LIMIT:
            out["png_withheld"] = ("the picture is %d bytes, more than the %d an answer carries; it is in "
                                   "the file" % (len(data), INLINE_LIMIT))
        elif max(width, height) > INLINE_SIDE:
            out["png_withheld"] = ("the picture is %dx%d, more than %d pixels a side; it is in the file "
                                   "(without full, the viewport alone fits)" % (width, height, INLINE_SIDE))
        else:
            out["png"] = base64.b64encode(data).decode("ascii")

    def screenshot(self, tab=None, full=False, inline=False):
        """A PNG of this tab's page — what is in its viewport, or with `full` the whole page — saved
        under the home; answers with the path, and with `inline` the picture itself too. It is a
        picture of the page in the caller's own browser's tab, drawn by the browser from the page
        alone: never the screen, never another tab, never the person's browser for a mind."""
        with self.held():
            t = self._tab(tab)
            params = {"format": "png"}
            if full:
                metrics = self._call("Page.getLayoutMetrics", tab=t)
                size = metrics.get("cssContentSize") or metrics.get("contentSize") or {}
                w, h = int(size.get("width") or 0), int(size.get("height") or 0)
                if w and h:
                    params["captureBeyondViewport"] = True
                    params["clip"] = {"x": 0, "y": 0, "width": w, "height": min(h, 16384), "scale": 1}
            try:
                got = self._call("Page.captureScreenshot", params, t)
            except Refused as e:
                raise Refused("the browser could not draw this tab (%s). A tab in the background is not "
                              "drawn: switch_tab brings it forward, then screenshot." % e) from None
            data = base64.b64decode(got.get("data") or "")
            if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
                raise Refused("the browser answered with no picture")
            width, height = struct.unpack(">II", data[16:24])
            path = self._save_png(data)
            out = {"path": path, "bytes": len(data), "size": [width, height], "full_page": bool(full),
                   "url": t.url, "title": t.title, "tab": short(t.id),
                   "of": "this tab's page only, drawn by the browser: not the screen, not another tab"}
            if inline:
                self._inline(out, data, width, height)
            return out
