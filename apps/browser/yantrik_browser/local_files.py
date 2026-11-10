"""`go` opens a local file under the home, and nothing outside it (#1152; a part of driver.py,
moved out of it).

A Mind that has just written a build opens it as `file:///home/you/site/index.html`, a bare path
or `~/path`. What a local file is held to, before anything loads: the real path (every link
followed) is the home or under it, a regular file, no host; and for an agent the OS's own rule for
a file an agent names (`yantrik_surface.files.may_read_file`, the shell's `files_stat`). And again
at every read or act: a tab that a page's link or window took to a local file outside the home is
sent to about:blank and nothing of it is read. Chromium gives each local file an origin of its
own, so a page's script cannot read other local files, and nothing new listens anywhere.
"""

import os
import urllib.parse

from yantrik_surface import files as home_files
from yantrik_surface.surface import Refusal as SurfaceRefusal

from . import hands
from .refusals import PageRefused, Refused


def file_path_of(url):
    """The absolute path a `file:` URL names, or a Refused saying why it names none: a host
    (`file://server/share`), a relative path, an empty one. The query and fragment are not part
    of the path (`?v=3` is a cache-buster, and Chromium ignores it for files)."""
    parts = urllib.parse.urlsplit(url)
    if parts.netloc not in ("", "localhost"):
        raise Refused("a local file has no host: `file://%s` is not a file on this machine" % parts.netloc)
    path = urllib.parse.unquote(parts.path or "")
    if not path.startswith("/"):
        raise Refused("a local file is named by its whole path, like file:///home/you/site/index.html")
    return path


def file_rule_of(file_rule):
    """The rule a `Driver` is built with: the OS's own for a file an agent names (in the home,
    outside its protected places, asked of the shell), `False` for none (a test's own home)."""
    return None if file_rule is False else (file_rule or home_files.may_read_file)


class LocalFilesMixin:
    """The local-file methods of `Driver`. They rely on the driver's `home`, `file_rule`, `_call`
    and `browser`."""

    def _local_url(self, url):
        """`~/path` and `/path` as the `file:` URL they name; any other url as it is."""
        if url.startswith("~/"):
            url = os.path.join(self.home(), url[2:])
        if url.startswith("/"):
            # A path is a local file (#1152: a Mind opening the build it just wrote).
            url = "file://" + urllib.parse.quote(url)
        return url

    def _vet_local_file(self, url):
        """The real path of the file a `file:` URL may open, or Refused: a regular file under the
        home, reached through no link that leaves it — and, for an agent, one the OS's file rule
        allows (the shell's `files_stat`: outside the home's protected places). The rule is asked
        with the path as written; it walks the links itself."""
        path = file_path_of(url)
        home = os.path.realpath(self.home())
        real = os.path.realpath(path)
        if not (real == home or real.startswith(home.rstrip(os.sep) + os.sep)):
            linked = "" if real == os.path.normpath(path) else " (it is a link to %s)" % real
            raise Refused("go opens local files only under the home, %s; %s is outside it%s. "
                          "Nothing was opened." % (home, path, linked))
        if not os.path.lexists(real) and not os.path.exists(real):
            raise Refused("there is no file at %s; nothing was opened" % path)
        if os.path.isdir(real):
            raise Refused("%s is a folder; go opens a file — name one inside it (index.html, say)" % path)
        if not os.path.isfile(real):
            raise Refused("%s is not a regular file; nothing was opened" % path)
        if self.file_rule is not None:
            try:
                self.file_rule(path)
            except SurfaceRefusal as e:
                raise PageRefused("%s. Nothing was opened." % str(e).rstrip(".")) from None
        return real

    def _still_allowed(self, t):
        """A tab that has gone to a local file (a link the page held, a window it opened) is held
        to the same rule as `go`: outside the home, it is not read — it is sent to about:blank and
        the act is refused saying so. Checked once per address; `go` passes `leaving` to skip it."""
        url = str(t.url or "")
        if not url.lower().startswith("file:") or getattr(t, "vetted_file", None) == url:
            return
        try:
            self._vet_local_file(url)
        except Refused as e:
            try:
                self._call("Page.navigate", {"url": "about:blank"}, t)
                hands.settle(self.browser, t, 3.0)
            except Refused:
                pass
            raise PageRefused("this tab had gone to %s, which the Browser does not open: %s It was "
                              "taken to about:blank, and nothing of it was read." % (url, str(e))) from None
        t.vetted_file = url
