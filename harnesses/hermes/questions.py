"""Hermes's questions to the person, as cards on the desktop (a part of adapter.py, moved out of it).

`QuestionsMixin` holds the methods of `YantrikAdapter` that ask a question, wait for its answer,
and let go of one nobody answers. Behaviour is the adapter's; the file split only keeps each under
600 lines. The mixin relies on the adapter's attributes (`_ledger`, `_cards`, `_session`...).
"""

from __future__ import annotations

import asyncio
import logging
import os
from typing import Any, Dict, List, Optional

from gateway.platforms.base import MessageEvent, SendResult

from . import desktop
from .question import Card as _Card, without_skip_hint as _without_skip_hint

logger = logging.getLogger(__name__)

# How long a question may wait for the person before the turn stops waiting on it (#1070). Hermes
# itself waits an hour, and a turn blocked on a question nobody saw was blocked until the gateway
# was restarted (battery of 2 Nov, tasks 3, 13 and 17). Long enough to read it and think; short
# enough that "I asked and you were away" is what the person comes back to, not a frozen reply.
QUESTION_SECONDS = float(os.environ.get("YANTRIK_QUESTION_SECONDS") or 300.0)
# What Hermes is told when the question went unanswered: plain, and with a way on.
NOT_ANSWERED = (
    "The person did not answer within {minutes} minutes, so there is no answer. Do not ask the "
    "same question again. Do the part of the task that does not depend on it, and say plainly "
    "which part you did not do and that it was waiting on their answer."
)


class QuestionsMixin:
    async def _answer_question(self, event: MessageEvent, turn: desktop.Turn, session_key: str) -> None:
        """The person answered the question Hermes asked; this turn carries the rest of the run.

        The gateway takes the answer inline and says nothing itself: the run that asked goes on,
        and what it says next is the answer to this turn. So the turn is left open, and closed when
        that run ends (`on_processing_complete`), as any turn is.
        """
        card = next((cid for cid, c in self._cards.items() if c.session_key == session_key), None)
        origin = None if card is not None else self._asked.pop(session_key, None)
        await self.handle_message(event)
        if _question_pending(session_key):
            # Hermes kept the question: the reply was not one of its choices.
            if origin is not None:
                self._asked[session_key] = origin
            await self._stream(turn, "That was not one of the choices. Send its number, or your own words.")
            await self._finish(turn.turn_id, why="the answer did not fit the question")
            return
        if card is not None:
            # The question was a card, and the turn that asked it is still open: the run goes on
            # there, not here. A desktop that sends typed words to the card does not reach this;
            # one from before it does (or another of the person's platforms), and is told where.
            self._cards.pop(card, None)
            await self._stream(turn, "Sent to Hermes as the answer to its question. Its reply goes on above.")
            await self._finish(turn.turn_id, why="answered a question shown as a card")
            return
        self._ledger.mark_started(turn.turn_id)
        if origin is not None:
            self._carried[origin] = turn.turn_id
        logger.info("[yantrik] turn %s answered Hermes's question and carries its run", turn.turn_id)

    def _end_question(self, session_key: str) -> None:
        self._asked.pop(session_key, None)
        self._cards = {cid: c for cid, c in self._cards.items() if c.session_key != session_key}
        try:
            from tools import clarify_gateway

            ended = clarify_gateway.clear_session(session_key)
        except Exception:
            return
        if ended:
            logger.info("[yantrik] ended the question Hermes was waiting on")

    def _learn_events(self, exc: Exception) -> None:
        """Remember a desktop that does not know `harness.event`, so a question goes as text at once."""
        if "unknown method" in str(exc).lower():
            self._events = False

    async def send_clarify(
        self,
        chat_id: str,
        question: str,
        choices: Optional[list],
        clarify_id: str,
        session_key: str,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> SendResult:
        """Hermes asks the person something and waits for the answer (Hermes 0.21+).

        On a desktop that takes `harness.event` the question is a card in the turn that asked it
        (a `request` event, its choices as buttons and Hermes's recommended one marked), and the
        turn stays open: the answer comes back on a poll (`_take_answers`) and the run's reply goes
        on in the same bubble.

        Otherwise (an older desktop, or one that refused the card) the question is the last thing
        the asking turn says, and the turn is completed: the person's answer is the next message,
        and the desktop sends it only once this turn is done. Kept open with no card, as the base
        platform's plain `send` kept it, the answer queued behind it until Hermes's clarify timed
        out an hour later (VM 541, 14 Oct 2026). A typed number or text resolves the question as it
        does in the base, through the gateway's text intercept.
        """
        text = f"❓ {question}"
        if choices:
            numbered = [f"{i}. {choice}" for i, choice in enumerate(choices, start=1)]
            text = "\n".join([text, "", *numbered, "", "Reply with the number, the option text, or your own answer."])
            # Either way: a typed answer from a desktop that sends it as a message, or from another
            # of the person's platforms, still reaches the question through the text intercept.
            try:
                from tools import clarify_gateway

                clarify_gateway.mark_awaiting_text(clarify_id)
            except Exception:
                logger.warning("[yantrik] could not ready Hermes's question for a typed answer")
        turn = self._ledger.route(chat_id, None)
        if turn is not None and await self._ask_card(turn, question, choices, clarify_id, session_key):
            return SendResult(success=True)
        if turn is None:
            logger.info(
                "[yantrik] Hermes asked with nothing open on the desktop: %s", " ".join(text.split())[:300]
            )
            return SendResult(success=False, error="no open conversation turn on the desktop")
        self._asked[session_key] = self._run_of(turn.turn_id)
        message_id, delta = self._ledger.sent(turn, text)
        if not await self._stream(turn, delta):
            return SendResult(success=False, error="the desktop closed this turn")
        await self._finish(turn.turn_id, why="Hermes asked the person a question")
        return SendResult(success=True, message_id=message_id)

    async def _ask_card(
        self, turn: desktop.Turn, question: str, choices: Optional[list], clarify_id: str, session_key: str
    ) -> bool:
        """Show Hermes's question as a card in `turn`. False when the desktop did not take it."""
        if self._events is False or not self._session:
            return False
        prompt, skippable = _without_skip_hint(question)
        card = _Card.of(turn.turn_id, session_key, choices, skippable)
        event: Dict[str, Any] = {"kind": "request", "request_id": clarify_id, "prompt": prompt}
        if card.labels:
            event["options"] = card.labels
        if card.recommended is not None:
            event["recommended"] = card.recommended
        try:
            reply = await self._call(
                desktop.EVENT, {"session": self._session, "turn_id": int(turn.turn_id), "event": event}
            )
        except desktop.HarnessError as exc:
            self._learn_events(exc)
            logger.info("[yantrik] the desktop did not take Hermes's question as a card (%s); asking in text", exc)
            return False
        self._events = True
        not_taken = isinstance(reply, dict) and (reply.get("refused") or reply.get("ignored") or reply.get("dropped"))
        if not_taken:
            # A desktop that keeps no runs refuses a question it could not route an answer back to.
            logger.info("[yantrik] the desktop did not take Hermes's question as a card: %s; asking in text", not_taken)
            return False
        self._cards[clarify_id] = card
        logger.info("[yantrik] turn %s: Hermes's question is a card, waiting for its answer", turn.turn_id)
        self._tasks = [t for t in self._tasks if not t.done()]
        self._tasks.append(asyncio.create_task(self._expire_card(clarify_id)))
        return True

    async def _expire_card(self, clarify_id: str) -> None:
        """A question nobody answered must not hold the turn for as long as Hermes would wait.

        The first time, Hermes is told there was no answer and carries on without it, and the
        reply says so. A second question in the same turn that is also left alone ends the turn
        there, with what it did say, and the question dropped.
        """
        try:
            await asyncio.sleep(QUESTION_SECONDS)
        except asyncio.CancelledError:
            return
        card = self._cards.pop(clarify_id, None)
        if card is None:
            return  # answered, or the run ended
        minutes = max(1, round(QUESTION_SECONDS / 60))
        turn = self._ledger.get(card.turn_id)
        misses = self._unanswered[card.turn_id] = self._unanswered.get(card.turn_id, 0) + 1
        logger.info("[yantrik] turn %s: no answer to question %s in %ss", card.turn_id, clarify_id, QUESTION_SECONDS)
        if misses >= 2 or turn is None:
            self._end_question(card.session_key)
            if turn is not None:
                note = (f"Hermes asked again and there was no answer in {minutes} minutes, so it stopped "
                        "here. Ask again whenever you are ready.")
                _, delta = self._ledger.sent(turn, note)
                await self._stream(turn, delta)
                await self._finish(card.turn_id, why="the question went unanswered twice")
            return
        note = (f"No answer to the question within {minutes} minutes. Hermes carries on without it "
                "and will say what it could not do.")
        _, delta = self._ledger.sent(turn, note)
        await self._stream(turn, delta)
        try:
            from tools import clarify_gateway

            taken = clarify_gateway.resolve_gateway_clarify(clarify_id, NOT_ANSWERED.format(minutes=minutes))
        except Exception:
            logger.warning("[yantrik] could not tell Hermes its question went unanswered", exc_info=True)
            taken = False
        if not taken:
            # Hermes had already let go of it (its own timeout, a stop): nothing is waiting.
            self._end_question(card.session_key)

    def _take_answers(self, answers: List[Dict[str, Any]]) -> None:
        """Answers to questions shown as cards, from a poll: each one lets Hermes's clarify go on.

        The desktop sends an answer once, to the run that asked, and only for a question that run
        asked; the turn is checked here as well, so an answer can never release another question.
        """
        for given in answers:
            clarify_id = str(given.get("request_id") or "")
            card = self._cards.get(clarify_id)
            if card is None or str(given.get("turn_id")) != card.turn_id:
                logger.info("[yantrik] an answer came for a question no card is waiting on (%s)", clarify_id)
                continue
            del self._cards[clarify_id]
            try:
                from tools import clarify_gateway

                taken = clarify_gateway.resolve_gateway_clarify(clarify_id, card.choice_for(given.get("answer")))
            except Exception:
                logger.warning("[yantrik] could not hand Hermes the answer to its question", exc_info=True)
                continue
            if not taken:
                # Hermes's own timeout let go of it first; the run went on without the answer.
                logger.info("[yantrik] Hermes had stopped waiting for the answer to question %s", clarify_id)

    def _run_of(self, turn_id: str) -> str:
        """The turn that started the run `turn_id` carries: itself, unless it answered a question."""
        return next((run for run, carrier in self._carried.items() if carrier == turn_id), turn_id)


def _question_pending(session_key: str) -> bool:
    """Whether Hermes asked the person something and is waiting for a plain-text answer."""
    try:
        from tools import clarify_gateway

        try:
            # A question with choices counts too, as it does for the gateway's own text intercept.
            return clarify_gateway.get_pending_for_session(session_key, include_choice_prompts=True) is not None
        except TypeError:
            return clarify_gateway.get_pending_for_session(session_key) is not None
    except Exception:
        return False


def _approval_pending(session_key: str) -> bool:
    try:
        from tools import approval

        with approval._lock:
            return bool(approval._gateway_queues.get(session_key))
    except Exception:
        return False
