"""Which profile each of the desktop's browsers has, and the one old DevTools port (#1085).

There are two: the person's Browser (their own signed-in profile) and Mind View's browser (a
profile of its own, which every mind's web tools drive). The paths are in browsers.conf (the
yantrik-browser-ports crate owns it; a release copies it beside this file), and the Rust side
builds its constants from the same file. Nothing else spells them.

Neither browser has a port (#1137, #1200): each runs with `--remote-debugging-pipe`, held by
pipes.py. A DevTools port is reachable by every process of the machine, and a job that speaks raw
DevTools to it goes round the fence this service puts on a browser's tabs. `PERSON_PORT` is the
number the person's Browser used to listen on, kept so the guard can say an older build's browser
still does. Mind View's old port is spelled nowhere: nothing may fall back to it.
"""

import os
import tempfile

_HERE = os.path.dirname(os.path.abspath(__file__))
CONF = next((p for p in (os.path.join(_HERE, "browsers.conf"),
                         os.path.join(_HERE, "..", "..", "..", "crates", "yantrik-browser-ports", "browsers.conf"))
             if os.path.isfile(p)), os.path.join(_HERE, "browsers.conf"))


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


PERSON, MIND = "person", "mind"
