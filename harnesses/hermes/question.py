"""A question of Hermes's shown as a card on the desktop: its choices both ways (adapter.py)."""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any, List, Optional, Tuple

# How Hermes marks the choice it recommends in a clarify's choices ("Sat 17 + Sun 18 (Recommended)").
# The card marks it itself, so the label is shown without it.
RECOMMENDED_MARK = re.compile(r"\s*\(recommended\)\s*", re.IGNORECASE)

# The card's own button for Hermes's skip. Hermes adds 'Reply "skip" to skip this question.' to
# every question of a batch (gateway.clarify.skip_hint) and takes the word "skip" as SKIPPED, but
# only from a typed reply: an answer from a card reaches resolve_gateway_clarify as it is. So on
# a card the line promised what nothing did (VM 541, 14 Oct). It is taken off the prompt and the
# card offers Skip, which, like a typed "skip", is answered with Hermes's SKIPPED.
SKIP_LABEL = "Skip"


@dataclass
class Card:
    """A question of Hermes's shown as a card: the turn that asked it, and its choices both ways."""

    turn_id: str
    session_key: str
    # As Hermes wrote them, which is what its clarify is answered with.
    choices: List[str]
    # As the card shows them: without Hermes's "(Recommended)", which `recommended` marks instead.
    labels: List[str]
    recommended: Optional[int]
    # Hermes said the question may be skipped: the card's last button is Skip.
    skippable: bool = False

    @staticmethod
    def of(turn_id: str, session_key: str, choices: Optional[list], skippable: bool = False) -> "Card":
        written = [str(c) for c in (choices or [])]
        labels, recommended = labels_of(written)
        if skippable and SKIP_LABEL not in labels:
            labels = labels + [SKIP_LABEL]
        return Card(turn_id, session_key, written, labels, recommended, skippable)

    def choice_for(self, answer: Any) -> str:
        """The answer as Hermes knows it: a pressed button is its own choice; Skip, or the word
        "skip" typed, is Hermes's SKIPPED; other words are words."""
        said = str(answer or "")
        if self.skippable and said.strip().casefold() == SKIP_LABEL.casefold() and SKIP_LABEL.casefold() not in (c.strip().casefold() for c in self.choices):
            return skipped()
        if said in self.labels[: len(self.choices)]:
            return self.choices[self.labels.index(said)]
        return said


def skipped() -> str:
    """Hermes's own answer for a skipped question; the word itself on a Hermes without one."""
    try:
        from tools.clarify_gateway import SKIPPED

        return SKIPPED
    except Exception:
        return "skip"


def without_skip_hint(question: str) -> Tuple[str, bool]:
    """The question without Hermes's skip line, and whether it had one. The line is Hermes's own
    words in the person's language when Hermes can say them, else its English."""
    hints = []
    try:
        from agent.i18n import t

        hints.append(str(t("gateway.clarify.skip_hint")).strip())
    except Exception:
        pass
    hints.append('Reply "skip" to skip this question.')
    text = str(question or "")
    for hint in hints:
        if hint and text.rstrip().endswith(hint):
            return text.rstrip()[: -len(hint)].rstrip(), True
    return text, False


def labels_of(choices: List[str]) -> Tuple[List[str], Optional[int]]:
    """The choices without Hermes's recommendation mark, and which one carried it (the first)."""
    labels = [RECOMMENDED_MARK.sub(" ", c).strip() or c for c in choices]
    marked = [i for i, c in enumerate(choices) if RECOMMENDED_MARK.search(c)]
    return labels, (marked[0] if marked else None)
