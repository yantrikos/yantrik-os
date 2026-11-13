"""`screenshot`: a PNG of this tab's page, saved under the home, answered with its path (#1152; a
part of driver.py, moved out of it).

`Page.captureScreenshot` runs on the tab's own DevTools session and is drawn by the renderer from
the page alone: it cannot hold the screen, the browser's chrome or another window — and for a
mind the session is in Mind View's browser, so the person's is out of reach by construction. (A
tab of Mind View's browser is reachable by any mind that names it: they share that browser.) A local page draws only what the fence (fence.py) let load.

The file is a mind's picture of a page, which may hold what the page showed the person (#1170): it
goes in Yantrik's own protected state, never among the person's desktop captures in
`~/Pictures/Screenshots`, which it would be mistaken for. The folder is checked, every time, to
be a real one of ours (no link anywhere on its way); the file is made new, never through a link,
and only so many are kept, so a loop of screenshots cannot fill the disk. A full-page picture is
bounded in both sides and in pixels before the browser is asked to draw it.
"""

import base64
import errno
import os
import stat
import struct
import time

from .cdp import short
from .refusals import Refused

SHOTS_DIR = (".local", "share", "yantrik", "browser-shots")   # under the home: Yantrik's protected state
KEEP_FILES = 40                      # pictures kept; the oldest go as new ones come
KEEP_BYTES = 256 * 1024 * 1024       # and no more than this many bytes of them
FULL_WIDTH = 8000                    # the widest a full-page picture is drawn
FULL_HEIGHT = 16384                  # the tallest
FULL_PIXELS = 40_000_000             # and the most pixels: a bitmap of this is ~160 MB
# What an answer carries of the picture itself, when asked (`inline`): the MCP bridge runs as the
# Mind and cannot read the file, so the bytes travel in the answer — bounded, since an answer is
# one line on a socket, and a vision model takes a few MB and 8000 pixels a side at most.
INLINE_LIMIT = 2 * 1024 * 1024
INLINE_SIDE = 8000


class ShotMixin:
    """The screenshot methods of `Driver`. They rely on the driver's `home`, `held`, `_tab` and `_call`."""

    def _shots_folder(self):
        """Our private folder for pictures, made if it is not there, or Refused when anything on
        its way is a link, or it is not ours alone: a picture is never written where a link leads."""
        home = os.path.realpath(self.home())
        folder = os.path.join(self.home(), *SHOTS_DIR)
        os.makedirs(folder, mode=0o700, exist_ok=True)
        if os.path.realpath(folder) != os.path.join(home, *SHOTS_DIR):
            raise Refused("the folder for pictures has a link on its way; no picture was saved")
        st = os.lstat(folder)
        if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.getuid() or st.st_mode & 0o077:
            raise Refused("the folder for pictures is not private to the person; no picture was saved")
        return folder

    @staticmethod
    def _prune(folder, newest):
        """Keep the newest KEEP_FILES pictures within KEEP_BYTES; the oldest are removed, never
        `newest`, the one just made."""
        entries = []
        for n in os.listdir(folder):
            if n.startswith("Page ") and n.endswith(".png"):
                st = os.lstat(os.path.join(folder, n))
                entries.append((st.st_mtime_ns, len(n), n, st.st_size))
        entries.sort()
        total = sum(e[3] for e in entries)
        while len(entries) > 1 and (len(entries) > KEEP_FILES or total > KEEP_BYTES):
            _, _, name, size = entries.pop(0)
            if name == newest:
                continue
            os.unlink(os.path.join(folder, name))
            total -= size

    def _save_png(self, data):
        """Write a capture as `Page <date> at <time>.png`, the shell's own naming (capture/names.rs),
        and never through a link: the file is made new (O_EXCL | O_NOFOLLOW, 0600), a name already
        taken gets a (2), (3)…, and the oldest are removed past the keep."""
        folder = self._shots_folder()
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
            self._prune(folder, name)
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
        alone: never the screen, never the person's browser for a mind. The tab is any tab of the
        caller's browser: every mind shares Mind View's."""
        with self.held():
            t = self._tab(tab)
            params = {"format": "png"}
            if full:
                metrics = self._call("Page.getLayoutMetrics", tab=t)
                size = metrics.get("cssContentSize") or metrics.get("contentSize") or {}
                w, h = int(size.get("width") or 0), int(size.get("height") or 0)
                if w and h:
                    w = min(w, FULL_WIDTH)
                    params["captureBeyondViewport"] = True
                    params["clip"] = {"x": 0, "y": 0, "width": w,
                                      "height": min(h, FULL_HEIGHT, FULL_PIXELS // w), "scale": 1}
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
                   "of": "one tab's page, drawn by the browser: not the screen, not the person's browser"}
            if inline:
                self._inline(out, data, width, height)
            return out
