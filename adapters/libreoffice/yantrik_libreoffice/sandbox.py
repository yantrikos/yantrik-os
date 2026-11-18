"""The sandbox an agent's LibreOffice runs in: bubblewrap namespaces, with Landlock inside (#953).

A formula, a link, a macro, a filter or an export reads and connects with the rights of the
process that evaluates it. Five rounds of review found ways round a scan of formulas, and then
round a Landlock fence alone: Landlock cannot refuse `connect()` to a UNIX socket by path, so the
person's own LibreOffice pipe and the session bus stayed reachable (#942, M1). So the LibreOffice
that does an agent's work now starts in a world of its own, built by `bwrap`:

  * its own mount namespace, on an empty root: only what is bound below exists in it. That is
    /usr and a short list of /etc read-only, the few places LibreOffice's own links point to
    (Debian's /var/spool/libreoffice, #942 M2), a fresh /proc for its own processes, a minimal
    /dev with a private /dev/shm, and four private folders of the adapter's: its profile, its
    temporary files, the copies of the documents it works on, and a /tmp holding nothing but its
    own UNO pipe. The person's home, /run (the session bus, the display, every Yantrik socket)
    and the real /tmp are not there to name;
  * its own network namespace: no interface but loopback, so no TCP, no UDP, and no abstract
    UNIX socket of the session's (those belong to a network namespace);
  * its own IPC, PID and UTS namespaces, a new session (no terminal to push keys into), no
    capabilities, and it dies with the adapter;
  * an environment of its own: no DBUS_SESSION_BUS_ADDRESS, XDG_RUNTIME_DIR, DISPLAY or
    WAYLAND_DISPLAY.

Inside it, fence.py puts Landlock up as a second wall where the kernel offers it, then proves the
sandbox holds before LibreOffice starts: the person's protected places and the home cannot be
opened, the person's sockets and three canaries the adapter left listening outside cannot be
reached, and no network leaves. Only then is soffice run. Where bwrap is missing or cannot make
the namespaces, nothing is started and the agent's call is refused.

This module only builds the command: it starts nothing but the availability check.
"""

import os
import re
import stat
import subprocess

from .places import parts, within

# Where things are inside the sandbox.
ROOT = "/yantrik"
PROFILE = ROOT + "/profile"
WORK = ROOT + "/work"
TMP = ROOT + "/tmp"
HOME = ROOT + "/home"
LAUNCHER = ROOT + "/bin/fence.py"
# LibreOffice makes its UNO pipe in /tmp, always (sal's osl pipe), so /tmp inside is the folder
# of the adapter's that holds only this LibreOffice's pipes.
PIPES = "/tmp"
PYTHON = "/usr/bin/python3"

# Where bwrap is looked for: the system's own copy, never one on a PATH a caller could set.
BWRAP = ("/usr/bin/bwrap", "/bin/bwrap")

# `--disable-userns` (bwrap 0.8) keeps the sandbox from making user namespaces of its own, which
# are kernel attack surface and could rearrange what it sees (review L2). An older bwrap fails the
# check, and agents are refused.
NAMESPACES = ("--unshare-user", "--unshare-ipc", "--unshare-pid", "--unshare-net",
              "--unshare-uts", "--unshare-cgroup-try", "--disable-userns")

# What of /etc LibreOffice needs: the dynamic loader, the user and group names (sal names its pipe
# by the uid it reads back), fonts, its own settings, the time zone and the locale. Nothing else
# of /etc is there.
ETC = ("alternatives", "fonts", "group", "ld.so.cache", "ld.so.conf", "ld.so.conf.d",
       "libreoffice", "locale.alias", "localtime", "mime.types", "nsswitch.conf", "papersize",
       "passwd", "timezone")
VAR = ("/var/cache/fontconfig",)
TOP = ("/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32")
# Where a link in LibreOffice's install may point and still be shown, read-only, and how many
# folders deep it must be: never a whole /var or /opt, nor one folder straight below them (review
# L3), so a link to /var/lib shows nothing.
LINK_TARGETS = {"/etc": 2, "/var": 3, "/opt": 3}
# Entries looked at in LibreOffice's install for links that point out of it.
MAX_WALKED = 50000
CHECK_SECONDS = 15
_SAFE_LOCALE = re.compile(r"^[A-Za-z0-9_.@+-]{1,64}$")


class SandboxError(Exception):
    """The sandbox cannot be built here: the sentence says why."""


class Private:
    """The adapter's own folder behind one sandbox: made 0700, removed when the sandbox stops.
    Only `pipes`, `profile`, `work` and `tmp` are shown inside; the rest (the launch log, the
    canary, `uno` where the UNO pipe is pinned: unopipe.py) is the adapter's alone."""

    def __init__(self, root):
        self.root = root
        self.pipes = os.path.join(root, "pipes")
        self.profile = os.path.join(root, "profile")
        self.work = os.path.join(root, "work")
        self.tmp = os.path.join(root, "tmp")
        self.uno = os.path.join(root, "uno")
        self.log = os.path.join(root, "launch.log")

    def make(self):
        for path in (self.pipes, self.profile, self.work, self.tmp, self.uno):
            os.mkdir(path, 0o700)
        return self

    def shown(self):
        """(host folder, folder inside) for each private folder the sandbox shows."""
        return [(self.pipes, PIPES), (self.profile, PROFILE), (self.work, WORK), (self.tmp, TMP)]


def find_bwrap(candidates=BWRAP):
    """The system's bwrap: a regular file owned by root that only root can change."""
    for path in candidates:
        try:
            st = os.stat(path)
        except OSError:
            continue
        if stat.S_ISREG(st.st_mode) and st.st_uid == 0 and not st.st_mode & 0o022:
            return path
    raise SandboxError("bubblewrap (bwrap) is not installed where the system keeps it (%s)"
                       % ", ".join(candidates))


def top_links():
    """/bin, /lib and the rest: the links a merged /usr has, or the folders an older one has."""
    out = []
    for top in TOP:
        if os.path.islink(top):
            if within(os.path.realpath(top), "/usr"):
                out += ["--symlink", os.readlink(top), top]
        elif os.path.isdir(top):
            out += ["--ro-bind", top, top]
    return out


def check(bwrap, run=subprocess.run):
    """Make one empty sandbox with the namespaces LibreOffice's will have, and run `true` in it.
    Raises `SandboxError` when it cannot be made: user namespaces off, or bwrap refused."""
    argv = ([bwrap] + list(NAMESPACES) + ["--die-with-parent", "--new-session",
                                          "--ro-bind", "/usr", "/usr"]
            + top_links() + ["--proc", "/proc", "--dev", "/dev", "--", "/usr/bin/true"])
    try:
        done = run(argv, stdin=subprocess.DEVNULL, capture_output=True, timeout=CHECK_SECONDS,
                   env={"PATH": "/usr/bin:/bin"})
    except (OSError, subprocess.SubprocessError) as e:
        raise SandboxError("bwrap would not run (%s)" % e) from None
    if done.returncode != 0:
        said = (done.stderr or b"").decode("utf-8", "replace").strip().splitlines()
        if said and "disable-userns" in said[-1]:
            raise SandboxError("bwrap is older than 0.8, which the sandbox needs to keep user "
                               "namespaces from being made inside it (%s)" % said[-1])
        raise SandboxError("bwrap cannot make the sandbox's namespaces here (%s); unprivileged "
                           "user namespaces may be turned off"
                           % (said[-1] if said else "exit status %d" % done.returncode))


def _shown(path, argv):
    """`--ro-bind` for a path, or `--symlink` for a link that stays under /usr."""
    if os.path.islink(path):
        if within(os.path.realpath(path), "/usr"):
            return argv + ["--symlink", os.readlink(path), path]
        path_real = os.path.realpath(path)
        return argv + ["--ro-bind", path_real, path] if os.path.exists(path_real) else argv
    return argv + ["--ro-bind", path, path]


def linked_targets(root, hidden, walked=MAX_WALKED):
    """Where the links in LibreOffice's install `root` point outside /usr, outermost first, so the
    places they lead to can be shown read-only (Debian's share/uno_packages/cache is a link to
    /var/spool/libreoffice, and soffice aborts without it: #942 M2). A target in the person's
    places, outside /etc, /var and /opt, or too near the top of them, is never shown. Raises `SandboxError` past `walked`
    entries."""
    found, budget = set(), [walked]
    for folder, dirs, files in os.walk(root, followlinks=False):
        budget[0] -= len(dirs) + len(files)
        if budget[0] < 0:
            raise SandboxError("LibreOffice's install at %s has more than %d entries to look "
                               "through for links" % (root, walked))
        for name in dirs + files:
            path = os.path.join(folder, name)
            if not os.path.islink(path):
                continue
            target = os.path.realpath(path)
            if within(target, "/usr") or within(target, root) or not os.path.exists(target):
                continue
            if not any(within(target, base) and len(parts(target)) >= depth
                       for base, depth in LINK_TARGETS.items()):
                continue
            if any(within(target, place) or within(place, target) for place in hidden):
                continue
            found.add(target)
    outer = [t for t in sorted(found) if not any(t != o and within(t, o) for o in found)]
    return outer


def masks(binds, hidden):
    """For each hidden place that lies inside a folder the sandbox shows, the bwrap arguments that
    cover it again: an empty tmpfs over a folder, /dev/null over a file. `binds` are
    (host source, inside) pairs."""
    out, seen = [], set()
    for place in hidden:
        if not os.path.lexists(place):
            continue
        for source, inside in binds:
            if not within(place, source):
                continue
            rel = os.path.relpath(os.path.normpath(place), os.path.normpath(source))
            target = os.path.normpath(os.path.join(inside, rel))
            if target in seen:
                continue
            seen.add(target)
            if os.path.isdir(place):
                out += ["--tmpfs", target]
            else:
                out += ["--ro-bind", "/dev/null", target]
    return out


def environment(env):
    """The sandbox's whole environment: its own home and temporary folder, no display, no bus,
    no runtime folder, and the person's language when it is a plain locale name."""
    out = {"PATH": "/usr/bin:/bin", "HOME": HOME, "TMPDIR": TMP,
           "XDG_CONFIG_HOME": HOME + "/.config", "XDG_CACHE_HOME": HOME + "/.cache",
           "XDG_DATA_HOME": HOME + "/.local/share", "SAL_USE_VCLPLUGIN": "svp",
           "LANG": "C.UTF-8"}
    for name in ("LANG", "LC_ALL", "LC_CTYPE", "LC_NUMERIC", "LC_TIME", "LC_COLLATE",
                 "LC_MONETARY", "LC_MESSAGES", "LC_PAPER", "LANGUAGE"):
        value = env.get(name) or ""
        if _SAFE_LOCALE.match(value):
            out[name] = value
    return out


def inner_rules(install_root):
    """Landlock's rules inside the sandbox (fence.py), in the sandbox's own paths."""
    out = [("system", p) for p in ("/usr", "/etc", "/var", "/proc", ROOT + "/bin")]
    if install_root and not within(install_root, "/usr"):
        out.append(("system", install_root))
    out += [("devices", "/dev"), ("sockets", PIPES)]
    return out + [("private", p) for p in (PROFILE, WORK, TMP, HOME)]


def command(bwrap, private, install_root, launcher, hidden, inner):
    """The argv that starts `inner` (the launcher's argv, in the sandbox's paths) in the sandbox.
    `private` is a `Private`, `install_root` LibreOffice's install, `launcher` fence.py's host
    path, `hidden` the person's places to cover again wherever a shown folder holds one."""
    argv = [bwrap] + list(NAMESPACES) + ["--die-with-parent", "--new-session", "--cap-drop",
                                         "ALL", "--hostname", "yantrik-agent-office"]
    argv += ["--ro-bind", "/usr", "/usr"] + top_links()
    binds = [("/usr", "/usr")]
    for name in ETC:
        path = "/etc/" + name
        if os.path.lexists(path):
            argv = _shown(path, argv)
            binds.append((os.path.realpath(path), path))
    for path in VAR:
        if os.path.isdir(path):
            argv += ["--ro-bind", path, path]
            binds.append((path, path))
    if install_root and not within(install_root, "/usr"):
        argv += ["--ro-bind", install_root, install_root]
        binds.append((install_root, install_root))
    for target in linked_targets(install_root, hidden) if install_root else []:
        argv += ["--ro-bind", target, target]
        binds.append((target, target))
    argv += ["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/dev/shm"]
    for source, inside in private.shown():
        argv += ["--bind", source, inside]
    argv += ["--tmpfs", HOME, "--ro-bind", launcher, LAUNCHER]
    argv += masks(binds + private.shown(), hidden)
    argv += ["--remount-ro", "/", "--chdir", WORK, "--"]
    return argv + list(inner)
