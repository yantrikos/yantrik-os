#!/usr/bin/python3
"""Stop Mind View's browser, and only that: the update's step on both sides of a build change.

A build starts Mind View's browser on a DevTools pipe the shell hands to the browser service
(#1200); a build before that started it on a port. A browser left from the other side of an update
or a rollback would hold Mind View's profile, and a second launch hands over to it instead of
starting one the service holds: the mind would find no browser it can drive, and an old one's port
would stay open. So before the shell restarts, the main process of the desktop account's Mind View browser is told to stop. It
is the mind's, and it reopens it. The person's Browser is never matched.

This runs as root, so what it will signal is held to facts a planted file cannot make true:

  * only the desktop account's own profile is looked at (`--user`, which yantrik-update knows:
    its home's `.local/share/<Mind View's profile>`), never another account's home and never a scan
    of what processes hold open;
  * the profile directory and its `SingletonLock` link are owned by that account, the directory is
    not writable by others, nor by a group unless that is the account's own private group (its
    primary group, with no other member and no other account's primary group: a desktop with umask
    002 makes every directory 775), and its real path is itself (no link anywhere on the way);
  * the link names this machine and a pid that belongs to that account, that started no later than
    the link was written (a stale link naming a pid reused since fails this), and is not this
    process;
  * the process holds files of the profile open: an fd of it whose target is under the profile's
    real path. A real Mind View browser holds many (measured on 520); the person's Browser, or any
    process a planted `SingletonLock` names, holds none. This is checked again, on the pid the
    pidfd holds, just before the signal;
  * the signal goes through a pidfd opened and re-checked first, so the pid cannot change hands
    between the check and the kill.

(A Chromium's main process holds no `LOCK` file open, measured on Chrome 152, so the test is any
file of the profile, not a `LOCK` the way it is for "whose browser holds this port".)

    python3 stop_mind_view.py --user NAME           # run as root by yantrik-update
    python3 stop_mind_view.py --user NAME --list    # print what it would stop, stop nothing
"""

import os
import signal
import socket
import stat
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from yantrik_browser import ports  # noqa: E402

TICK_SLACK = 2.0      # seconds a lock may be older than the process that wrote it (clock ticks)


def _boot_time(proc):
    with open(os.path.join(proc, "stat")) as f:
        for line in f:
            if line.startswith("btime "):
                return float(line.split()[1])
    raise OSError("no btime")


def started_at(proc, pid, ticks=None):
    """When process `pid` started, in seconds since the epoch, or None."""
    try:
        with open(os.path.join(proc, str(pid), "stat"), encoding="utf-8", errors="replace") as f:
            start = int(f.read().rpartition(")")[2].split()[19])
        return _boot_time(proc) + start / (ticks or os.sysconf("SC_CLK_TCK"))
    except (OSError, ValueError, IndexError):
        return None


def private_group(uid, gid):
    """Whether `gid` is the account's own private group: its primary group, with no other member."""
    try:
        import grp
        import pwd
        entry = pwd.getpwuid(uid)
        return (entry.pw_gid == gid
                and all(m == entry.pw_name for m in grp.getgrgid(gid).gr_mem)
                and not any(e.pw_gid == gid and e.pw_uid != uid for e in pwd.getpwall()))
    except (ImportError, KeyError):
        return False


def profile_of(home):
    return os.path.join(home, ".local", "share", *ports.MIND_VIEW_PROFILE.split("/"))


def holds_profile(proc, pid, profile):
    """Whether process `pid` has a file of `profile` open: an fd whose target is under its real path."""
    real = os.path.realpath(profile) + os.sep
    fds = os.path.join(proc, str(pid), "fd")
    try:
        names = os.listdir(fds)
    except OSError:
        return False
    for name in names:
        try:
            if os.readlink(os.path.join(fds, name)).startswith(real):
                return True
        except OSError:
            continue
    return False


def mind_view_main(home, uid, proc="/proc", host=None, me=None, group_ok=private_group):
    """The pid of the desktop account's Mind View browser, or None when there is none that is
    certainly it. `home` and `uid` are that account's."""
    profile = profile_of(home)
    try:
        d = os.lstat(profile)
        link = os.lstat(os.path.join(profile, "SingletonLock"))
        target = os.readlink(os.path.join(profile, "SingletonLock"))
    except OSError:
        return None
    if (not stat.S_ISDIR(d.st_mode) or d.st_uid != uid or d.st_mode & 0o002
            or (d.st_mode & 0o020 and not group_ok(uid, d.st_gid))
            or not stat.S_ISLNK(link.st_mode) or link.st_uid != uid
            or os.path.realpath(profile) != os.path.join(os.path.realpath(home), profile[len(home):].lstrip(os.sep))):
        return None
    name, _, pid = target.rpartition("-")
    if not pid.isdigit() or name != (host or socket.gethostname()):
        return None
    pid = int(pid)
    if pid == (me or os.getpid()):
        return None
    try:
        if os.stat(os.path.join(proc, str(pid))).st_uid != uid:
            return None
    except OSError:
        return None
    began = started_at(proc, pid)
    if began is None or began > link.st_mtime + TICK_SLACK or not holds_profile(proc, pid, profile):
        return None
    return pid


def stop(pid, uid, profile, proc="/proc", sig=signal.SIGTERM):
    """Signal `pid` through a pidfd, after checking again that it is still that account's and still
    holds the profile's files."""
    try:
        fd = os.pidfd_open(pid)
    except (OSError, AttributeError):
        return False
    try:
        if os.stat(os.path.join(proc, str(pid))).st_uid != uid or not holds_profile(proc, pid, profile):
            return False
        signal.pidfd_send_signal(fd, sig)
        return True
    except OSError:
        return False
    finally:
        os.close(fd)


def main(argv):
    user = argv[argv.index("--user") + 1] if "--user" in argv and argv.index("--user") + 1 < len(argv) else None
    if not user:
        print("stop_mind_view.py: --user NAME (the desktop account) is required", file=sys.stderr)
        return 2
    try:
        import pwd
        entry = pwd.getpwnam(user)
    except (ImportError, KeyError):
        print("stop_mind_view.py: no account %s" % user, file=sys.stderr)
        return 2
    pid = mind_view_main(entry.pw_dir, entry.pw_uid)
    if pid is None:
        return 0
    if "--list" in argv:
        print("would stop %d" % pid)
    else:
        print("stopping %d" % pid if stop(pid, entry.pw_uid, profile_of(entry.pw_dir)) else "could not stop %d" % pid)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
