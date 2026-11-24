"""When the model call behind a turn fails, the sentence that says so (#1386).

Pi settles a failed model call into an assistant message with `stopReason: "error"` and an
`errorMessage` that is the provider's status followed by its body: `429 {"error": {...}}`. The
gateway answers every refusal with its own sentence in `error.message`; that sentence, as it is,
is all the person is told. Nothing is made up: a body that is not the usual JSON says only the
status. Request bodies and keys are never read, so they cannot be echoed.
"""

import json
import re
from typing import Any, Dict, Optional

_STATUS = re.compile(r"^\s*(\d{3})\b")
_LONGEST = 400


def provider_failure(message: Dict[str, Any]) -> Optional[str]:
    """The sentence for a failed assistant `message`, or None when the model call did not fail."""
    if message.get("stopReason") != "error":
        return None
    raw = str(message.get("errorMessage") or "")
    status = _STATUS.match(raw)
    said = ""
    start = raw.find("{")
    if start >= 0:
        try:
            body = json.loads(raw[start:])
        except ValueError:
            body = None
        error = body.get("error") if isinstance(body, dict) else None
        if isinstance(error, dict):
            said = str(error.get("message") or "").strip()
        elif isinstance(error, str):
            said = error.strip()
        elif isinstance(body, dict):
            said = str(body.get("message") or "").strip()
    who = str(message.get("provider") or "").strip() or "The model's provider"
    code = " (%s)" % status.group(1) if status else ""
    if not said:
        return "%s refused this request%s and gave no reason." % (who, code)
    said = said[:_LONGEST]
    if not said.endswith((".", "!", "?")):
        said += "."
    return "%s refused this request%s: %s" % (who, code, said)
