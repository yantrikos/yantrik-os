"""Private mode: while the person has it on, no agent sees or does anything on this desktop.

The same rule as the transport's `yantrik_ipc_transport::privacy`, read from the same file: the
shell publishes `privacy.json` beside the settings file, and it stays on until the person turns it
off. Read on every call, so turning it on reaches connections already open:

- the mind door (`wire._DoorConnection`): every request from the mind account is refused;
- a surface's `act` (`surface.Surface.act`): a call carrying an agent token is refused;
- a surface's `describe`, and the view an `act` replies with, on the person's own socket: the
  private view (#924) — `"private": true`, `PRIVATE_SUMMARY`, and only the keys the app listed
  with `Surface.show_in_private`. The person's socket cannot tell an agent running as the person
  from the person, so nobody reads what the app holds while Private mode is on.

Absent is off; a file that is there and cannot be read or understood is on.
"""

import json
import os

PRIVACY_FILE = "privacy.json"

# The transport's `privacy::REFUSAL`, word for word: an agent recognises it and stops.
REFUSAL = ("PRIVATE: the person has turned on Private mode. Nothing on this desktop is shown to "
           "agents or done for them until they turn it off. Nothing was run.")


# The crate's `yantrik_surface::PRIVATE_SUMMARY` and `RESERVED_KEYS`, word for word.
PRIVATE_SUMMARY = "Private mode is on; this app's contents are hidden."
RESERVED_KEYS = ("private", "open", "windows_open")

# The crate's `WITHHELD` and `WITHHELD_REFUSAL` (#926 review, M1): what a caller the kernel does
# not vouch for reads, in Private mode, in place of an act's result and a handler's refusal.
WITHHELD = "Private mode is on; what this action answered is not shown to this caller."
WITHHELD_REFUSAL = ("PRIVATE: the app refused this call, and while the person has Private mode on "
                    "its reason is not shown to this caller.")

# `yantrik_ipc_transport::private_owner`'s, quoted from it: the shell, the called app's own program,
# and these read an act's own answer while the person is private. Why each is there is said there.
SHELL_BINARY = "yantrik-ui"
PRIVATE_READERS = ("yantrik-term",)
INSTALL_DIR = "/opt/yantrik/bin"
_DELETED = " (deleted)"


def is_private_reader_in(exe, own, directory):
    """Whether `exe` (a caller's /proc/<pid>/exe) may read an act's own answer while the person is
    private: the shell, a listed reader, or `own` (this process's program), directly in `directory`."""
    def strip(e):
        return e[:-len(_DELETED)] if e.endswith(_DELETED) else e
    exe = strip(exe or "")
    if not os.path.isabs(exe) or os.path.dirname(exe) != directory:
        return False
    name = os.path.basename(exe)
    return name == SHELL_BINARY or name in PRIVATE_READERS or (own is not None and strip(own) == exe)


def _exe_of(pid):
    try:
        return os.readlink("/proc/%d/exe" % pid)
    except (OSError, TypeError, ValueError):
        return None


def reads_while_private(pid):
    """Whether the process `pid`, as the kernel named the caller, may read an act's own answer
    while the person is private. Unidentifiable is no."""
    exe = _exe_of(pid) if isinstance(pid, int) and pid > 0 else None
    if exe is None:
        return False
    return is_private_reader_in(exe, _exe_of(os.getpid()), INSTALL_DIR)


def privacy_path():
    """Where the shell publishes Private mode, or None when this process cannot tell whose home it
    is: `HOME` when absolute, else the account's home from the password database, as the
    transport finds it. (`gate` is imported here, not at the top: `wire` imports this module, and
    `gate` imports `wire`.)"""
    from . import gate
    settings = gate.settings_path()
    if os.path.isabs(settings):
        return os.path.join(os.path.dirname(settings), PRIVACY_FILE)
    try:
        import pwd
        home = pwd.getpwuid(os.getuid()).pw_dir
    except (ImportError, KeyError, OSError):
        return None
    return os.path.join(home, ".config", "yantrik", PRIVACY_FILE) if os.path.isabs(home) else None


def private_in(text):
    """The file's meaning: only `{"private": false}` is not private."""
    try:
        value = json.loads(text)
    except ValueError:
        return True
    if not isinstance(value, dict):
        return True
    flag = value.get("private", True)
    return flag if isinstance(flag, bool) else True


def is_private():
    """Whether the person is in Private mode now. A process that cannot find the file takes the
    person to be private."""
    path = privacy_path()
    if path is None:
        return True
    try:
        with open(path, encoding="utf-8") as f:
            return private_in(f.read())
    except FileNotFoundError:
        return False
    except (OSError, UnicodeDecodeError):
        return True
