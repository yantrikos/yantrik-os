"""Which DevTools port and profile each of the desktop's browsers has (#1085).

There are two: the person's Browser (their own signed-in profile) and Mind View's browser (a
profile of its own, which every mind's web tools drive). The numbers and paths are in
browsers.conf, beside this file, and the Rust side (the yantrik-browser-ports crate) builds its
constants from the same file. Nothing else spells them.
"""

import os
import tempfile

CONF = os.path.join(os.path.dirname(os.path.abspath(__file__)), "browsers.conf")


def read_conf(path=CONF):
    """The file's `key = value` lines."""
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                out[k.strip()] = v.strip()
    return out


_CONF = read_conf()
PERSON_PORT = int(_CONF["person_port"])
MIND_VIEW_PORT = int(_CONF["mind_view_port"])
IDENTITY_PORTS = (int(_CONF["identity_port_first"]), int(_CONF["identity_port_last"]))
MIND_VIEW_PROFILE = _CONF["mind_view_profile"]
IDENTITY_PROFILES = _CONF["identity_profiles"]


def _absolute(environ, var):
    value = environ.get(var)
    return value if value and os.path.isabs(value) else None


def data_home(environ=None):
    """$XDG_DATA_HOME, else ~/.local/share; the temp directory for a process with neither (the
    Rust side's `data_home`)."""
    env = os.environ if environ is None else environ
    if _absolute(env, "XDG_DATA_HOME"):
        return env["XDG_DATA_HOME"]
    if _absolute(env, "HOME"):
        return os.path.join(env["HOME"], ".local", "share")
    return tempfile.gettempdir()


def mind_view_profile(environ=None):
    return os.path.join(data_home(environ), *MIND_VIEW_PROFILE.split("/"))


def identities_root(environ=None):
    return os.path.join(data_home(environ), *IDENTITY_PROFILES.split("/"))


def address(port):
    return "http://127.0.0.1:%d" % port


PERSON, MIND = "person", "mind"


def port_of(side):
    return PERSON_PORT if side == PERSON else MIND_VIEW_PORT
