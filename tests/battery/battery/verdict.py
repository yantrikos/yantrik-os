"""Verdicts and the reason patterns that turn a "not done" into NAMED."""
import re
from dataclasses import dataclass, field

PASS, FAIL, NAMED, UNSURE = "PASS", "FAIL", "NAMED", "UNSURE"

NAMED_PATTERNS = [
    (r"\b(approval|approve|allow(?:ed)?|needs? your (?:ok|ok\.|permission|go-ahead)|asked you|waiting for you)\b", "needs the person's approval"),
    (r"\b(declin(?:ed|e)|said no|did not answer|didn.t answer|timed out)\b", "approval declined or timed out"),
    (r"\b(no (?:e-?mail |mail )?account|not (?:set up|configured|signed in)|no mailbox)\b", "no account set up"),
    (r"\b(refused|not allowed|won.t open|guard|private state)\b", "refused by a guard"),
    (r"\b(can.?t|cannot|unable to|couldn.?t|could not|don.t have a way|no tool)\b", "says it cannot"),
    (r"\b(which|what) (?:thursday|day|rows|do you mean|should)\b|\bdo you want\b|\bwhich one\b", "asked a clarifying question"),
]


@dataclass
class Verdict:
    status: str
    reason: str
    evidence: dict = field(default_factory=dict)


def named_reason(reply, cards=None, asked=False):
    """A named reason for a not-done outcome, from the reply's own words or a card that appeared; else None."""
    if cards:
        return f"approval card for {cards[0].get('app')}.{cards[0].get('action')}"
    if asked:
        return "asked a clarifying question"
    for pat, why in NAMED_PATTERNS:
        if re.search(pat, reply or "", re.I):
            return why
    return None


def or_named(ok_status, reply, cards, asked, reason_if_fail, evidence):
    """PASS passes; otherwise NAMED when the reply or a card names why, else FAIL."""
    if ok_status:
        return Verdict(PASS, "outcome verified in the app", evidence)
    why = named_reason(reply, cards, asked)
    if why:
        return Verdict(NAMED, why, {**evidence, "not_done": reason_if_fail})
    return Verdict(FAIL, reason_if_fail, evidence)
