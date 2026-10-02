"""Hold Hermes's desktop platform to the desktop's tools, every time the gateway starts.

hermes.sh sets the allowlist once, at install. Nothing after that kept it: `hermes update`, a
plugin installed by hand or by a skill, or an edit to config.yaml can widen what the `yantrik`
platform gets, and the first sign would be Hermes's own `terminal` running as the person past every
desktop approval. So the adapter asks at startup and again at every attach, and does not attach to a
desktop on a platform that has more than it should.

`hermes_config.py` is the install's own file, copied beside this one by hermes.sh, so the rule that
sets the allowlist and the rule that checks it are the same code. When it is not there the answer is
no, never "nothing to check".
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Optional

from . import desktop


def _hermes_config() -> Any:
    try:
        from . import hermes_config
    except ImportError:
        return None
    return hermes_config


def config_path() -> str:
    """The config.yaml Hermes reads, as it says, or where Hermes keeps it by default."""
    try:
        from hermes_cli.config import get_config_path

        return str(get_config_path())
    except Exception:
        home = os.environ.get("HERMES_HOME", "").strip() or str(Path.home() / ".hermes")
        return str(Path(home) / "config.yaml")


def _sentence(problems: "list[str]") -> str:
    return "; ".join(problems)


def start(path: Optional[str] = None, config: Any = None) -> Optional[str]:
    """At gateway start: say what the file had become, put the allowlist and the desktop's MCP
    server back, and answer why the desktop must not be attached to, or None when it may.

    A widened file is refused even though it has just been put right: the gateway that is starting
    may have read it before this ran, so it is a refusal for this run. Restarting the gateway reads
    the restored file, and attaches.
    """
    config = config or _hermes_config()
    if config is None:
        return "the check of Hermes's desktop tools is missing; install Hermes again from this desktop"
    path = path or config_path()
    found, left = config.reassert(path)
    if left:
        return "Hermes's desktop platform has more than the desktop's tools and could not be put right: " + _sentence(left)
    if found:
        return (
            "Hermes's desktop tools had been widened (" + _sentence(found) + "). They are restored; "
            "restart Hermes's gateway to attach: systemctl --user restart hermes-gateway"
        )
    return None


def check(path: Optional[str] = None, config: Any = None) -> Optional[str]:
    """Before an attach, when the file may have changed since the gateway started: the same
    question, with nothing changed, answered as why not or None."""
    config = config or _hermes_config()
    if config is None:
        return "the check of Hermes's desktop tools is missing; install Hermes again from this desktop"
    path = path or config_path()
    try:
        found = config.problems(path, config.hermes_resolver())
    except (config.ConfigError, OSError) as exc:
        found = [str(exc)]
    if found:
        return "Hermes's desktop tools have been widened (" + _sentence(found) + "); restart Hermes's gateway to put them back"
    return None


def decide(reason: Optional[str]) -> bool:
    """Whether to attach, saying so where the desktop's Hermes row shows it. Never logs a path's contents."""
    if reason:
        desktop.say_refused(reason)
        return False
    desktop.clear_refused()
    return True
