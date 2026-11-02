"""Which bridge a tool call goes to: the Lens conversation's, or the gateway's (#1070, item 4).

The desktop follows the person's + only for a bridge that carries the conversation's own token and
serves that conversation alone (`control_taint_scope.rs`). Hermes runs one tokenless bridge for
the whole gateway, which also serves cron jobs, `/bg` tasks and other platforms, and nothing in a
call says which of them it came from. The plugin therefore runs a second bridge, with the Lens's
token, and sends a call to it only when it can PROVE the call is from a Lens turn. Anything it
cannot prove goes to the tokenless bridge, which keeps its taint for as long as it runs: fail
closed.

The proof is three facts that Hermes sets per message or per agent in ContextVars, copied into the
tool's worker thread (gateway/session_context.py), not `os.environ`, which is process-global and
racy. Read at Hermes 38d3dbc (HERMES_COMMIT):

- the session key is exactly the Lens's (the key the adapter itself computed for its chat),
- the triggering message is one of the Lens turns the adapter has open now, and
- the agent's session id (`HERMES_SESSION_ID`, set when the agent is built) is the id Hermes's
  session store holds for the Lens's key right now.

A `/bg` task (gateway/run_turn.py `_run_background_task_inner`) is started under `copy_context`
and never rebinds the session vars, so it inherits the Lens key AND the message id of the turn that
started it, and passes the first two checks for as long as that turn is open. It builds its agent
with `session_id=task_id` (`bg_...`), which is not the Lens session's id: the third check is what
tells it apart. A delegated child has a session of its own too, and so is the shared bridge's.

Pure: the caller hands in what it read, so each case is a test.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, FrozenSet, Optional

LENS = "lens"
SHARED = "shared"


@dataclass(frozen=True)
class Standing:
    """What the adapter can vouch for at the moment of a call."""

    lens_key: str
    # The session id Hermes's store holds for that key now; "" when it cannot be read.
    session_id: str
    # Message ids of the Lens turns that are open and being worked on, and of the runs they carry.
    open_ids: FrozenSet[str]
    # Whether the Lens bridge holds the token the desktop gave for this conversation right now.
    token_current: bool


def route(key: object, message_id: object, session_id: object, standing: Optional[Standing]) -> str:
    """`LENS` only when every part of the proof holds; `SHARED` for anything else, an unset or odd
    value included. The three are the ContextVars' values as read."""
    if standing is None or not standing.token_current:
        return SHARED
    if not isinstance(key, str) or not isinstance(message_id, str) or not isinstance(session_id, str):
        return SHARED  # the "never set" sentinel is not a string: cron, the CLI, a test
    if not key or not standing.lens_key or key != standing.lens_key:
        return SHARED
    if not message_id or message_id not in standing.open_ids:
        return SHARED
    if not session_id or not standing.session_id or session_id != standing.session_id:
        return SHARED
    return LENS


def context_values() -> tuple:
    """The session key, message id and agent session id of the calling context, as Hermes set
    them. Anything that cannot be read comes back as `None`, which routes to the shared bridge."""
    try:
        from gateway import session_context as sc

        return sc._SESSION_KEY.get(), sc._SESSION_MESSAGE_ID.get(), sc._SESSION_ID.get()
    except Exception:
        return None, None, None


def route_here(standing_of: Callable[[], Optional[Standing]]) -> str:
    """`route` for the calling context."""
    try:
        standing = standing_of()
    except Exception:
        return SHARED
    key, message_id, session_id = context_values()
    return route(key, message_id, session_id, standing)
