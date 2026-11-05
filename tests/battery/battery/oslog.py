"""The OS's own record of what happened to each approval card, read back from the shell's log.

The log lines are written by crates/yantrik-ui/src/control_approvals.rs:
  "a person denied one action"                       request=appr-N      -> denied
  "a person allowed one action, once" / "...for this session" /
  "allowed once: this card offers no session rule"   request=appr-N      -> ALLOWED (a hard failure here)
A card that nobody answered leaves no line (the caller's own result says "did not answer"), so it is `expired`.
The runner never believes that a card is gone means it was declined: only a `denied` line does."""
import os
import re

LOG = "/opt/yantrik/logs/yantrik-os.log"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
DENIED = re.compile(r"a person denied one action\s+request=(appr-\d+)")
ALLOWED = re.compile(r"(?:a person allowed one action(?:, once| for this session)?|allowed once: this card offers no session rule)\s+request=(appr-\d+)")
GRANTED = re.compile(r"granted=true")


class OsLog:
    def __init__(self, path=LOG):
        self.path = path

    def offset(self):
        try:
            return os.path.getsize(self.path)
        except OSError:
            return None

    def read_since(self, offset):
        """The text written after `offset`, or None when the log cannot be read."""
        if offset is None:
            return None
        try:
            with open(self.path, "rb") as f:
                f.seek(offset)
                return ANSI.sub("", f.read().decode("utf-8", "replace"))
        except OSError:
            return None


def outcomes(text, card_ids):
    """{card id: 'denied' | 'ALLOWED' | 'expired' | 'unknown'} plus the number of spent grants (granted=true acts)."""
    if text is None:
        return {c: "unknown" for c in card_ids}, None
    denied = set(DENIED.findall(text))
    allowed = set(ALLOWED.findall(text))
    res = {}
    for c in card_ids:
        res[c] = "ALLOWED" if c in allowed else "denied" if c in denied else "expired"
    return res, len(GRANTED.findall(text))
