"""Landlock's abstract-socket scope for a process and all it starts (#411, M1).

An abstract AF_UNIX socket (`@/tmp/.X11-unix/X0`, the person's Xwayland) is not a file, so
PrivateTmp= and ProtectHome= do not hide it, and RestrictAddressFamilies=AF_UNIX does not stop a
connect to it. systemd has no property for it. Landlock (kernel 6.12, ABI 6) does:
LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET refuses a connect to an abstract socket made by a process
outside the domain. A unit's processes keep talking to each other, and to nothing the person runs.
The scope is inherited by every child and cannot be lifted.

Stdlib only; syscall numbers 444-446 are the same on every architecture Linux has them on.
"""
import ctypes
import os

SCOPE_ABSTRACT_UNIX_SOCKET = 1
CREATE_VERSION = 1 << 0
SYS_CREATE, SYS_RESTRICT = 444, 446
PR_SET_NO_NEW_PRIVS = 38
MIN_ABI = 6


class Unavailable(OSError):
    """The kernel cannot scope abstract sockets (or the call was refused)."""


def _libc():
    return ctypes.CDLL(None, use_errno=True)


def abi(libc=None):
    """The kernel's Landlock ABI version; 0 when it has none."""
    libc = libc or _libc()
    got = libc.syscall(SYS_CREATE, None, ctypes.c_size_t(0), ctypes.c_uint32(CREATE_VERSION))
    return max(got, 0)


def scope_abstract_unix():
    """Refuse this process, and every child, a connect to an abstract socket made outside it.

    Raises Unavailable if the kernel is too old or the call fails: the caller decides whether that
    stops it.
    """
    libc = _libc()
    have = abi(libc)
    if have < MIN_ABI:
        raise Unavailable(0, "Landlock ABI %d, abstract-socket scope needs %d (kernel 6.12)" % (have, MIN_ABI))
    # struct landlock_ruleset_attr {handled_access_fs; handled_access_net; scoped;}: nothing handled
    # but the scope, so files and the network are left to the unit's other properties.
    attr = (ctypes.c_uint64 * 3)(0, 0, SCOPE_ABSTRACT_UNIX_SOCKET)
    fd = libc.syscall(SYS_CREATE, attr, ctypes.c_size_t(ctypes.sizeof(attr)), ctypes.c_uint32(0))
    if fd < 0:
        err = ctypes.get_errno()
        raise Unavailable(err, "landlock_create_ruleset: " + os.strerror(err))
    try:
        if libc.prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0:
            err = ctypes.get_errno()
            raise Unavailable(err, "no_new_privs: " + os.strerror(err))
        if libc.syscall(SYS_RESTRICT, ctypes.c_int(fd), ctypes.c_uint32(0)) != 0:
            err = ctypes.get_errno()
            raise Unavailable(err, "landlock_restrict_self: " + os.strerror(err))
    finally:
        os.close(fd)
