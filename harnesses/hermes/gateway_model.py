"""The model the desktop's gateway answers Hermes with.

Hermes' config says `model: picked`, which names nothing: the gateway resolves it at each call.
So the real model is asked of the gateway itself, as Hermes: `/v1/models` carries it under
`yantrik.picked` (`<account>/<model>`). Display only; nothing is sent but the token Hermes already
holds, and only to a loopback address.
"""

from __future__ import annotations

import ipaddress
import json
import urllib.parse
import urllib.request
from typing import Any, Dict, Optional

PICKED = "picked"
TIMEOUT_SECONDS = 2.0


def _is_loopback_http(url: str) -> bool:
    """The token is only ever sent to the machine's own gateway: plain http, loopback, no user info."""
    try:
        parts = urllib.parse.urlsplit(url)
        host = ipaddress.ip_address((parts.hostname or "").strip("[]"))
        return parts.scheme == "http" and parts.username is None and parts.password is None and host.is_loopback
    except ValueError:
        return False


def _model_part(picked: str) -> str:
    """The model's own name: an account slug is never shown to the person."""
    return picked.split("/", 1)[1] if "/" in picked else picked


def real_model(model_config: Dict[str, Any]) -> Optional[str]:
    """What `model: picked` resolves to now, as the gateway says, or None when it cannot be told
    (not on the gateway, nothing usable picked, the gateway not answering)."""
    base_url = str(model_config.get("base_url") or "").strip()
    token = str(model_config.get("api_key") or "").strip()
    if not base_url or not token or not _is_loopback_http(base_url):
        return None
    request = urllib.request.Request(base_url.rstrip("/") + "/models", headers={"Authorization": "Bearer " + token})
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as reply:
            listing = json.loads(reply.read(1024 * 1024) or b"{}")
        entry = next(m for m in listing["data"] if m.get("id") == PICKED)
        picked = entry["yantrik"]["picked"]
    except (OSError, ValueError, KeyError, TypeError, StopIteration):
        return None
    return _model_part(picked) if isinstance(picked, str) and picked else None
