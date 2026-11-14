"""The person's places an agent's LibreOffice must never reach, worked out before its sandbox is
built (sandbox.py) and proved shut from inside it (fence.py).

The protected places are `home_paths::PROTECTED` (crates/yantrik-ipc-contracts), the one list the
shell's rule applies. The sandbox cannot ask the shell about a whole tree, so the list is read here
as written there; tests/test_fence.py fails the moment the two differ. Like the Rust rule it is
followed where `$XDG_DATA_HOME`, `$XDG_CONFIG_HOME` and `$XDG_STATE_HOME` are moved, and taken both
as written and where links resolve.

The sandbox shows none of the home at all, so these lists are not what keeps a place out: they are
what is hidden again if a folder the sandbox does show ever holds one (a home inside a system
folder, say), what the launcher's proof must fail to open, and which of the person's sockets it must
fail to reach.
"""

import os
import stat

PROTECTED = (
    ".ssh", ".gnupg", ".config/labwc", ".config/yantrik", "memory.db", ".bashrc", ".profile",
    ".bash_history", ".bash_profile", ".bash_login", ".bash_logout", ".zshrc", ".zshenv",
    ".zprofile", ".zlogin", ".pam_environment", ".config/autostart", ".config/environment.d",
    ".config/systemd", ".local/share/applications", ".config/mimeapps.list",
    ".aws", ".azure", ".config/gcloud", ".config/gh", ".docker/config.json", ".kube",
    ".git-credentials", ".netrc", ".npmrc", ".pypirc", ".cargo/credentials",
    ".cargo/credentials.toml", ".vault-token", ".terraform.d/credentials.tfrc.json",
    ".password-store", ".local/share/keyrings",
    ".claude", ".claude.json", ".hermes", ".openclaw", ".pi",
    "pictures/screenshots", "videos/recordings",
    ".local/share/yantrik", ".local/state/yantrik",
    ".local/share/yantrik-mind", ".config/yantrik-mind", ".config/yantrik-mind.env",
    ".mozilla", ".config/chromium", ".config/google-chrome", ".config/bravesoftware",
    ".var/app/com.google.chrome", ".var/app/org.chromium.chromium", ".var/app/com.brave.browser",
    "snap/chromium", "snap/firefox", ".thunderbird", ".config/evolution",
)

# The most of the person's sockets the launcher is asked to fail to reach; more are not looked for.
MAX_SOCKETS = 64


def parts(path):
    return [p.lower() for p in os.path.normpath(path).split(os.sep) if p]


def within(path, base):
    """Whether `path` is `base` or below it, a whole component at a time, case folded."""
    path, base = parts(path), parts(base)
    return len(path) >= len(base) and path[:len(base)] == base


def _absolute_env(env, name):
    value = (env.get(name) or "").strip()
    return value if os.path.isabs(value) else None


def protected_places(home, env=None):
    """Every protected place for this home, as written and where it resolves."""
    env = os.environ if env is None else env
    found = [os.path.join(home, place) for place in PROTECTED]
    for name, prefix in (("XDG_DATA_HOME", ".local/share/"), ("XDG_CONFIG_HOME", ".config/"),
                         ("XDG_STATE_HOME", ".local/state/")):
        base = _absolute_env(env, name)
        if base:
            found += [os.path.join(base, place[len(prefix):]) for place in PROTECTED
                      if place.startswith(prefix)]
    return sorted(set(found + [os.path.realpath(p) for p in found]))


def runtime_dirs(env=None, uid=None):
    """The person's runtime folders: `$XDG_RUNTIME_DIR` and `/run/user/<uid>`, where the session
    bus, the display, the sound server and every Yantrik app socket live."""
    env = os.environ if env is None else env
    uid = os.getuid() if uid is None else uid
    found = ["/run/user/%d" % uid]
    runtime = _absolute_env(env, "XDG_RUNTIME_DIR")
    if runtime:
        found.append(runtime)
    return sorted(set(found + [os.path.realpath(p) for p in found]))


def hidden(home, env=None, uid=None):
    """What the sandbox hides again wherever a folder it shows would hold it: the home itself, each
    protected place, the runtime folders, and the display's sockets."""
    return sorted(set([os.path.normpath(home), os.path.realpath(home)]
                      + protected_places(home, env) + runtime_dirs(env, uid)
                      + ["/tmp/.X11-unix", "/tmp/.ICE-unix"]))


def probes(home, env=None):
    """What the launcher must fail to open once the sandbox is up: the home itself, and every
    protected place that is there."""
    return [home] + [p for p in protected_places(home, env) if os.path.lexists(p)]


def persons_sockets(env=None, uid=None):
    """The person's sockets the launcher must fail to connect to: everything in the runtime folders
    and one folder down (the session bus, the display, `yantrik/app-*.sock`), the display's sockets
    in /tmp, every LibreOffice pipe in /tmp (the person's own among them) and the system bus."""
    uid = os.getuid() if uid is None else uid
    folders = runtime_dirs(env, uid) + ["/tmp/.X11-unix"]
    found = []
    for folder in folders:
        for entry in _entries(folder):
            if _is_socket(entry.path):
                found.append(entry.path)
            elif entry.is_dir(follow_symlinks=False):
                found += [e.path for e in _entries(entry.path) if _is_socket(e.path)]
    found += [e.path for e in _entries("/tmp")
              if e.name.startswith("OSL_PIPE_%d_" % uid) and _is_socket(e.path)]
    if _is_socket("/run/dbus/system_bus_socket"):
        found.append("/run/dbus/system_bus_socket")
    return sorted(set(found))[:MAX_SOCKETS]


def _entries(folder):
    try:
        return sorted(os.scandir(folder), key=lambda e: e.name)
    except OSError:
        return []


def _is_socket(path):
    try:
        return stat.S_ISSOCK(os.lstat(path).st_mode)
    except OSError:
        return False


class Unsuitable(Exception):
    """LibreOffice is installed somewhere the sandbox will not show."""


def install_root(soffice, home):
    """The folder LibreOffice is installed in: the `program` folder's parent, from the real path
    of `soffice`. Raises `Unsuitable` for an install in the person's home or a runtime folder,
    since showing it would show the person's own files, and for one that is a top folder itself."""
    real = os.path.realpath(soffice)
    root = os.path.dirname(os.path.dirname(real))
    if within(root, "/usr") and root != "/usr":
        return root
    for place in [home, os.path.realpath(home)] + runtime_dirs():
        if within(root, place) or within(place, root):
            raise Unsuitable("LibreOffice is installed in %s, inside the person's own files, "
                             "which an agent's LibreOffice is never shown" % root)
    if len(parts(root)) < 2:
        raise Unsuitable("LibreOffice is installed straight in %s, which an agent's LibreOffice "
                         "is not shown whole" % root)
    return root
