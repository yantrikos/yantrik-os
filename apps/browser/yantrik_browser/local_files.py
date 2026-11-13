"""`go` opens a local file under the home, and nothing outside it (#1152; a part of driver.py,
moved out of it).

A Mind that has just written a build opens it as `file:///home/you/site/index.html`, a bare path
or `~/path`. What a local file is held to, before anything loads: the real path (every link
followed) is the home or under it, a regular file of a web type (never a hidden place such as
`.ssh` or `.config`, where programs keep what the person must not lose), no host; and for an agent
the OS's own rule for a file an agent names (`yantrik_surface.files.may_read_file`, the shell's
`files_stat`).

The same gate holds everything such a page draws and every time the tab is read or acted on
(#1170). A page's image, frame, object, stylesheet or font is a `file:` request like any other,
and fence.py pauses each one and asks `vet_file`; a protected file or a file outside the home is
never drawn, so no screenshot of the page carries it. The tab's own address is vetted again at
every read, act, reload and history step, never once per address: a link swapped behind a page
opens nothing, and a tab that a page's link or window took out is sent to about:blank. What loads
is the vetted real path, not the path as written. Chromium gives each local file an origin of its
own, so a page's script cannot read other local files, and nothing new listens anywhere.
"""

import os
import urllib.parse

from yantrik_surface import files as home_files
from yantrik_surface.surface import Refusal as SurfaceRefusal

from . import hands
from .refusals import PageRefused, Refused

# What a local page is made of. A local page and what it embeds are web files; a key, a database
# or an unnamed file is not, whatever folder it is in.
WEB_TYPES = {".html", ".htm", ".xhtml", ".svg", ".txt", ".md", ".css", ".js", ".mjs", ".json", ".map",
             ".xml", ".csv", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".bmp", ".ico",
             ".woff", ".woff2", ".ttf", ".otf", ".wasm", ".mp3", ".ogg", ".wav", ".mp4", ".webm"}


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


def vet_file(url, home, rule):
    """The real path of the file a `file:` URL may open or draw, or Refused: a regular file of a
    web type under the home with no other name (a hard link is refused), in no hidden place, reached
    through no link that leaves it — and, for an agent, one the OS's file rule allows (`rule`, asked with the path as written; it walks
    the links itself). The one gate: for `go`, for each act on the tab, and for every request
    the page makes."""
    path = file_path_of(url)
    home = os.path.realpath(home)
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
    if os.stat(real).st_nlink > 1:
        raise PageRefused("%s is a hard link: the same file has another name, perhaps in a hidden or "
                          "protected place, and the Browser cannot tell. Nothing was opened." % path)
    if home_files.where_programs_look(path, home):
        raise PageRefused("%s is in a hidden place (a dotfile or a dot-folder), where programs keep "
                          "their settings and what the person must not lose. Nothing was opened." % path)
    if os.path.splitext(real)[1].lower() not in WEB_TYPES:
        raise PageRefused("%s is not a web file (html, svg, txt, css, js, json, an image, a font…). "
                          "Nothing was opened." % path)
    if rule is not None:
        try:
            rule(path)
        except SurfaceRefusal as e:
            raise PageRefused("%s. Nothing was opened." % str(e).rstrip(".")) from None
    return real


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

    def _agent_calling(self):
        """Whether the call in hand is an agent's (read on the dispatch's own thread)."""
        return home_files.agent_is_calling()

    def _vet_local_file(self, url):
        """The real path of the file a `file:` URL may open, or Refused (vet_file)."""
        return vet_file(url, self.home(), self.file_rule)

    def _asset_gate(self):
        """The gate fence.py asks for each `file:` request the browser pauses: the `file:` URL of
        the vetted real path to load, or None to refuse. It runs on the fence's thread, where no
        dispatch is in hand, so whose rule it is comes with the request: `agent` is the
        requesting tab's (cdp.Tab.agent), never the last caller's."""
        home, rule = self.home(), self.file_rule

        def gate(url, agent):
            asked = rule
            if rule is home_files.may_read_file:
                asked = lambda path: home_files.may_read_file(path, as_agent=agent)  # noqa: E731
            try:
                return "file://" + urllib.parse.quote(vet_file(url, home, asked))
            except Refused:
                return None
        return gate

    def _still_allowed(self, t):
        """A tab that has gone to a local file (a link the page held, a window it opened, a link
        swapped behind it since) is held to the same gate as `go`, every time: outside it, it is
        not read — it is sent to about:blank and the act is refused saying so."""
        url = str(t.url or "")
        if not url.lower().startswith("file:"):
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
