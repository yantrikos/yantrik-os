"""Who may hand this service a browser's DevTools pipe, and whether the pipe is that browser's (#1151).

`take` used to believe a header: any process whose program was *named* yantrik-ui, with any pid.
Here the caller is the installed shell by identity (the file behind `/proc/<pid>/exe`, compared by
device and inode with the root-owned install), the pid it names is a browser it started with the
pipe switch and no port, and the stream it sends is that browser's fds 3 and 4 (the kernel's own
record of who is at the other end of the socket).

A process of the person's own account can still be made to look like these with effort (it can
rewrite its own argv); what this closes is the cheap routes: a renamed copy, a made-up pid, a
stream that is not the browser's. The copy made inside a private mount namespace, placed at the
installed path and unlinked so that `exe` reads `(deleted)` (review of aff60d15, N2), is closed
by reading the file behind `exe` itself: it must be root's and writable by nobody else, and the
caller must share this service's mount and user namespaces.
"""

import os
import socket
import struct

INSTALLED_SHELL = "/opt/yantrik/bin/yantrik-ui"
PIPE_FLAG = "--remote-debugging-pipe"
_DELETED = " (deleted)"

_NETLINK_SOCK_DIAG, _SOCK_DIAG_BY_FAMILY, _UNIX_DIAG_PEER, _SHOW_PEER = 4, 20, 2, 4
_NLM_F_REQUEST, _NLMSG_HDR, _DIAG_MSG = 1, 16, 16
_NO_COOKIE = 0xFFFFFFFFFFFFFFFF   # "do not match a cookie"; anything else is ESTALE


class NotProvable(Exception):
    """A fact about a process could not be read; the hand-over is refused, never allowed."""


def shell_is_installed(pid, installed=INSTALLED_SHELL, proc="/proc", owner=0):
    """None when `pid` runs the installed shell; otherwise why not. A shell whose file an update
    has since replaced reads `<path> (deleted)`: that is the installed path, and still the shell,
    as long as the file behind `exe` is `owner`'s (root's) and nobody else may write it, which
    a copy an account made for itself is not."""
    at = os.path.join(proc, str(pid))
    try:
        exe = os.readlink(os.path.join(at, "exe"))
    except OSError as e:
        return "the caller (pid %d) could not be identified (%s)" % (pid, e)
    replaced = exe.endswith(_DELETED)
    if (exe[:-len(_DELETED)] if replaced else exe) != installed:
        return "the caller (pid %d) runs %s, not the installed shell" % (pid, exe)
    try:
        theirs = os.stat(os.path.join(at, "exe"))   # the file itself, deleted or not
    except OSError as e:
        return "the caller's program could not be read (%s)" % e
    if theirs.st_uid != owner or theirs.st_mode & 0o022:
        return "the caller (pid %d) runs a program that is not the installer's alone" % pid
    if not replaced:
        try:
            ours = os.stat(installed)
        except OSError as e:
            return "the installed shell could not be read (%s)" % e
        if (theirs.st_dev, theirs.st_ino) != (ours.st_dev, ours.st_ino):
            return "the caller (pid %d) is not the installed shell's file" % pid
    for ns in ("mnt", "user"):
        try:
            if os.readlink(os.path.join(at, "ns", ns)) != os.readlink(os.path.join(proc, "self", "ns", ns)):
                return "the caller (pid %d) is in another %s namespace" % (pid, ns)
        except OSError as e:
            return "the caller's namespaces could not be read (%s)" % e
    return None


def _parent_of(pid, proc):
    with open(os.path.join(proc, str(pid), "stat"), encoding="ascii", errors="replace") as f:
        return int(f.read().rpartition(")")[2].split()[1])


def browser_is_the_callers(pid, caller_pid, proc="/proc"):
    """None when `pid` is a child of `caller_pid` started with the DevTools pipe and no port."""
    try:
        with open(os.path.join(proc, str(pid), "cmdline"), "rb") as f:
            args = f.read().split(b"\0")
        parent = _parent_of(pid, proc)
    except (OSError, ValueError, IndexError) as e:
        return "that browser (pid %d) could not be read (%s)" % (pid, e)
    flags = [a.decode("utf-8", "replace") for a in args]
    if PIPE_FLAG not in flags or any(a.startswith("--remote-debugging-port") for a in flags):
        return "pid %d was not started with a DevTools pipe and no port" % pid
    if parent != caller_pid:
        return "pid %d is not a browser the caller started" % pid
    return None


def peer_inode(sock):
    """The inode of the socket at the other end of connected unix `sock`, from the kernel
    (sock_diag, UNIX_DIAG_PEER). Raises NotProvable."""
    try:
        ino = os.fstat(sock.fileno()).st_ino
        body = struct.pack("=BBHIIIQ", socket.AF_UNIX, 0, 0, 0xFFFFFFFF, ino, _SHOW_PEER, _NO_COOKIE)
        msg = struct.pack("=IHHII", _NLMSG_HDR + len(body), _SOCK_DIAG_BY_FAMILY, _NLM_F_REQUEST, 1, 0) + body
        with socket.socket(socket.AF_NETLINK, socket.SOCK_DGRAM, _NETLINK_SOCK_DIAG) as nl:
            nl.settimeout(2.0)
            nl.send(msg)
            data = nl.recv(8192)
    except OSError as e:
        raise NotProvable("the kernel would not say who is at the other end of the stream (%s)" % e) from None
    pos = _NLMSG_HDR + _DIAG_MSG
    while pos + 4 <= len(data):
        length, kind = struct.unpack_from("=HH", data, pos)
        if length < 4:
            break
        if kind == _UNIX_DIAG_PEER and length >= 8:
            return struct.unpack_from("=I", data, pos + 4)[0]
        pos += (length + 3) & ~3
    raise NotProvable("the stream has no other end")


def stream_is_the_browsers(sock, pid, proc="/proc"):
    """None when the other end of `sock` is the browser's fd 3 (the stream the shell gave it)."""
    try:
        link = os.readlink(os.path.join(proc, str(pid), "fd", "3"))
        theirs = peer_inode(sock)
    except OSError as e:
        return "the browser's fd 3 could not be read (%s)" % e
    except NotProvable as e:
        return str(e)
    if link != "socket:[%d]" % theirs:
        return "the stream sent is not the one pid %d reads and writes" % pid
    return None
