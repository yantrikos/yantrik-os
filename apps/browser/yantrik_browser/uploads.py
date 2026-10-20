"""Which file `upload` may put into a page, and the copy of it the page is given (#874).

An upload is the one browser act that sends a file from this machine to a website, so it is an
exfiltration channel: a mind steered by a hostile page or message could try to put
`~/.ssh/id_ed25519`, the vault or a private document into a form. So:

- The path is asked of the desktop's own rule for what an agent may read in the person's home
  (`files_stat`, `yantrik_ipc_contracts::home_paths`, #619): in the home, outside every protected
  place and key or secrets file, there, a file, with one name, and not reached through a link.
  That is the one list every surface shares; nothing here keeps a second one. A shell that cannot
  be asked refuses the upload.
- Here as well, on the bytes themselves: no link anywhere in the path (above the home too), a
  regular file opened without following a link, the same file that was checked, and at most
  UPLOAD_LIMIT bytes, read no further than that.
- The person's card names the file (absolute path, size, sha256 prefix), the site and the input's
  label, and `upload` is bound to all of them. The page is never given the person's file: it is
  given a private copy of the bytes that were hashed and compared, so a file changed after the
  set, or a link swapped in behind it, changes nothing that leaves.
"""

import hashlib
import os
import shutil
import stat as st
import tempfile
import time
import unicodedata

UPLOAD_LIMIT = 25 * 1024 * 1024     # bytes: the largest file `upload` puts into a page
SHA_SHOWN = 16                      # hex digits of the sha256 a card shows and `upload` binds
STAT_TIMEOUT = 5.0                  # seconds the shell is given to answer files_stat
STAGED_FOR = 6 * 3600               # seconds a staged copy is kept for the page to send
PATH_MOST = 4096                    # characters in a path

# What files_stat's reasons mean, for the person reading the refusal.
REASONS = {
    "protected": "is in a protected place: keys, credentials, the desktop's own configuration and "
                 "memory, and key or secrets files (*.key, *.pem, .env) are out of every agent's "
                 "reach",
    "outside": "is outside the person's home, and an agent uploads only files in the home",
    "not_allowed": "cannot be read",
    "broken_link": "is a link that leads nowhere",
    "hard_link": "has a second name (a hard link), which may be anywhere, a key included",
    "not_a_path": "is not a path: give an absolute path or ~/...",
    "not_found": "is not there",
}


class Refused(Exception):
    """A path `upload` will not send, with the sentence the caller reads."""


class Facts:
    """One file as it was checked: where it is, its bytes, and what the card shows of them."""

    def __init__(self, path, data):
        self.path = path
        self.data = data
        self.size = len(data)
        self.sha256 = hashlib.sha256(data).hexdigest()

    @property
    def shown(self):
        return self.sha256[:SHA_SHOWN]


def ask_shell_stat(path):
    """The desktop's `files_stat` answer for `path`, from the shell itself."""
    from yantrik_surface import gate, wire
    reply = wire.call_once(wire.default_socket_path(gate.SHELL), "app.act",
                           {"action": "files_stat", "args": {"path": path}},
                           timeout=STAT_TIMEOUT, peer_rule=gate.must_be_the_shell)
    result = (reply or {}).get("result") if isinstance(reply, dict) else None
    answer = result.get("result") if isinstance(result, dict) else None
    if not isinstance(answer, dict):
        error = (reply or {}).get("error") if isinstance(reply, dict) else None
        raise ConnectionError((error or {}).get("message", "no answer") if isinstance(error, dict)
                              else "no answer")
    return answer


def _unprintable(text):
    """Whether a path holds a character that would draw wrong on the card: a control, a format
    character (bidi overrides, zero-width), or a line or paragraph separator."""
    return any(unicodedata.category(c) in ("Cc", "Cf", "Zl", "Zp") for c in text)


def expand(asked, home=None):
    """`asked` as an absolute, normal path (`~` is the home), or Refused."""
    if not isinstance(asked, str) or not asked.strip():
        raise Refused("upload needs the path of a file")
    if len(asked) > PATH_MOST or "\0" in asked or _unprintable(asked):
        raise Refused("that path holds characters a card cannot show truthfully (controls, bidi "
                      "or zero-width marks); nothing was uploaded")
    home = home if home is not None else os.environ.get("HOME", "")
    path = asked.strip()
    if path == "~" or path.startswith("~/"):
        path = home + path[1:]
    if not os.path.isabs(path):
        raise Refused("%s is not an absolute path: give it as /home/... or ~/..." % asked)
    if ".." in path.split("/"):
        raise Refused("%s goes up with `..`: give the file's own path" % asked)
    return os.path.normpath(path)


def _no_link_on_the_way(path):
    """Refuse if any part of `path`, from / down, is a link. Returns the lstat of the file."""
    probe = "/"
    for part in [p for p in path.split("/") if p]:
        probe = os.path.join(probe, part)
        try:
            info = os.lstat(probe)
        except FileNotFoundError:
            raise Refused("%s is not there" % path) from None
        except OSError as e:
            raise Refused("%s cannot be read (%s)" % (path, e.strerror or e)) from None
        if st.S_ISLNK(info.st_mode):
            raise Refused("%s goes through a link (%s): an upload names the file itself, never a "
                          "link to one, so the card says what really leaves. Nothing was uploaded."
                          % (path, probe))
    return info


def check(asked, stat=None, home=None, limit=UPLOAD_LIMIT):
    """The file at `asked`, checked and read: a `Facts`, or Refused with the reason."""
    path = expand(asked, home)
    stat = stat or ask_shell_stat
    try:
        answer = stat(path)
    except Exception as e:  # noqa: BLE001 - any failure to ask is a refusal, never a pass
        raise Refused("the desktop could not say whether an agent may read %s (%s), so nothing "
                      "was uploaded" % (path, e)) from None
    if answer.get("exists") is not True:
        reason = answer.get("reason") or "unknown"
        raise Refused("%s %s. Nothing was uploaded." % (path, REASONS.get(reason, "cannot be read (%s)" % reason)))
    if answer.get("kind") != "file":
        raise Refused("%s is a %s, not a file. Nothing was uploaded." % (path, answer.get("kind") or "thing"))
    if answer.get("via_link") is not False or answer.get("real") != path:
        raise Refused("%s is reached through a link (it is really %s): an upload names the file "
                      "itself. Nothing was uploaded." % (path, answer.get("real") or "elsewhere"))

    seen = _no_link_on_the_way(path)
    if not st.S_ISREG(seen.st_mode):
        raise Refused("%s is not a regular file. Nothing was uploaded." % path)
    if seen.st_size > limit:
        raise Refused("%s is %s, over the %s an upload may be (UPLOAD_LIMIT). Nothing was uploaded."
                      % (path, size_words(seen.st_size), size_words(limit)))
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC | getattr(os, "O_NOCTTY", 0))
    except OSError as e:
        raise Refused("%s cannot be opened as your account (%s). Nothing was uploaded."
                      % (path, e.strerror or e)) from None
    try:
        info = os.fstat(fd)
        if not st.S_ISREG(info.st_mode):
            raise Refused("%s is not a regular file. Nothing was uploaded." % path)
        if (info.st_dev, info.st_ino) != (seen.st_dev, seen.st_ino) or info.st_nlink != 1:
            raise Refused("%s changed while it was being checked. Nothing was uploaded." % path)
        chunks, got = [], 0
        while got <= limit:
            chunk = os.read(fd, min(1 << 20, limit + 1 - got))
            if not chunk:
                break
            chunks.append(chunk)
            got += len(chunk)
        if got > limit:
            raise Refused("%s is over the %s an upload may be (UPLOAD_LIMIT). Nothing was uploaded."
                          % (path, size_words(limit)))
    finally:
        os.close(fd)
    return Facts(path, b"".join(chunks))


def size_words(n):
    """A size as a person reads it, with the exact bytes."""
    if n < 1024:
        return "%d bytes" % n
    for unit, scale in (("MiB", 1 << 20), ("KiB", 1 << 10)):
        if n >= scale:
            return "%.1f %s (%d bytes)" % (n / scale, unit, n)
    return "%d bytes" % n


# ── the copy the page is given ──


def _private_dir(path):
    """`path` as a directory only this account can enter, made if it is not there; Refused if it
    is a link, someone else's, or open to others."""
    try:
        os.mkdir(path, 0o700)
    except FileExistsError:
        pass
    except OSError as e:
        raise Refused("the upload's private folder %s could not be made (%s)" % (path, e.strerror or e)) from None
    info = os.lstat(path)
    if (not st.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid()
            or st.S_IMODE(info.st_mode) & 0o077):
        raise Refused("the upload's private folder %s is not this account's alone; nothing was "
                      "uploaded" % path)
    return path


def staging_root(runtime=None):
    runtime = runtime if runtime is not None else os.environ.get("XDG_RUNTIME_DIR", "")
    if not runtime or not os.path.isabs(runtime):
        raise Refused("there is no private runtime folder (XDG_RUNTIME_DIR) to hold the upload's "
                      "copy, so nothing was uploaded")
    return _private_dir(os.path.join(_private_dir(os.path.join(runtime, "yantrik-browser")), "uploads"))


def sweep(root, now=None, keep=STAGED_FOR):
    """Remove copies staged more than `keep` seconds ago."""
    now = time.time() if now is None else now
    for name in os.listdir(root):
        where = os.path.join(root, name)
        try:
            info = os.lstat(where)
        except OSError:
            continue
        if now - info.st_mtime > keep:
            if st.S_ISDIR(info.st_mode):
                shutil.rmtree(where, ignore_errors=True)
            else:
                try:
                    os.unlink(where)
                except OSError:
                    pass


def stage(facts, runtime=None):
    """A private, read-only copy of exactly the bytes `facts` holds, under the file's own name,
    for the page to read. Returns its path."""
    root = staging_root(runtime)
    sweep(root)
    folder = tempfile.mkdtemp(prefix="up-", dir=root)
    name = os.path.basename(facts.path)
    dfd = os.open(folder, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                     0o600, dir_fd=dfd)
        try:
            view = memoryview(facts.data)
            while view:
                view = view[os.write(fd, view):]
            os.fchmod(fd, 0o400)
        finally:
            os.close(fd)
    finally:
        os.close(dfd)
    return os.path.join(folder, name)
