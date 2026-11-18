"""The sandbox's work folder, as the adapter outside it reaches into it (#953, review H1).

The work folder is the one place the sandboxed LibreOffice can write, so everything in it is the
sandbox's to change: a document a filter exploit or a macro has taken over can rename what is
there and put links in its place, to the person's key or to a folder of the person's. The adapter
reads and writes there with the person's rights, outside every namespace, so it must never follow
anything it finds there. This file is the only way in:

  * the work folder itself is held by an open handle, and is what the sandbox cannot rename (a
    mount point inside, under a parent the sandbox is not shown);
  * each document gets a slot of its own, a folder with a fresh random name, made and opened here
    and held by a handle from then on. Renaming it, or putting a link where it was, changes
    nothing the adapter does, because the adapter never names it by path again;
  * a file is reached only as one name inside a slot's handle, never followed if it is a link,
    and read only when it is a regular file of the adapter's own uid with one name (a second name
    could be anything) and of a size the adapter takes.

Nothing here names a path of the work folder to the host's file functions; the paths kept are
only what is handed to the sandboxed LibreOffice, which sees them inside its own world.
"""

import os
import secrets
import shutil
import stat

from .basics import OfficeError
from .copies import MAX_BYTES, read_open

_DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
_FILE = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
_NEW = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC

TAMPERED = ("what the agents' LibreOffice left at %s is not a plain file of its own (%s), so it "
            "was not read; nothing reached the person's files")


class WorkFolder:
    """The sandbox's work folder at `path` (the adapter's own path for it), held open."""

    def __init__(self, path):
        self.path = os.path.normpath(path)
        self._fd = os.open(self.path, _DIR)
        self._dev = os.fstat(self._fd).st_dev
        self._slots = {}    # slot name -> handle on that slot's folder

    def slot(self):
        """A new, empty folder for one document; its path, as the sandbox's LibreOffice is told."""
        name = secrets.token_hex(8)
        os.mkdir(name, 0o700, dir_fd=self._fd)
        try:
            fd = os.open(name, _DIR, dir_fd=self._fd)
        except OSError as e:
            raise OfficeError("the agents' work folder changed under the adapter (%s); nothing "
                              "was copied" % e.strerror) from None
        st = os.fstat(fd)
        if st.st_uid != os.getuid() or st.st_dev != self._dev:
            os.close(fd)
            raise OfficeError("the agents' work folder changed under the adapter; nothing was "
                              "copied")
        self._slots[name] = fd
        return os.path.join(self.path, name)

    def _locate(self, path):
        """(slot handle, file name) for a path one name inside a slot; refused for any other."""
        rel = os.path.relpath(os.path.normpath(path), self.path).split(os.sep)
        if len(rel) != 2 or rel[0] not in self._slots or rel[1] in ("", ".", ".."):
            raise OfficeError("%s is not a file this adapter made room for" % path)
        return self._slots[rel[0]], rel[1]

    def read(self, path):
        """The bytes of the file at `path`, refused unless it is one plain file of our own."""
        slot, name = self._locate(path)
        try:
            fd = os.open(name, _FILE, dir_fd=slot)
        except OSError as e:
            raise OfficeError(TAMPERED % (path, e.strerror)) from None
        try:
            return read_open(fd, path, owner=os.getuid(), refused=TAMPERED)[0]
        finally:
            os.close(fd)

    def write_new(self, path, data):
        """Create the file at `path`, never replacing one."""
        slot, name = self._locate(path)
        fd = os.open(name, _NEW, 0o600, dir_fd=slot)
        try:
            view = memoryview(data)
            while view:
                view = view[os.write(fd, view):]
        finally:
            os.close(fd)

    def replace(self, path, data):
        """Put `data` at `path`, whatever is there now (a link is replaced, never followed)."""
        slot, name = self._locate(path)
        temporary = ".~yantrik-%s.tmp" % secrets.token_hex(6)
        self.write_new(os.path.join(os.path.dirname(path), temporary), data)
        os.replace(temporary, name, src_dir_fd=slot, dst_dir_fd=slot)

    def remove(self, path):
        slot, name = self._locate(path)
        try:
            os.unlink(name, dir_fd=slot)
        except FileNotFoundError:
            pass

    def size(self, path):
        """The size of the plain file at `path`, or None when nothing is there."""
        slot, name = self._locate(path)
        try:
            st = os.stat(name, dir_fd=slot, follow_symlinks=False)
        except FileNotFoundError:
            return None
        if not stat.S_ISREG(st.st_mode) or st.st_size > MAX_BYTES:
            raise OfficeError(TAMPERED % (path, "a link, or no regular file of a size it takes"))
        return st.st_size

    def discard(self, path):
        """Close the slot at `path` and remove what is under the work folder by that name. The
        work folder's own path is fixed, so removing below it reaches nothing outside."""
        fd = self._slots.pop(os.path.basename(os.path.normpath(path)), None)
        if fd is not None:
            os.close(fd)
            shutil.rmtree(os.path.join(self.path, os.path.basename(path)), ignore_errors=True)

    def close(self):
        for name in list(self._slots):
            self.discard(os.path.join(self.path, name))
        if self._fd is not None:
            os.close(self._fd)
            self._fd = None
