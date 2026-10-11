"""Put something on the Yantrik OS desktop where a mind can find it, read it and act in it.

    from yantrik_surface import Surface

    items = []
    s = Surface("hello", summary=lambda: "%d items" % len(items))

    @s.view
    def state():
        return {"items": items}

    @s.action("add", grade="standard")
    def add(text: str, count: int = 1) -> dict:
        "Add an item to the list, `count` times."
        items.extend([text] * count)
        return {"added": count}

    s.serve()   # binds app-hello.sock in the session's socket directory

`yos describe hello` then reads it and `yos act hello add text=milk` acts in it, under the
same grades, ceiling, mode and grants as every app this OS ships — because this package is a
port of the Rust runtime's dispatch, not an opinion of its own. Standard library only;
Python 3.11 or newer.

The pieces: `surface` (the dispatch: actions, parameters, envelopes, refusals), `gate` (the
ceiling, the mode, the grant), `wire` (the socket, the framing, the revision hash).
"""

from .gate import (
    DEFAULT_CEILING,
    DEFAULT_MODE,
    LADDER,
    MODES,
    OPEN_ENDED,
    SOCKET_FLOOR,
    Authority,
    GrantRefused,
    Mode,
    decide,
    grant_refusal,
    mode_from,
    open_ended,
    unrecoverable,
)
from .surface import (
    PROTOCOL,
    Action,
    Later,
    NotAnswered,
    Param,
    Refusal,
    Surface,
    agent_token,
    caller,
)
from .stopping import StopOnSignal
from .wire import PeerCred, PeerRefused, RpcError, SocketBusy, call_once, revision, socket_dir

__version__ = "0.1.0"

__all__ = [
    "Action",
    "Authority",
    "DEFAULT_CEILING",
    "DEFAULT_MODE",
    "GrantRefused",
    "LADDER",
    "Later",
    "MODES",
    "Mode",
    "NotAnswered",
    "OPEN_ENDED",
    "PROTOCOL",
    "Param",
    "PeerCred",
    "PeerRefused",
    "Refusal",
    "RpcError",
    "SOCKET_FLOOR",
    "SocketBusy",
    "StopOnSignal",
    "Surface",
    "agent_token",
    "call_once",
    "caller",
    "decide",
    "grant_refusal",
    "mode_from",
    "open_ended",
    "revision",
    "socket_dir",
    "unrecoverable",
]
