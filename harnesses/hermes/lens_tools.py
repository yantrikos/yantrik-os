"""The desktop's tools as Hermes tools that choose their bridge per call (#1070, item 4).

One proxy for each tool `yos-mcp` offers, in the plugin toolset `yantrik_desk`. A call from a
Lens turn goes to the Lens bridge (`lens_bridge.LensBridge`, which carries the conversation's
token); every other call goes to the gateway's shared bridge, the `yantrik_os` MCP server Hermes
itself runs, through Hermes's own tool registry. Which is decided by `lens_route.route_here` and
fails closed to the shared one.

Turned on by `install`, as the gateway starts, once the proxies are registered: it asks
`hermes_config.py lens on` to put `yantrik_desk` and Hermes's `no_mcp` sentinel in the platform's
list. `no_mcp` drops MCP servers for this platform only, so the model is offered the proxies and
not the shared bridge's own copies (which would skip the routing); Telegram, cron and the CLI keep
theirs, and `yantrik_os` stays in `mcp_servers`, so the proxies' shared path still works. If the
proxies could not be registered, `lens off` puts the plain list back.
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
# The shared bridge's tools are registered by Hermes as mcp__<server>__<tool> (at 38d3dbc:
# tools/mcp_tool_schema.py `mcp_prefixed_tool_name`, which also clamps a long name with a hash).
SHARED_SERVER = "yantrik_os"


def shared_name(tool: str) -> str:
    try:
        from tools.mcp_tool_schema import mcp_prefixed_tool_name

        return mcp_prefixed_tool_name(SHARED_SERVER, tool)  # Hermes's own spelling, whatever it is
    except Exception:
        clean = lambda part: re.sub(r"[^A-Za-z0-9_]", "_", part)
        return "mcp__%s__%s" % (clean(SHARED_SERVER), clean(tool))


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


def set_platform_list(on: bool) -> bool:
    """Ask `hermes_config.py` (one copy of that logic, in the image) to turn the platform's Lens
    list on or off, with Hermes's own Python. True when it is now as asked."""
    import os
    import subprocess
    import sys

    script = os.path.join(
        os.environ.get("YANTRIK_HARNESS_LIB") or "/opt/yantrik/share/harnesses/lib", "install", "hermes_config.py")
    if not os.path.isfile(script):
        return False
    try:
        done = subprocess.run(
            [sys.executable, script, "lens", "on" if on else "off", "-"],
            stdin=subprocess.DEVNULL, capture_output=True, timeout=60)
    except (OSError, subprocess.SubprocessError):
        return False
    return done.returncode in (0, 10)  # 10: it changed the file


# What this process registered. The platform's list is changed from the gateway's `connect`, never
# from plugin load: any `hermes` command loads plugins, and one run while an older gateway is still
# up (after an update, before its restart) would otherwise name tools that gateway does not have.
REGISTERED: List[str] = []


def install(ctx: Any, standing_of: Callable[[], Optional[lens_route.Standing]], bridge: LensBridge) -> List[str]:
    """Register the proxies from the bridge's own tool list. Nothing is registered when the bridge
    cannot be started (the desktop's tools are not installed)."""
    REGISTERED[:] = []
    try:
        from .lens_bridge import tool_list

        REGISTERED[:] = register_proxies(ctx, tool_list(), bridge, standing_of)
    except Exception as exc:
        logger.info("[yantrik] the Lens bridge's proxy tools were not registered: %s", exc.__class__.__name__)
    return list(REGISTERED)


def offer_to_platform(set_list: Optional[Callable[[bool], bool]] = None) -> bool:
    """As the gateway starts, with the proxies registered in THIS process: give the platform the
    proxies and drop its MCP tools, or, when there are none, put the plain list back."""
    set_list = set_list or set_platform_list
    try:
        done = bool(set_list(bool(REGISTERED)))
    except Exception:
        logger.info("[yantrik] the platform's tool list could not be changed", exc_info=True)
        return False
    if not done:
        logger.info("[yantrik] the platform's tool list was not changed to match the Lens bridge")
    return done
