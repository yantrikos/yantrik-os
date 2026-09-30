"""The mind door (#411): where minds, running as their own account, reach a surface.

A port of `yantrik_ipc_transport::mind_door` — the same directory, the same rules. If the two
ever disagree, the Rust file is right and this one is wrong.

Minds run as `yantrik-mind`. They cannot open the person's runtime directory (0700), so each app
surface also listens in a directory the person owns and the minds' group may only enter
(`/run/yantrik-minds`, 2750 person:yantrik-minds, sockets 0660). On that socket the kernel says who
is calling (`SO_PEERCRED`): a connection is served only when its uid is the mind account's. The
person's own socket never admits that uid, so the uid alone answers "is this a mind".

When the directory is missing, or not exactly as described, there is no door: the person's socket
works as before and nothing is served to anybody else.

The Rust apps have opened this door since #411; a Python surface (Blender) did not, so a mind that
opened Blender could never describe it or act on it.
"""

import grp
import os
import pwd
import stat

MIND_USER = "yantrik-mind"
MIND_GROUP = "yantrik-minds"
DEFAULT_DIR = "/run/yantrik-minds"
# A bounded number of mind connections at once, as `DOOR_CONNECTIONS`: a mind that opens
# connections and holds them uses up its own share and nothing of the person's.
DOOR_CONNECTIONS = 32

_ids = {}


def door_dir():
    """The door directory's path, as named (`YANTRIK_MIND_RUN`, or the default). Not a promise
    that it exists."""
    named = os.environ.get("YANTRIK_MIND_RUN", "")
    return named if named else DEFAULT_DIR


def _lookup(kind):
    if kind not in _ids:
        try:
            _ids[kind] = (pwd.getpwnam(MIND_USER).pw_uid if kind == "user"
                          else grp.getgrnam(MIND_GROUP).gr_gid)
        except KeyError:
            _ids[kind] = None
    return _ids[kind]


def mind_uid():
    """The mind account's uid, if this machine has one."""
    return _lookup("user")


def mind_gid():
    """The minds' group, if this machine has one."""
    return _lookup("group")


def is_mind(uid):
    """Whether a caller with this uid is a mind. Never root, never this process's own uid: an
    account table that named either as the mind account would make the person (or root) a mind."""
    return isinstance(uid, int) and uid != 0 and uid != os.getuid() and mind_uid() == uid


def acceptable(owner, group, mode, me, minds):
    """What a door directory must be for this process to serve on it: owned by this process's uid,
    group the minds' group, mode exactly 2750."""
    return owner == me and group == minds and mode & 0o7777 == 0o2750


def serving_dir():
    """The door directory, when this process should serve on it; otherwise None."""
    if mind_uid() is None:
        return None
    minds = mind_gid()
    if minds is None:
        return None
    directory = door_dir()
    try:
        st = os.stat(directory)
    except OSError:
        return None
    if not stat.S_ISDIR(st.st_mode) or not acceptable(st.st_uid, st.st_gid, st.st_mode,
                                                      os.getuid(), minds):
        return None
    return directory


def opens_a_door(socket_name):
    """Which sockets open a door: the harness and the app surfaces, and nothing else by name."""
    if not socket_name.endswith(".sock"):
        return False
    service = socket_name[:-len(".sock")]
    return service == "harness" or service.startswith("app-")


def door_for(address, socket_dir, door):
    """Where the door socket for the service listening at `address` goes: the same file name in
    the door directory, for an app surface's socket in this session's own socket directory.
    Anything else (a test's temp path, an explicit address) gets no door."""
    name = os.path.basename(address)
    if os.path.dirname(address) != socket_dir or not opens_a_door(name):
        return None
    return os.path.join(door, name)
