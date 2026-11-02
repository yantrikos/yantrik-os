"""A page of the Mind's own, opened from disk: `file:` URLs, held to the roots a Mind may read.

`go` opens web pages. A Mind that builds one ("make me a site") has to look at it to know it
renders (#1068), so `go` also takes a `file:` URL, with these rules:

  - the file is in the person's home, outside its protected places, links followed: the shell's
    `files_stat` decides, through `yantrik_surface.files` (`home_paths::may_read_file`), the one
    rule every agent's file is held to. Nothing is copied here;
  - it is a file on this machine: a host other than `localhost` is refused;
  - `..` is refused rather than collapsed, so the path asked is the path checked;
  - the check holds for the page *where it is now*, not only where `go` sent it: a page of the
    Mind's own can link to `file:///etc/passwd`, and following the link is a navigation `go` never
    saw. Every action takes its tab through `Driver._tab`, which asks `check_open_page`.

Reading such a page is a private read for the bridge's taint rule (`deploy/yantrik-os/yos-mcp`):
whatever is in the page is on this machine, and nothing about it says it is safe to type on.
"""

import posixpath
import urllib.parse

from yantrik_surface import Refusal, files


class LocalRefused(Exception):
    """A `file:` address declined, with the sentence the caller reads."""


def is_file_url(url):
    return str(url or "").strip().lower().startswith("file:")


def path_of(url):
    """The absolute path a `file:` URL names, or `LocalRefused`."""
    u = urllib.parse.urlsplit(str(url).strip())
    if u.scheme.lower() != "file":
        raise LocalRefused("%s is not a file: address" % url)
    if u.netloc not in ("", "localhost"):
        raise LocalRefused("go opens files on this machine only; `%s` is another host" % u.netloc)
    path = urllib.parse.unquote(u.path)
    if "\0" in path or not path.startswith("/"):
        raise LocalRefused("`%s` does not name a file by its full path" % url)
    if ".." in path.split("/"):
        raise LocalRefused("`%s` has `..` in it; give the path without it" % url)
    return posixpath.normpath(path) if path != "/" else path


def check(url, stat=None):
    """Refuse (`LocalRefused`) a `file:` address that is not a file a Mind may read. Returns the
    path. Asked of the shell for every caller: this browser is a Mind's."""
    path = path_of(url)
    try:
        files.may_read_file(path, stat, anyone=True)
    except Refusal as e:
        raise LocalRefused(str(e)) from None
    return path
