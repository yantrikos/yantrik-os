"""The person's files into an agent's sandbox and back: a copy in, and only checked bytes out
(#953).

The sandbox shows none of the person's home (sandbox.py), so an agent's LibreOffice never opens a
person's file where it lies. The adapter, outside, copies the file the rule allowed into the
sandbox's work folder, and LibreOffice works on that copy. Nothing it does reaches the person's
folders except through here, after the scan and the write rule have had their say:

  * `take` copies a file in. It opens the file it was told (links resolved, the last one not
    followed), refuses anything but a regular file with one name (a hard link could be a
    protected file under another name), and checks that what it opened is still at the path the
    rule judged.
  * `put_back` replaces the person's file with the saved copy, all at once: written beside it
    under a temporary name in the same folder (opened by handle, so a folder swapped for a link
    meanwhile is caught) and renamed over it. It refuses when the file changed on disk since it
    was copied in, so an agent's save never destroys what someone else wrote there.
  * `create` writes a new file and never replaces one (`save_as`, `export_pdf`).

What goes back out is bytes, never a path in the work folder: the sandbox can change anything
there, so workdir.py reads it, through handles, before any of it reaches this file (review H1).

A file is a copy, not the person's document: the person's unsaved changes in their own LibreOffice
are never in it, and `workspace.py` refuses a file the person has open.
"""

import errno
import hashlib
import os
import secrets
import stat

from .basics import OfficeError

# The largest file copied in for an agent.
MAX_BYTES = 200 * 1024 * 1024

CHANGED = ("%s changed on disk since the agent opened it, so saving over it would destroy that "
           "change; nothing was written. `close` the document, `open` the file again, and redo "
           "the edit")


class Copy:
    """One of the person's files, copied into the sandbox: `given` is the path the agent named,
    `real` where it resolved, `work` the copy, `digest` the sha256 of the bytes `real` held."""

    def __init__(self, given, real, work, digest, mode):
        self.given, self.real, self.work, self.digest, self.mode = given, real, work, digest, mode


def _where(fd):
    """The path the kernel says an open file is at."""
    return os.readlink("/proc/self/fd/%d" % fd)


def _read(path):
    """`(bytes, mode)` of the regular file at `path` (its last part not followed)."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    except OSError as e:
        if e.errno == errno.ELOOP:
            raise OfficeError("%s is a link; nothing was copied" % path) from None
        raise OfficeError("%s could not be read (%s)" % (path, e.strerror)) from None
    try:
        return read_open(fd, path, moved=lambda: _where(fd) != path)
    finally:
        os.close(fd)


def read_open(fd, path, owner=None, refused=None, moved=None):
    """`(bytes, mode)` of the open file `fd`, named `path` in the sentences: refused unless it is
    a regular file with one name, of at most MAX_BYTES, owned by `owner` when that is given, and
    (with `moved`) still where it was judged. `refused` words a refusal with its reason."""
    def no(reason):
        return OfficeError(refused % (path, reason) if refused else "%s %s" % (path, reason))
    st = os.fstat(fd)
    if not stat.S_ISREG(st.st_mode):
        raise no("is not a regular file")
    if st.st_nlink > 1:
        raise no("has a second name (a hard link), which could be a protected file under "
                 "another name; it is not opened for an agent")
    if owner is not None and st.st_uid != owner:
        raise no("belongs to someone else")
    if st.st_size > MAX_BYTES:
        raise no("is %d bytes; an agent's LibreOffice takes files up to %d"
                 % (st.st_size, MAX_BYTES))
    if moved is not None and moved():
        raise OfficeError("%s moved while it was being opened; nothing was copied" % path)
    chunks, size = [], 0
    while True:
        chunk = os.read(fd, 1 << 20)
        if not chunk:
            break
        size += len(chunk)
        if size > MAX_BYTES:
            raise no("grew past %d bytes while it was read" % MAX_BYTES)
        chunks.append(chunk)
    return b"".join(chunks), stat.S_IMODE(st.st_mode)


def digest_of(data):
    return hashlib.sha256(data).hexdigest()


def _write_new(path, data, mode=0o600, dir_fd=None):
    """Create `path` (never replacing a file) holding `data`; a file it made and could not fill
    is removed again."""
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, mode,
                 dir_fd=dir_fd)
    try:
        view = memoryview(data)
        while view:
            view = view[os.write(fd, view):]
        os.fsync(fd)
    except OSError:
        os.close(fd)
        os.unlink(path, dir_fd=dir_fd)
        raise
    os.close(fd)


def take(given, real, work, folder):
    """Copy the file at `real` (what `given` resolved to, both already judged by the rule) to the
    new file `work` in the sandbox's work folder `folder` (a workdir.WorkFolder)."""
    data, mode = _read(real)
    folder.write_new(work, data)
    return Copy(given, real, work, digest_of(data), mode)


def unchanged(copy):
    """Whether the person's file still holds what was copied in."""
    try:
        data, _ = _read(copy.real)
    except OfficeError:
        return False
    return digest_of(data) == copy.digest


def _folder(path):
    """An open handle on the folder `path`, checked to be that folder and not a link to another."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError as e:
        raise OfficeError("the folder %s could not be opened (%s); nothing was written"
                          % (path, e.strerror)) from None
    if _where(fd) != path:
        os.close(fd)
        raise OfficeError("the folder %s moved; nothing was written" % path)
    return fd


def _stamp(st):
    """What says a file is the same one, unwritten: its identity, size and change times."""
    return st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns, st.st_ctime_ns


def _held(dir_fd, name, copy):
    """The stamp of the person's file `name` in `dir_fd` when it still holds what was copied in,
    read through the folder's handle; None when it changed."""
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                     dir_fd=dir_fd)
    except OSError:
        return None
    try:
        data, _ = read_open(fd, copy.given)
        st = os.fstat(fd)
    except OfficeError:
        return None
    finally:
        os.close(fd)
    return _stamp(st) if digest_of(data) == copy.digest else None


def put_back(copy, data):
    """Replace the person's file with `data`, the saved copy's bytes as workdir.py read them, at
    once, unless it changed since it was copied in. Updates `copy.digest`.

    POSIX has no rename that compares first, so the check is made twice: the digest, through the
    folder's handle, before the new bytes are written beside it, and the file's identity and
    change times again just before the rename. A write in the few instructions between that last
    look and the rename is the window left (review L4)."""
    folder, name = os.path.split(copy.real)
    dir_fd = _folder(folder)
    temporary = ".~yantrik-agent-%s.tmp" % secrets.token_hex(6)
    try:
        stamp = _held(dir_fd, name, copy)
        if stamp is None:
            raise OfficeError(CHANGED % copy.given)
        _write_new(temporary, data, copy.mode, dir_fd=dir_fd)
        try:
            if _stamp(os.stat(name, dir_fd=dir_fd, follow_symlinks=False)) != stamp:
                raise OfficeError(CHANGED % copy.given)
            os.replace(temporary, name, src_dir_fd=dir_fd, dst_dir_fd=dir_fd)
        except BaseException:
            os.unlink(temporary, dir_fd=dir_fd)
            raise
        os.fsync(dir_fd)
    except OSError as e:
        raise OfficeError("%s could not be written (%s); it is as it was"
                          % (copy.given, e.strerror)) from None
    finally:
        os.close(dir_fd)
    copy.digest = digest_of(data)


def create(data, folder, name, shown):
    """Write `data` (bytes workdir.py read from the sandbox) as the new file `name` in the real
    folder `folder`, never replacing one; `shown` is the path the caller named, for the
    sentences. Returns the mode it was made with."""
    mode = 0o666 & ~_umask()
    dir_fd = _folder(folder)
    try:
        _write_new(name, data, mode, dir_fd=dir_fd)
    except FileExistsError:
        raise OfficeError("a file is already at %s, and this action never replaces one; "
                          "choose another path" % shown) from None
    except OSError as e:
        raise OfficeError("%s could not be written (%s)" % (shown, e.strerror)) from None
    finally:
        os.close(dir_fd)
    return mode


def _umask():
    """This process's umask, read without changing it (os.umask would, for every thread)."""
    try:
        with open("/proc/self/status", encoding="ascii") as f:
            for line in f:
                if line.startswith("Umask:"):
                    return int(line.split()[1], 8)
    except (OSError, ValueError, IndexError):
        pass
    return 0o022
