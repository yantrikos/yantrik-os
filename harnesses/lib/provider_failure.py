"""When the model call behind a turn fails, the sentence that says so (#1386).

Pi settles a failed model call into an assistant message with `stopReason: "error"` and an
`errorMessage`. pi-ai builds that from the SDK's error, so it comes in three forms:
`429 <text>`, `429: {json}` and `429 {json}`. The gateway is Pi's only provider and names the
account itself in `error.message`, so that sentence is used as it is, with no "who" prefix.
Nothing is made up: with no reason it says only the status. Request bodies and keys are never
read, so they cannot be echoed.
"""

import json
import re
from typing import Any, Dict, Optional

_STATUS = re.compile(r"^\s*(\d{3})\b:?\s*(.*)$", re.S)
_LONGEST = 400


def _reason(rest: str) -> str:
    """The reason in what follows the status: a JSON body's message, else the text itself."""
    rest = rest.strip()
    if not rest:
        return ""
    if rest.startswith("{"):
        try:
            body = json.loads(rest)
        except ValueError:
            return ""
        error = body.get("error") if isinstance(body, dict) else None
        if isinstance(error, dict):
            return str(error.get("message") or "").strip()
        if isinstance(error, str):
            return error.strip()
        return str(body.get("message") or "").strip() if isinstance(body, dict) else ""
    return rest


def provider_failure(message: Dict[str, Any]) -> Optional[str]:
    """The sentence for a failed assistant `message`, or None when the model call did not fail."""
    if message.get("stopReason") != "error":
        return None
    raw = str(message.get("errorMessage") or "")
    found = _STATUS.match(raw)
    status = found.group(1) if found else ""
    said = _reason(found.group(2) if found else raw)
    if not said:
        code = " (%s)" % status if status else ""
        return "The model's account refused this request%s and gave no reason." % code
    said = said[:_LONGEST]
    if not said.endswith((".", "!", "?")):
        said += "."
    return said
