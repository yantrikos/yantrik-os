"""The desktop's tools as Hermes tools that choose their bridge per call (#1070, item 4).

One proxy for each tool `yos-mcp` offers, in the plugin toolset `yantrik_desk`. A call from a
Lens turn goes to the Lens bridge (`lens_bridge.LensBridge`, which carries the conversation's
token); every other call goes to the gateway's shared bridge, the `yantrik_os` MCP server Hermes
itself runs, through Hermes's own tool registry. Which is decided by `lens_route.route_here` and
fails closed to the shared one.

INERT until the platform lists the toolset. Hermes adds a plugin toolset to a platform only when
the platform's `platform_toolsets` names it, and `hermes_config.py` does not yet. Doing that
means the model must stop being offered the shared bridge's own copies of these tools on this
platform, or it can call them and skip the routing; see the PR for #1070.
"""

from __future__ import annotations

import json
import logging
import re
from typing import Any, Callable, Dict, List, Optional

from . import lens_route
from .lens_bridge import LensBridge, Unavailable

logger = logging.getLogger(__name__)

TOOLSET = "yantrik_desk"
# The shared bridge's tools are registered by Hermes as mcp_<server>_<tool>.
SHARED_SERVER = "yantrik_os"


def shared_name(tool: str) -> str:
    return "mcp_%s_%s" % (re.sub(r"[^A-Za-z0-9_]", "_", SHARED_SERVER), re.sub(r"[^A-Za-z0-9_]", "_", tool))


def _dispatch_shared(tool: str, args: Dict[str, Any]) -> str:
    """The call as the gateway's tokenless bridge answers it, through Hermes's registry."""
    try:
        from tools.registry import registry

        return registry.dispatch(shared_name(tool), args)
    except Exception:
        return json.dumps({"error": "the desktop's tools are not reachable from here"})


def _result(text: str, is_error: bool) -> str:
    return json.dumps({"error": text} if is_error else {"result": text}, ensure_ascii=False)


def make_handler(
    tool: str,
    bridge: LensBridge,
    standing_of: Callable[[], Optional[lens_route.Standing]],
    shared: Callable[[str, Dict[str, Any]], str] = _dispatch_shared,
) -> Callable[..., str]:
    """The handler for one tool. Runs in a Hermes worker thread, whose context is the calling
    turn's: that is what `route_here` reads."""

    def handler(args: Optional[Dict[str, Any]] = None, **_ignored: Any) -> str:
        args = dict(args or {})
        if lens_route.route_here(standing_of) != lens_route.LENS:
            return shared(tool, args)
        try:
            text, is_error = bridge.call(tool, args)
        except Unavailable as exc:
            # Fail closed, and say so: not run again on the other bridge, where it might happen
            # twice or under no conversation's name.
            return json.dumps({"error": "%s. Nothing was repeated elsewhere; ask again." % exc})
        return _result(text, is_error)

    handler.__name__ = "yantrik_desk_" + re.sub(r"\W", "_", tool)
    return handler


def schema_of(tool: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "name": tool["name"],
        "description": str(tool.get("description") or ""),
        "parameters": tool.get("inputSchema") or {"type": "object", "properties": {}},
    }


def register_proxies(
    ctx: Any,
    tools: List[Dict[str, Any]],
    bridge: LensBridge,
    standing_of: Callable[[], Optional[lens_route.Standing]],
) -> List[str]:
    """Register one proxy per tool. Returns the names registered."""
    names = []
    for tool in tools:
        name = tool.get("name")
        if not isinstance(name, str) or not name:
            continue
        ctx.register_tool(
            name=name,
            toolset=TOOLSET,
            schema=schema_of(tool),
            handler=make_handler(name, bridge, standing_of),
            description=str(tool.get("description") or "")[:200],
            emoji="🖥️",
        )
        names.append(name)
    return names


def install(ctx: Any, standing_of: Callable[[], Optional[lens_route.Standing]], bridge: LensBridge) -> List[str]:
    """Register the proxies from the bridge's own tool list. Nothing is registered when the bridge
    cannot be started (the desktop's tools are not installed): the platform then has only the
    shared bridge, as before."""
    try:
        from .lens_bridge import tool_list

        return register_proxies(ctx, tool_list(), bridge, standing_of)
    except Exception as exc:
        logger.info("[yantrik] the Lens bridge's proxy tools were not registered: %s", exc.__class__.__name__)
        return []
