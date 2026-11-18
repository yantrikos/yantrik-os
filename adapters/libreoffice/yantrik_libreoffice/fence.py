"""The launcher an agent's LibreOffice starts through, inside its sandbox (#942, #953).

sandbox.py builds the namespaces: a mount namespace that shows none of the person's files, a
network namespace with nothing but loopback, and the rest. This file runs first inside them, as
`python3 -I fence.py --allow KIND PATH ... --probe PATH ... --socket PATH ... --abstract NAME ...
--tcp PORT ... -- soffice ...`, and does two things before LibreOffice exists:

  * **Landlock, as a second wall.** Where the kernel offers it, the process (and everything it
    starts) is allowed only the paths it is given, in the sandbox's own paths; a bwrap bug or a
    folder shown by mistake would still meet the kernel's refusal. Where the kernel has no
    Landlock, the namespaces stand alone, and the launcher says so.
  * **The proof.** It must fail to open each probe (the home, each protected place), fail to
    connect to each of the person's sockets and to each canary the adapter left listening outside
    (a path, an abstract name, a TCP port on the host's loopback), and see no network interface
    but loopback and no route out. A refusal counts only as the kernel's own words for "not
    there" or "not allowed"; anything else (too many files open, a timeout) fails the proof.

    What each part proves differs (review L1). The paths (the probes, the person's sockets, the
    path canary) are not in the sandbox's mount namespace at all, so with today's binds they pass
    by not existing: they are the net that catches a later bind that shows one, not a test of
    the namespaces. What tests the namespaces is the rest: the abstract and TCP canaries, which
    only a network namespace of its own refuses, and the interface and route checks.

Only then does it exec the command. If any step fails it exits with `REFUSED` and a sentence on
stderr, and the command never runs: there is no unsandboxed fallback. Only the standard library is
used, because `-I` runs it with nothing else on the path.
"""

import ctypes
import errno
import os
import socket
import stat
import sys

# The exit status of a launch the fence refused; nothing was started.
REFUSED = 78

# Landlock's system calls have one number on every architecture (since Linux 5.13).
_CREATE_RULESET, _ADD_RULE, _RESTRICT_SELF = 444, 445, 446
_CREATE_RULESET_VERSION = 1
_RULE_PATH_BENEATH = 1
_PR_SET_NO_NEW_PRIVS = 38

# Filesystem rights (include/uapi/linux/landlock.h).
EXECUTE, WRITE_FILE, READ_FILE, READ_DIR = 1 << 0, 1 << 1, 1 << 2, 1 << 3
REMOVE_DIR, REMOVE_FILE, MAKE_CHAR, MAKE_DIR = 1 << 4, 1 << 5, 1 << 6, 1 << 7
MAKE_REG, MAKE_SOCK, MAKE_FIFO, MAKE_BLOCK = 1 << 8, 1 << 9, 1 << 10, 1 << 11
MAKE_SYM, REFER, TRUNCATE, IOCTL_DEV = 1 << 12, 1 << 13, 1 << 14, 1 << 15
# The rights each ABI version knows: a ruleset handles all of them, so none is left open.
_FS_BY_ABI = {1: (1 << 13) - 1, 2: (1 << 14) - 1, 3: (1 << 15) - 1, 4: (1 << 15) - 1}
_FS_LATEST = (1 << 16) - 1
# The rights a rule on a file (not a folder) may carry.
_FILE_RIGHTS = EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE | IOCTL_DEV
# ABI 4: TCP. Handled with no rule allowing any port, so the process opens no TCP connection.
_NET_BIND_TCP, _NET_CONNECT_TCP = 1 << 0, 1 << 1
# ABI 6: no abstract unix socket and no signal reaches a process outside the fence.
_SCOPE_ABSTRACT_UNIX, _SCOPE_SIGNAL = 1 << 0, 1 << 1

_READ = READ_FILE | READ_DIR
# What each kind of allowed place may do. No kind has MAKE_SYM: the adapter outside reads the
# folders the fenced process writes, and a link made there could lead it to one of the person's
# files (#953 review H1, H2). workdir.py and unopipe.py never follow one either, for a kernel
# without Landlock; this is the second wall.
KINDS = {
    "system": _READ | EXECUTE,
    "work": _READ | WRITE_FILE | REMOVE_DIR | REMOVE_FILE | MAKE_DIR | MAKE_REG | REFER | TRUNCATE,
    "private": _READ | EXECUTE | WRITE_FILE | REMOVE_DIR | REMOVE_FILE | MAKE_DIR | MAKE_REG
               | MAKE_SOCK | MAKE_FIFO | REFER | TRUNCATE,
    "devices": _READ | WRITE_FILE | MAKE_REG | REMOVE_FILE | TRUNCATE | IOCTL_DEV,
    # The UNO pipe is a socket LibreOffice makes in /tmp; it may make one there and remove it,
    # and read nothing in /tmp.
    "sockets": MAKE_SOCK | REMOVE_FILE,
}


class FenceError(Exception):
    """The fence could not be put up, or did not hold: the sentence says which."""


class _RulesetAttr(ctypes.Structure):
    _fields_ = [("handled_access_fs", ctypes.c_uint64), ("handled_access_net", ctypes.c_uint64),
                ("scoped", ctypes.c_uint64)]


class _PathBeneathAttr(ctypes.Structure):
    _pack_ = 1
    _fields_ = [("allowed_access", ctypes.c_uint64), ("parent_fd", ctypes.c_int32)]


def _libc():
    if not sys.platform.startswith("linux"):
        raise FenceError("Landlock is Linux's, and this is %s" % sys.platform)
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    return libc


def _failed(what):
    code = ctypes.get_errno()
    return FenceError("%s failed: %s" % (what, os.strerror(code) if code else "unknown error"))


def abi():
    """The Landlock ABI version this kernel offers. Raises `FenceError` when it offers none: an
    old kernel, or one booted without Landlock among its security modules."""
    libc = _libc()
    version = libc.syscall(_CREATE_RULESET, None, ctypes.c_size_t(0),
                           ctypes.c_uint32(_CREATE_RULESET_VERSION))
    if version < 1:
        raise FenceError("this kernel offers no Landlock (%s)" % os.strerror(ctypes.get_errno()))
    return int(version)


def restrict(rules):
    """Fence this process, and everything it starts from now on, to `rules`: (kind, path) pairs,
    `kind` a key of KINDS. A path that does not exist is skipped; a link is never followed, so a
    rule names the place itself and not where a link points. Raises `FenceError`, having changed
    nothing it cannot take back, when Landlock is missing or refuses a step."""
    version = abi()
    libc = _libc()
    handled = _FS_BY_ABI.get(version, _FS_LATEST)
    attr = _RulesetAttr(handled_access_fs=handled)
    size = 8
    if version >= 4:
        attr.handled_access_net, size = _NET_BIND_TCP | _NET_CONNECT_TCP, 16
    if version >= 6:
        attr.scoped, size = _SCOPE_ABSTRACT_UNIX | _SCOPE_SIGNAL, 24
    ruleset = libc.syscall(_CREATE_RULESET, ctypes.byref(attr), ctypes.c_size_t(size),
                           ctypes.c_uint32(0))
    if ruleset < 0:
        raise _failed("making a Landlock ruleset")
    try:
        for kind, path in rules:
            _allow(libc, ruleset, KINDS[kind] & handled, path)
        if libc.prctl(_PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0:
            raise _failed("turning off new privileges")
        if libc.syscall(_RESTRICT_SELF, ctypes.c_int(ruleset), ctypes.c_uint32(0)) != 0:
            raise _failed("applying the Landlock ruleset")
    finally:
        os.close(ruleset)
    return version


def _allow(libc, ruleset, access, path):
    try:
        fd = os.open(path, os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC)
    except FileNotFoundError:
        return
    except OSError as e:
        raise FenceError("%s cannot be allowed: %s" % (path, e.strerror)) from None
    try:
        mode = os.fstat(fd).st_mode
        if stat.S_ISLNK(mode):
            return
        if not stat.S_ISDIR(mode):
            access &= _FILE_RIGHTS
        rule = _PathBeneathAttr(allowed_access=access, parent_fd=fd)
        if libc.syscall(_ADD_RULE, ctypes.c_int(ruleset), ctypes.c_int(_RULE_PATH_BENEATH),
                        ctypes.byref(rule), ctypes.c_uint32(0)) != 0:
            raise _failed("allowing %s" % path)
    finally:
        os.close(fd)


# The kernel's words for "not there" and "not allowed": the only failures a proof accepts
# (#942 review L3). ENOTDIR is a path through a file, which leads nowhere.
_ABSENT_OR_REFUSED = (errno.ENOENT, errno.EACCES, errno.EPERM, errno.ENOTDIR)
# And for a connection: nothing listening, no route, or not allowed.
_UNREACHED = _ABSENT_OR_REFUSED + (errno.ECONNREFUSED, errno.ENETUNREACH, errno.EHOSTUNREACH,
                                   errno.EADDRNOTAVAIL)
# An address no network routes to (TEST-NET-1, RFC 5737): only a sandbox with no route out
# refuses to aim a UDP socket at it.
_NOWHERE = ("192.0.2.1", 9)


def _masked(fd):
    """Whether what was opened is a cover the sandbox laid over a place: an empty folder, or
    /dev/null."""
    st = os.fstat(fd)
    if stat.S_ISDIR(st.st_mode):
        return not os.listdir(fd)
    if stat.S_ISCHR(st.st_mode):
        try:
            return st.st_rdev == os.stat("/dev/null").st_rdev
        except OSError:
            return False
    return False


def _unproved(what, e):
    """The proof's failure for an error that is neither absent nor refused (a timeout, too many
    files open, a loop of links): such an error says nothing about the sandbox."""
    said = errno.errorcode.get(e.errno, str(e.errno)) if e.errno else (str(e) or type(e).__name__)
    return FenceError("the sandbox could not be proved: %s failed with %s, which is neither "
                      "absent nor refused" % (what, said))


def prove(probes):
    """Raise `FenceError` unless every probe is not there, refused, or covered: run after the
    sandbox and Landlock are up, so one that went up wrong is caught before anything runs. Opened
    with O_NONBLOCK, so a probe that is a FIFO cannot hang the launch."""
    for path in probes:
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOCTTY)
        except OSError as e:
            if e.errno in _ABSENT_OR_REFUSED:
                continue
            raise _unproved("opening %s" % path, e) from None
        try:
            if _masked(fd):
                continue
        finally:
            os.close(fd)
        raise FenceError("the sandbox did not hold: %s could still be opened" % path)


def _connect(family, address, kind=socket.SOCK_STREAM):
    sock = socket.socket(family, kind)
    sock.settimeout(2.0)
    try:
        sock.connect(address)
    finally:
        sock.close()


def _reached(what, family, address, kind=socket.SOCK_STREAM):
    """Whether `address` can be connected to; raises `FenceError` for an error that proves
    nothing either way."""
    try:
        _connect(family, address, kind)
    except OSError as e:
        if e.errno in _UNREACHED:
            return False
        raise _unproved(what, e) from None
    return True


def prove_sockets(paths, abstract):
    """Raise `FenceError` unless no UNIX socket in `paths` and no abstract name in `abstract` can
    be connected to (the person's LibreOffice pipe, the session bus, the canaries: #942 M1)."""
    for label, address in [(p, p) for p in paths] + [("@" + n, "\0" + n) for n in abstract]:
        if _reached("connecting to %s" % label, socket.AF_UNIX, address):
            raise FenceError("the sandbox did not hold: the socket %s could still be reached"
                             % label)


def prove_network(ports):
    """Raise `FenceError` unless the only interface is loopback, no TCP `ports` the adapter is
    listening on (on the host's loopback) answer, and no route leads out (UDP, which Landlock never
    handles, included: #942 L2)."""
    try:
        names = sorted(name for _, name in socket.if_nameindex())
    except OSError as e:
        raise _unproved("listing the network interfaces", e) from None
    if names not in ([], ["lo"]):
        raise FenceError("the sandbox did not hold: it has network interfaces (%s)"
                         % ", ".join(names))
    for port in ports:
        if _reached("TCP to port %d" % port, socket.AF_INET, ("127.0.0.1", port)):
            raise FenceError("the sandbox did not hold: TCP port %d on the host could be reached"
                             % port)
    if _reached("a route out", socket.AF_INET, _NOWHERE, socket.SOCK_DGRAM):
        raise FenceError("the sandbox did not hold: there is a route out of it")


def command(python, launcher, rules, probes, argv, sockets=(), abstract=(), ports=()):
    """The argv that runs `argv` through the launcher at `launcher`, under `python` -I."""
    out = [python, "-I", launcher]
    for kind, path in rules:
        out += ["--allow", kind, path]
    for flag, values in (("--probe", probes), ("--socket", sockets), ("--abstract", abstract),
                         ("--tcp", [str(p) for p in ports])):
        for value in values:
            out += [flag, value]
    return out + ["--"] + list(argv)


_LISTS = {"--probe": "probes", "--socket": "sockets", "--abstract": "abstract", "--tcp": "ports"}


def _parse(argv):
    told = {"rules": [], "probes": [], "sockets": [], "abstract": [], "ports": []}
    rest = list(argv)
    while rest and rest[0] != "--":
        flag = rest.pop(0)
        if flag == "--allow" and len(rest) >= 2 and rest[0] in KINDS:
            told["rules"].append((rest.pop(0), rest.pop(0)))
        elif flag in _LISTS and rest:
            value = rest.pop(0)
            if flag == "--tcp":
                if not value.isdigit():
                    raise FenceError("the launcher was given a port that is not one (%r)" % value)
                value = int(value)
            told[_LISTS[flag]].append(value)
        else:
            raise FenceError("the launcher was not told what to allow (%r)" % flag)
    if len(rest) < 2:
        raise FenceError("the launcher was given no command to run")
    return told, rest[1:]


def main(argv=None):
    """The launcher: Landlock where offered, the proof, then exec. Never runs the command in a
    sandbox it could not prove."""
    try:
        told, run = _parse(sys.argv[1:] if argv is None else argv)
        try:
            abi()
        except FenceError as e:  # no Landlock here: the namespaces stand alone
            landlock = "no Landlock (%s)" % e
        else:
            landlock = "Landlock ABI %d" % restrict(told["rules"])
        prove(told["probes"])
        prove_sockets(told["sockets"], told["abstract"])
        prove_network(told["ports"])
    except FenceError as e:
        print("yantrik-libreoffice-sandbox: refused: %s" % e, file=sys.stderr, flush=True)
        return REFUSED
    print("yantrik-libreoffice-sandbox: proved, with %s" % landlock, file=sys.stderr, flush=True)
    try:
        os.execvp(run[0], run)
    except OSError as e:
        print("yantrik-libreoffice-sandbox: %s would not start: %s" % (run[0], e.strerror),
              file=sys.stderr, flush=True)
        return REFUSED
    return REFUSED  # not reached: exec replaced this process


if __name__ == "__main__":
    sys.exit(main())
