"""What Hermes says, on its way to the desktop (a part of adapter.py, moved out of it).

`OutputMixin` holds the methods of `YantrikAdapter` that close a turn, stream text into one, and
keep what has nowhere to go. Behaviour is the adapter's; the file split only keeps each under 600
lines. The mixin relies on the adapter's attributes (`_ledger`, `_session`, `_late`...).
"""

from __future__ import annotations

import logging
import time
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional, Tuple

from gateway.platforms.base import SendResult

from . import desktop

logger = logging.getLogger(__name__)

# The destructive commands the person has already confirmed by sending them from the desktop: the
# Lens's new-conversation button sends /new, and a person typing /new or /reset there asked for it
# in so many words. Hermes 0.21 registers both as "new". Anything else (/undo) is still asked.
CONFIRMED_BY_ASKING = {"new", "reset"}
# How Hermes's "still working" heartbeat begins ("⏳ Working — 3 min — iteration 1/500, clarify").
# It names the tool and the iteration budget, which are Hermes's internals, not the answer.
STATUS_MARK = "⏳"

# What a result that arrives with no conversation open is shown under, in the next reply.
LATE_HEADING = "While you were away, Hermes finished something in the background (kept in memory, so lost if the gateway restarts):"
LATE_KEPT = 20
LATE_CHARS = 4000
LATE_CUT = "[…cut]"


def _quoted(text: str) -> str:
    """`text` as a block quote, line by line: nothing in it can start a fence, a rule, a fake turn or
    a tool-call row of its own, whatever it holds."""
    return "\n".join("> " + line if line else ">" for line in text.splitlines()) or ">"


def _cut(text: str, limit: int = LATE_CHARS) -> str:
    """At most `limit` characters, cut on a line boundary (a hard cut only for one very long line)."""
    text = text.strip()
    if len(text) <= limit:
        return text
    head = text[:limit]
    at = head.rfind("\n")
    return (head[:at] if at > 0 else head).rstrip() + "\n" + LATE_CUT


def _late_block(kept: List[Tuple[datetime, str]]) -> str:
    """What was kept, as one block: when each arrived, and what it said, quoted."""
    parts = [LATE_HEADING]
    for when, text in kept:
        parts.append(when.astimezone().strftime("%d %b %H:%M") + "\n" + _quoted(text))
    parts.append("---")
    return "\n\n".join(parts)


class OutputMixin:
    def _keep(self, content: str) -> None:
        """Remember something Hermes said with no conversation open (bounded, dated)."""
        if content.strip():
            self._late.append((datetime.now(timezone.utc), _cut(content)))
            del self._late[:-LATE_KEPT]

    async def _finish(self, turn_id: str, error: Optional[str] = None, why: str = "answered") -> None:
        """Close a turn on the desktop, exactly once.

        A turn that already said something is completed even when the gateway reports a failure:
        what was streamed stays on screen, and a failure would tell the person nothing was.
        """
        turn = self._ledger.close(turn_id)
        if turn is None or not self._session:
            return
        # Which path closed a turn is the first question when an answer goes missing: everything
        # the gateway says after a turn closes has nowhere to land.
        logger.info(
            "[yantrik] turn %s closed (%s) after %.0fs%s",
            turn_id,
            why,
            time.monotonic() - turn.opened,
            "" if turn.said_anything else ", having said nothing",
        )
        try:
            if error and not turn.said_anything:
                await self._call(
                    desktop.FAIL,
                    {"session": self._session, "turn_id": int(turn_id), "error": error},
                )
            else:
                await self._call(desktop.COMPLETE, {"session": self._session, "turn_id": int(turn_id)})
        except desktop.HarnessError as exc:
            logger.info("[yantrik] turn %s was already gone on the desktop: %s", turn_id, exc)

    async def _stream(self, turn: desktop.Turn, delta: str) -> bool:
        if not self._session:
            return False
        try:
            await self._call(
                desktop.CHUNK,
                {"session": self._session, "turn_id": int(turn.turn_id), "delta": delta},
            )
            return True
        except desktop.HarnessError as exc:
            # The desktop no longer waits for this turn (it timed out, or the shell restarted).
            logger.info("[yantrik] turn %s is closed on the desktop: %s", turn.turn_id, exc)
            self._ledger.close(turn.turn_id)
            return False

    async def send(
        self,
        chat_id: str,
        content: str,
        reply_to: Optional[str] = None,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> SendResult:
        turn = self._ledger.route(chat_id, self._carried.get(reply_to or "", reply_to))
        if _is_status(content, metadata):
            # Kept out of the answer, and given an id so Hermes edits it rather than sending anew.
            # With no turn open (a question asked, its answer not yet sent) there is no one to tell.
            if turn is None:
                return SendResult(success=False, error="no open conversation turn on the desktop")
            message_id = self._ledger.quiet(turn)
            await self._status(turn, content)
            return SendResult(success=True, message_id=message_id)
        if turn is None:
            # Something with no question to answer: a scheduled job's result, a background
            # process finishing. The desktop protocol has no way to say something unasked yet, so
            # it is kept and shown at the top of the next reply, under a line that says when it
            # came (#1070). Not dropped, and not claimed to be on screen now.
            logger.info("[yantrik] nothing open on the desktop for a %d-character message; kept", len(content))
            self._keep(content)
            return SendResult(success=True)
        if self._late and not turn.said_anything:
            # Its own message, ahead of the answer, so the answer's message id records only the
            # answer and Hermes's later edits to it diff against what was streamed. What was kept
            # when this began is what is dropped once it is shown: a result that arrives while it
            # streams stays for the next reply.
            shown = len(self._late)
            _, delta = self._ledger.sent(turn, _late_block(self._late[:shown]))
            if not await self._stream(turn, delta):
                return SendResult(success=False, error="the desktop closed this turn")
            del self._late[:shown]
        if "requires approval" in content or "/approve" in content:
            # Hermes is blocked until the person answers. Said in the log too, so a command waiting
            # on someone is visible to whoever is looking at the machine rather than only the chat.
            logger.info("[yantrik] turn %s waits for the person: %s", turn.turn_id, " ".join(content.split())[:300])
        message_id, delta = self._ledger.sent(turn, content)
        if not await self._stream(turn, delta):
            return SendResult(success=False, error="the desktop closed this turn")
        return SendResult(success=True, message_id=message_id)

    async def edit_message(
        self,
        chat_id: str,
        message_id: str,
        content: str,
        *,
        finalize: bool = False,
    ) -> SendResult:
        quiet = self._ledger.quiet_turn(message_id)
        if quiet is not None:
            await self._status(quiet, content)
            return SendResult(success=True, message_id=message_id)
        edited = self._ledger.edited(message_id, content)
        if edited is None:
            return SendResult(success=False, error="that turn is closed")
        turn, delta = edited
        if delta and not await self._stream(turn, delta):
            return SendResult(success=False, error="the desktop closed this turn")
        return SendResult(success=True, message_id=message_id)

    async def _status(self, turn: desktop.Turn, content: str) -> None:
        """Hermes's heartbeat, as what the desktop shows beside the answer: how long, nothing more."""
        line = (content.strip().splitlines() or [""])[0].lstrip(STATUS_MARK).strip()
        text = " — ".join(line.split(" — ")[:2])
        if not self._session or not text:
            return
        try:
            await self._call(
                desktop.EVENT,
                {"session": self._session, "turn_id": int(turn.turn_id), "event": {"kind": "status", "text": text}},
            )
        except desktop.HarnessError as exc:
            # A desktop older than `harness.event`, or a turn it has closed. The heartbeat is not
            # needed for anything; the adapter's own keeps the turn alive.
            self._learn_events(exc)
            logger.debug("[yantrik] turn %s status not shown: %s", turn.turn_id, exc)
            return
        self._events = True

    async def send_slash_confirm(
        self,
        chat_id: str,
        title: str,
        message: str,
        session_key: str,
        confirm_id: str,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> SendResult:
        """Hermes asks whether to go ahead with a destructive session command (Hermes 0.21+).

        For /new the person has already answered: it only reaches Hermes from this platform
        because they pressed the desktop's new-conversation button or typed it in the Lens. The
        button throws the reply away by design, so asking again put the question where nobody saw
        it, and the confirm left waiting held every turn after it — 38 minutes on VM 541 before
        anyone noticed. So it is confirmed once here and the turn that asked gets Hermes's reply.
        Anything else (/undo) returns not-supported, and Hermes asks the person in text as it
        always did. Only this platform's confirms are answered: turning off
        `approvals.destructive_slash_confirm` instead would stop Hermes asking on the person's
        other platforms too.
        """
        not_here = SendResult(success=False, error="Not supported")
        try:
            from tools import slash_confirm
        except Exception:
            return not_here
        pending = slash_confirm.get_pending(session_key) or {}
        command = str(pending.get("command") or "")
        if pending.get("confirm_id") != confirm_id or command not in CONFIRMED_BY_ASKING:
            return not_here
        turn = self._asking(chat_id)
        reply = await slash_confirm.resolve(session_key, confirm_id, "once")
        logger.info("[yantrik] /%s confirmed for the person, who sent it from the desktop", command)
        if turn is not None:
            # Hermes may answer with something other than text (an ephemeral reply), which
            # `resolve` gives back as nothing; the turn still says what happened.
            _, delta = self._ledger.sent(turn, reply or "Started a new conversation.")
            await self._stream(turn, delta)
            await self._finish(turn.turn_id, why=f"/{command} confirmed for the person")
        return SendResult(success=True)


def _is_status(content: str, metadata: Optional[Dict[str, Any]]) -> bool:
    """Whether a send is Hermes's "⏳ Working — 3 min" heartbeat rather than something to read.

    Hermes marks every mid-turn status send `_interim_send`, approval prompts included, so the
    mark alone is not enough; the heartbeat is the one of them that opens with the hourglass.
    """
    return bool((metadata or {}).get("_interim_send")) and content.lstrip().startswith(STATUS_MARK)
