"""The UNO pipe of the agents' LibreOffice, reached only where its sandbox cannot reach (#953,
review H2).

LibreOffice makes its pipe in /tmp, and inside the sandbox /tmp is the adapter's `pipes` folder,
which the sandbox can write. A LibreOffice taken over in there could put a link where its socket
was, to the person's own LibreOffice pipe in the real /tmp, and the adapter, connecting from
outside by that name, would hand the agent's calls to the person's unsandboxed LibreOffice.
Landlock forbids making that link, but not every kernel has Landlock. So the adapter never
connects by a name in that folder:

  * it pins the socket the sandbox made: looked at through a handle on the folder, never
    followed, required to be a socket of the adapter's own uid, and hard-linked (the link
    itself, never what it points to) into the adapter's `uno` folder, which the sandbox is not
    shown. The pinned name is checked to be the very inode looked at. A UNIX socket is reached
    by its inode, so the pinned name reaches the listener the sandbox made, whatever is later
    done to the name inside, and no later bind can take that inode over;
  * the listener on it is asked who it is (SO_PEERCRED): it must run as the adapter's uid and
    descend from the sandbox's bwrap, which the person's LibreOffice never does;
  * the name a UNO client looks for, /tmp/OSL_PIPE_<uid>_<pipe>, is made a link to the pinned
    name, and before every connection (`check`, Office's `reach`) the link and the pinned inode
    are looked at again. Anything else is refused, never connected to.

The pipe's name is random per sandbox, so it is never one the person's LibreOffice listens on.
"""

import os
import socket
import stat
import struct

from .basics import OfficeError

_DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
_UCRED = struct.Struct("3i")
# How far up the process tree a listener is looked for under the sandbox's bwrap.
MAX_DEPTH = 64

REFUSED = ("the agents' LibreOffice pipe is not the one its sandbox made (%s), so it was not "
           "connected to; nothing was done")


class PipeRefused(OfficeError):
    """The pipe is not the sandbox's: never retried, never connected to."""


def _refuse(why):
    return PipeRefused(REFUSED % why)


def parent_of(pid):
    """The parent of process `pid`, from /proc; None when it is gone."""
    try:
        with open("/proc/%d/stat" % pid, "rb") as f:
            return int(f.read().rsplit(b")", 1)[1].split()[1])
    except (OSError, ValueError, IndexError):
        return None


def descends(pid, ancestor, parent=parent_of):
    """Whether `pid` is a process below `ancestor` (never `ancestor` itself)."""
    for _ in range(MAX_DEPTH):
        pid = parent(pid)
        if pid is None or pid <= 0:
            return False
        if pid == ancestor:
            return True
    return False


class Pipe:
    """The pipe `name` the sandboxed LibreOffice listens on in `served` (the sandbox's /tmp, as
    the adapter names it), pinned into `pinned` (a 0700 folder of the adapter's the sandbox is not
    shown) and served only by a process below `sandbox_pid`."""

    def __init__(self, name, served, pinned, sandbox_pid, tmp="/tmp"):
        self.uid = os.getuid()
        self.file = "OSL_PIPE_%d_%s" % (self.uid, name)
        self.link = os.path.join(tmp, self.file)
        self.pinned = os.path.join(pinned, self.file)
        self.sandbox_pid = sandbox_pid
        self._served = os.open(served, _DIR)
        self._pinned = os.open(pinned, _DIR)
        self._inode = None
        self._linked = False

    def made(self):
        """Whether the sandbox has made anything by the pipe's name yet (never followed)."""
        try:
            os.stat(self.file, dir_fd=self._served, follow_symlinks=False)
        except FileNotFoundError:
            return False
        return True

    def pin(self):
        """Pin the sandbox's socket and link the UNO name to it; `PipeRefused` for anything that is
        not a socket of the sandbox's own."""
        try:
            st = os.stat(self.file, dir_fd=self._served, follow_symlinks=False)
        except OSError as e:
            raise _refuse(e.strerror) from None
        if not stat.S_ISSOCK(st.st_mode):
            raise _refuse("what is there is not a socket")
        if st.st_uid != self.uid:
            raise _refuse("its socket belongs to someone else")
        try:
            os.link(self.file, self.file, src_dir_fd=self._served, dst_dir_fd=self._pinned,
                    follow_symlinks=False)
        except OSError as e:
            raise _refuse("it could not be pinned: %s" % e.strerror) from None
        held = os.stat(self.file, dir_fd=self._pinned, follow_symlinks=False)
        if (held.st_dev, held.st_ino) != (st.st_dev, st.st_ino) or not stat.S_ISSOCK(held.st_mode):
            os.unlink(self.file, dir_fd=self._pinned)
            raise _refuse("it changed while it was pinned")
        self._inode = (held.st_dev, held.st_ino)
        self._ask_listener()
        try:
            os.symlink(self.pinned, self.link)
        except OSError as e:
            raise _refuse("%s could not be made: %s" % (self.link, e.strerror)) from None
        self._linked = True

    def _ask_listener(self):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(5)
        try:
            s.connect(self.pinned)
            pid, uid, _ = _UCRED.unpack(s.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED,
                                                     _UCRED.size))
        except OSError as e:
            raise _refuse("its listener would not answer: %s" % (e.strerror or e)) from None
        finally:
            s.close()
        if uid != self.uid:
            raise _refuse("its listener runs as uid %d" % uid)
        if not descends(pid, self.sandbox_pid):
            raise _refuse("its listener (process %d) is not in the sandbox" % pid)

    def check(self):
        """Before every connection: the UNO name is still our link to the pinned socket."""
        if self._inode is None:
            raise _refuse("it was never pinned")
        try:
            st = os.lstat(self.link)
            target = os.readlink(self.link) if stat.S_ISLNK(st.st_mode) else None
            held = os.stat(self.file, dir_fd=self._pinned, follow_symlinks=False)
        except OSError as e:
            raise _refuse(e.strerror) from None
        if target != self.pinned or st.st_uid != self.uid:
            raise _refuse("%s is no longer the adapter's link" % self.link)
        if (held.st_dev, held.st_ino) != self._inode or not stat.S_ISSOCK(held.st_mode):
            raise _refuse("the pinned socket was replaced")

    def remove(self):
        """Take the UNO name and the pinned socket away, and let go of the folders."""
        if self._linked:
            try:
                if os.readlink(self.link) == self.pinned:
                    os.remove(self.link)
            except OSError:
                pass
            self._linked = False
        if self._served is None:
            return
        try:
            os.unlink(self.file, dir_fd=self._pinned)
        except OSError:
            pass
        os.close(self._served)
        os.close(self._pinned)
        self._served = self._pinned = None
