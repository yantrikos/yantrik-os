"""Private mode: while the person has it on, no agent sees or does anything on this desktop.

The same rule as the transport's `yantrik_ipc_transport::privacy`, read from the same file: the
shell publishes `privacy.json` beside the settings file, and it stays on until the person turns it
off. Read on every call, so turning it on reaches connections already open:

- the mind door (`wire._DoorConnection`): every request from the mind account is refused;
- a surface's `act` (`surface.Surface.act`): a call carrying an agent token is refused.

Absent is off; a file that is there and cannot be read or understood is on.
"""

import json
import os

PRIVACY_FILE = "privacy.json"

# The transport's `privacy::REFUSAL`, word for word: an agent recognises it and stops.
REFUSAL = ("PRIVATE: the person has turned on Private mode. Nothing on this desktop is shown to "
           "agents or done for them until they turn it off. Nothing was run.")


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
