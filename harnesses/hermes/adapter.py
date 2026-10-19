"""Yantrik OS desktop — a Hermes gateway platform.

Yantrik OS never reaches for a mind. It offers a socket; a harness attaches, polls for what the
person typed, and streams the answer back. This plugin makes Hermes one of those harnesses, the
way its Telegram adapter makes it a Telegram bot: the gateway runs the agent, its tools, its
approvals and its memory exactly as it always does, and this file carries text between the gateway
and the desktop's chat.

Everything that makes Hermes Hermes — the model, the endpoint, the keys, the memory provider —
stays in ~/.hermes, where Hermes keeps it. Nothing here reads or passes any of it to the OS, and the
socket protocol has no field that could carry it.

Two things differ from a chat app, and both come from the desktop thinking in turns:

- A turn is closed exactly once. The gateway's processing hooks close it; a command the gateway
  answers inline (/approve, /deny, /stop) is closed when its reply has been sent.
- While Hermes is working, a plain message is not queued behind it or used to interrupt it. The
  gateway would merge it into the next turn, and the desktop turn it came from would never be
  answered. It is told what Hermes is doing and how to stop or approve instead.
- A question Hermes asks (its clarify tool) is a card in the turn that asked it, on a desktop that
  takes `harness.event`: the answer comes back on a poll, inside that turn, and the run goes on in
  the same reply. On an older desktop the question ends the turn that asked it instead, and the
  person's answer is a turn of its own that carries the rest of the run. Left open with no card,
  the asking turn held the answer behind it until Hermes gave up waiting an hour later.
"""

from __future__ import annotations

import asyncio
import getpass
import logging
import os
import sys
import re
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional, Tuple

from gateway.config import Platform, PlatformConfig
from gateway.platforms.base import (
    BasePlatformAdapter,
    MessageEvent,
    MessageType,
    ProcessingOutcome,
    SendResult,
)
from gateway.session import build_session_key

from . import desktop

logger = logging.getLogger(__name__)

PLATFORM = "yantrik"
HARNESS_ID = "hermes"
HARNESS_NAME = "Hermes Agent"
# The desktop has one conversation with its mind, so it is one chat.
CHAT_ID = "desktop"
OWNER_ID = "owner"

# Answers are streamed in pieces, so no length limit applies; a limit would only make the gateway
# split an answer into numbered parts.
MAX_MESSAGE_LENGTH = 1_000_000
# How long to wait after an empty poll. The protocol's own pacing (docs/harness.md).
IDLE_SECONDS = 0.2
# How long to wait before looking for the desktop again after it went away.
RETRY_SECONDS = 5.0
# A turn being worked on says so this often, well inside the desktop's 90-second window.
HEARTBEAT_SECONDS = 20.0
# A message the gateway has not started working on after this long will not be answered. Generous,
# because closing a turn early loses the answer: everything the gateway says afterwards has nowhere
# to go, and the person is left looking at a dead conversation while Hermes works on.
PICKUP_SECONDS = 180.0
# How quiet a turn whose gateway session has ended must be before it is treated as abandoned.
ABANDONED_QUIET_SECONDS = 90.0
# The destructive commands the person has already confirmed by sending them from the desktop: the
# Lens's new-conversation button sends /new, and a person typing /new or /reset there asked for it
# in so many words. Hermes 0.21 registers both as "new". Anything else (/undo) is still asked.
CONFIRMED_BY_ASKING = {"new", "reset"}
# The commands that end the run in progress, and with it any question it is waiting on.
ENDS_THE_RUN = {"stop", "new", "reset"}
# How Hermes's "still working" heartbeat begins ("⏳ Working — 3 min — iteration 1/500, clarify").
# It names the tool and the iteration budget, which are Hermes's internals, not the answer.
STATUS_MARK = "⏳"
# How Hermes marks the choice it recommends in a clarify's choices ("Sat 17 + Sun 18 (Recommended)").
# The card marks it itself, so the label is shown without it.
RECOMMENDED_MARK = re.compile(r"\s*\(recommended\)\s*", re.IGNORECASE)

# The card's own button for Hermes's skip. Hermes adds 'Reply "skip" to skip this question.' to
# every question of a batch (gateway.clarify.skip_hint) and takes the word "skip" as SKIPPED, but
# only from a typed reply: an answer from a card reaches resolve_gateway_clarify as it is. So on
# a card the line promised what nothing did (VM 541, 14 Oct). It is taken off the prompt and the
# card offers Skip, which, like a typed "skip", is answered with Hermes's SKIPPED.
SKIP_LABEL = "Skip"


def check_requirements() -> bool:
    """Linux only, since the desktop's socket is a unix socket; off with YANTRIK_HARNESS=off."""
    if not sys.platform.startswith("linux"):
        return False
    return os.environ.get("YANTRIK_HARNESS", "").strip().lower() != "off"


def _detail() -> str:
    """What the mind picker shows under the name: Hermes' version and the model it thinks with.

    Read for display only, and only as Hermes itself reports it.
    """
    parts = []
    try:
        from hermes_cli import __version__

        parts.append(f"Hermes {__version__}")
    except Exception:
        parts.append("Hermes")
    try:
        from hermes_cli.config import load_config

        model = (load_config().get("model") or {}).get("default")
        if model:
            parts.append(str(model))
    except Exception:
        pass
    return " · ".join(parts)


class YantrikAdapter(BasePlatformAdapter):
    """The desktop's chat, as a Hermes platform."""

    MAX_MESSAGE_LENGTH = MAX_MESSAGE_LENGTH
    SUPPORTS_MESSAGE_EDITING = True

    def __init__(self, config: PlatformConfig):
        super().__init__(config=config, platform=Platform(PLATFORM))
        self._ledger = desktop.Ledger()
        self._address: Optional[str] = None
        self._session: Optional[str] = None
        self._tasks: list[asyncio.Task] = []
        self._owner = os.environ.get("USER") or getpass.getuser() or OWNER_ID
        self._said_no_desktop = False
        # A run that asked the person a question goes on in the turn that answered it. The gateway
        # still knows the run by the turn that started it, so: the turn that started a run -> the
        # turn now carrying it, and the session whose question is waiting -> the run that asked.
        self._carried: Dict[str, str] = {}
        self._asked: Dict[str, str] = {}
        # Whether the desktop takes `harness.event`: None until it has answered one. A question is
        # offered as a card until the desktop says it does not know the method.
        self._events: Optional[bool] = None
        # Questions shown as cards, by Hermes's clarify id. The answer arrives on a later poll.
        self._cards: Dict[str, "_Card"] = {}

    # ── Lifecycle ──────────────────────────────────────────────────────────────────────────────

    async def connect(self, *, is_reconnect: bool = False) -> bool:
        # `is_reconnect` is Hermes's reconnect watcher re-establishing a platform it saw drop, so
        # that a platform with a server-side queue (Telegram's) keeps what arrived meanwhile. The
        # desktop holds no such queue for us — the poll loop simply attaches again — so a reconnect
        # is a connect. Hermes started passing it to every connect, the first one included, and an
        # adapter without the keyword never connected at all. What a reconnect must not do is leave
        # the last connection's loops running beside the new ones: two pollers would each take
        # turns and two heartbeats would keep one session alive twice over.
        stale, self._tasks = self._tasks, []
        for task in stale:
            task.cancel()
        if stale:
            await asyncio.gather(*stale, return_exceptions=True)
        self._tasks = [
            asyncio.create_task(self._run(), name="yantrik-poll"),
            asyncio.create_task(self._heartbeat(), name="yantrik-heartbeat"),
        ]
        # Connected to the gateway. Reaching the desktop is the poll loop's job, and it keeps
        # trying: the desktop starting after Hermes, or restarting, is ordinary.
        self._mark_connected()
        return True

    async def disconnect(self) -> None:
        stale, self._tasks = self._tasks, []
        for task in stale:
            task.cancel()
        if stale:
            await asyncio.gather(*stale, return_exceptions=True)
        self._tasks = []
        if self._address and self._session:
            try:
                await self._call(desktop.DETACH, {"session": self._session}, timeout=3.0)
            except desktop.HarnessError:
                pass
        self._session = None
        self._mark_disconnected()

    async def _call(self, method: str, params: Dict[str, Any], timeout: float = 10.0) -> Any:
        if not self._address:
            raise desktop.HarnessError("not attached to a desktop")
        return await asyncio.to_thread(desktop.call, self._address, method, params, timeout)

    async def _attach(self) -> bool:
        address = desktop.socket_path()
        if not address:
            if not self._said_no_desktop:
                logger.info("[yantrik] no desktop to attach to yet; looking every %ss", RETRY_SECONDS)
                self._said_no_desktop = True
            return False
        self._address = address
        try:
            result = await self._call(
                desktop.ATTACH,
                {
                    "id": HARNESS_ID,
                    "name": HARNESS_NAME,
                    "detail": _detail(),
                    "tools": True,
                    "memory": True,
                },
            )
        except desktop.HarnessError as exc:
            logger.warning("[yantrik] could not attach to %s: %s", address, exc)
            return False
        self._session = (result or {}).get("session")
        self._said_no_desktop = False
        logger.info("[yantrik] attached to the desktop at %s as `%s`", address, HARNESS_ID)
        return bool(self._session)

    async def _run(self) -> None:
        while True:
            try:
                if not self._session and not await self._attach():
                    await asyncio.sleep(RETRY_SECONDS)
                    continue
                try:
                    got = await self._call(desktop.POLL, {"session": self._session})
                except desktop.HarnessError as exc:
                    # The shell restarted or dropped this session. Whatever it owed is already
                    # failed on its side; attach again and carry on.
                    logger.warning("[yantrik] lost the desktop (%s); attaching again", exc)
                    self._session = None
                    for turn in self._ledger.open_turns():
                        self._ledger.close(turn.turn_id)
                    await asyncio.sleep(RETRY_SECONDS)
                    continue
                if isinstance(got, dict):
                    self._take_answers(got.get("answers") or [])
                if isinstance(got, dict) and got.get("turn_id") is not None:
                    asyncio.create_task(self._on_turn(got))
                    continue
                await asyncio.sleep(IDLE_SECONDS)
            except asyncio.CancelledError:
                return
            except Exception:
                logger.exception("[yantrik] poll loop error")
                await asyncio.sleep(RETRY_SECONDS)

    async def _heartbeat(self) -> None:
        """Tell the desktop every open turn is still being worked on.

        An empty chunk is presence, not text. A long task goes minutes between tool calls, and
        without this the desktop would decide the mind had gone.
        """
        while True:
            try:
                await asyncio.sleep(HEARTBEAT_SECONDS)
                for turn in self._ledger.open_turns():
                    if self._abandoned(turn):
                        logger.warning("[yantrik] turn %s outlived its session; closing it", turn.turn_id)
                        await self._finish(
                            turn.turn_id,
                            error="Stopped before Hermes answered.",
                            why="its session ended and it went quiet",
                        )
                        continue
                    await self._stream(turn, "")
            except asyncio.CancelledError:
                return
            except Exception:
                logger.exception("[yantrik] heartbeat error")

    def _abandoned(self, turn: desktop.Turn) -> bool:
        """Whether a turn the gateway was working on is no longer being worked on by anyone.

        The processing hooks close a turn in the ordinary case, but not every path through the
        gateway reaches them — a /stop releases the session without the hook ever firing. A turn
        left open would be kept alive by the heartbeat forever, so one whose session has been
        gone for two heartbeats in a row is closed here.
        """
        if not turn.started or not turn.session_key:
            return False
        if turn.session_key in self._active_sessions:
            turn.idle_beats = 0
            return False
        # A session can leave the gateway's active list while its answer is still on the way, so
        # the turn must also have gone quiet. Closing a live turn loses everything said next.
        if time.monotonic() - turn.active < ABANDONED_QUIET_SECONDS:
            return False
        turn.idle_beats += 1
        return turn.idle_beats >= 2

    # ── Turns in ───────────────────────────────────────────────────────────────────────────────

    async def _on_turn(self, assignment: Dict[str, Any]) -> None:
        turn_id = str(assignment["turn_id"])
        text = str(assignment.get("text") or "")
        source = self.build_source(
            chat_id=CHAT_ID,
            chat_name="Yantrik desktop",
            chat_type="dm",
            user_id=OWNER_ID,
            user_name=self._owner,
        )
        event = MessageEvent(
            text=text,
            message_type=MessageType.TEXT,
            source=source,
            message_id=turn_id,
            timestamp=datetime.now(timezone.utc),
        )
        turn = self._ledger.open(turn_id, CHAT_ID, text)
        session_key = build_session_key(
            source,
            group_sessions_per_user=self.config.extra.get("group_sessions_per_user", True),
            thread_sessions_per_user=self.config.extra.get("thread_sessions_per_user", False),
        )
        turn.session_key = session_key

        try:
            if session_key in self._active_sessions:
                await self._while_busy(event, turn, session_key)
                return
            await self.handle_message(event)
        except Exception as exc:
            logger.exception("[yantrik] turn %s failed to dispatch", turn_id)
            await self._finish(turn_id, error=f"Hermes could not take this message: {exc}")
            return
        asyncio.create_task(self._watch_pickup(turn_id))

    async def _while_busy(self, event: MessageEvent, turn: desktop.Turn, session_key: str) -> None:
        """A message that arrived while Hermes is working on an earlier one."""
        from hermes_cli.commands import should_bypass_active_session

        command = event.get_command()
        if not command and _question_pending(session_key):
            await self._answer_question(event, turn, session_key)
            return
        if command and should_bypass_active_session(command):
            # The gateway answers these inline — an approval, a stop — and its reply has been
            # streamed by the time this returns.
            await self.handle_message(event)
            if command in ENDS_THE_RUN:
                # A run waiting on the person's answer does not notice a stop until the question
                # lets go of it, which by itself is an hour later.
                self._end_question(session_key)
            # A /new confirmed on the person's behalf (`send_slash_confirm`) has closed its turn
            # already, with Hermes's own reply in it.
            if self._ledger.get(turn.turn_id) is not None and not turn.said_anything:
                await self._stream(turn, "Done.")
            await self._finish(turn.turn_id, why=f"/{command} answered inline")
            if command in ENDS_THE_RUN:
                # These end the work in progress without its processing hook firing, so the turns
                # it was answering are closed here rather than left waiting.
                for other in self._ledger.open_turns():
                    if other.chat_id == turn.chat_id and other.started:
                        await self._finish(
                            other.turn_id,
                            error="Stopped before Hermes answered.",
                            why=f"ended by /{command}",
                        )
            return

        working = [t for t in self._ledger.open_turns() if t.started and t.turn_id != turn.turn_id]
        about = working[0].text.strip().splitlines()[0][:80] if working and working[0].text.strip() else ""
        waiting = _approval_pending(session_key)
        lines = [f"Hermes is still working on “{about}”." if about else "Hermes is still working."]
        if _question_pending(session_key):
            # A command is not an answer: Hermes keeps the question for the next plain message.
            lines = ["Hermes is waiting for the answer to its question. Send it as a message, or /stop to drop it."]
        else:
            if waiting:
                lines.append("It is waiting for you to allow a command: send /approve or /deny.")
            lines.append("Send /stop to stop it. Anything else can wait until it has finished.")
        await self._stream(turn, " ".join(lines))
        await self._finish(turn.turn_id, why="told the person Hermes is busy")

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

    async def _watch_pickup(self, turn_id: str) -> None:
        await asyncio.sleep(PICKUP_SECONDS)
        turn = self._ledger.get(turn_id)
        if turn is not None and not turn.started and not turn.said_anything:
            logger.warning("[yantrik] turn %s was never picked up by the gateway", turn_id)
            await self._finish(
                turn_id,
                error="Hermes did not take this message up. Try again.",
                why="never picked up",
            )

    async def on_processing_start(self, event: MessageEvent) -> None:
        if event.message_id:
            self._ledger.mark_started(event.message_id)

    async def on_processing_complete(self, event: MessageEvent, outcome: ProcessingOutcome) -> None:
        if not event.message_id:
            return
        # A run that asked a question ends in the turn that answered it.
        turn_id = self._carried.pop(event.message_id, None) or event.message_id
        self._asked = {k: v for k, v in self._asked.items() if v != event.message_id}
        # A card still waiting when its run ends will not be answered by it; the desktop closes it.
        self._cards = {cid: c for cid, c in self._cards.items() if c.turn_id not in (turn_id, event.message_id)}
        if outcome == ProcessingOutcome.SUCCESS:
            await self._finish(turn_id, why="the gateway finished it")
        elif outcome == ProcessingOutcome.CANCELLED:
            await self._finish(turn_id, error="Stopped before Hermes answered.", why="cancelled")
        else:
            await self._finish(
                turn_id,
                error="Hermes could not answer this. Its log has the reason: "
                "journalctl --user -u hermes-gateway",
                why="the gateway failed it",
            )

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

    # ── Text out ───────────────────────────────────────────────────────────────────────────────

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
            # process finishing. The desktop protocol has no way to say something unasked yet.
            logger.info("[yantrik] nothing open on the desktop for a %d-character message", len(content))
            return SendResult(success=False, error="no open conversation turn on the desktop")
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
        return True

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

    def _asking(self, chat_id: str) -> Optional[desktop.Turn]:
        """The open turn that sent a /new or /reset, newest first; else the newest in the chat."""
        asking = [
            t
            for t in self._ledger.open_turns()
            if t.chat_id == chat_id and _command_of(t.text) in CONFIRMED_BY_ASKING
        ]
        if asking:
            return max(asking, key=lambda t: t.opened)
        return self._ledger.route(chat_id, None)

    async def send_typing(self, chat_id: str, metadata=None) -> None:
        # Presence is the heartbeat's job; the gateway asks for typing every two seconds.
        return None

    async def get_chat_info(self, chat_id: str) -> Dict[str, Any]:
        return {"name": "Yantrik desktop", "type": "dm", "chat_id": chat_id}


@dataclass
class _Card:
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
    def of(turn_id: str, session_key: str, choices: Optional[list], skippable: bool = False) -> "_Card":
        written = [str(c) for c in (choices or [])]
        labels, recommended = _labels(written)
        if skippable and SKIP_LABEL not in labels:
            labels = labels + [SKIP_LABEL]
        return _Card(turn_id, session_key, written, labels, recommended, skippable)

    def choice_for(self, answer: Any) -> str:
        """The answer as Hermes knows it: a pressed button is its own choice; Skip, or the word
        "skip" typed, is Hermes's SKIPPED; other words are words."""
        said = str(answer or "")
        if self.skippable and said.strip().casefold() == SKIP_LABEL.casefold() and SKIP_LABEL.casefold() not in (c.strip().casefold() for c in self.choices):
            return _skipped()
        if said in self.labels[: len(self.choices)]:
            return self.choices[self.labels.index(said)]
        return said


def _skipped() -> str:
    """Hermes's own answer for a skipped question; the word itself on a Hermes without one."""
    try:
        from tools.clarify_gateway import SKIPPED

        return SKIPPED
    except Exception:
        return "skip"


def _without_skip_hint(question: str) -> Tuple[str, bool]:
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


def _labels(choices: List[str]) -> Tuple[List[str], Optional[int]]:
    """The choices without Hermes's recommendation mark, and which one carried it (the first)."""
    labels = [RECOMMENDED_MARK.sub(" ", c).strip() or c for c in choices]
    marked = [i for i, c in enumerate(choices) if RECOMMENDED_MARK.search(c)]
    return labels, (marked[0] if marked else None)


def _command_of(text: str) -> str:
    """The slash command a message is, lower-cased and without its slash; empty for prose."""
    word = (text or "").strip().split(maxsplit=1)
    return word[0][1:].lower() if word and word[0].startswith("/") else ""


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


def _is_status(content: str, metadata: Optional[Dict[str, Any]]) -> bool:
    """Whether a send is Hermes's "⏳ Working — 3 min" heartbeat rather than something to read.

    Hermes marks every mid-turn status send `_interim_send`, approval prompts included, so the
    mark alone is not enough; the heartbeat is the one of them that opens with the hourglass.
    """
    return bool((metadata or {}).get("_interim_send")) and content.lstrip().startswith(STATUS_MARK)


def _approval_pending(session_key: str) -> bool:
    try:
        from tools import approval

        with approval._lock:
            return bool(approval._gateway_queues.get(session_key))
    except Exception:
        return False


def register(ctx) -> None:
    """Plugin entry point — called by the Hermes plugin system at startup."""
    # Every desktop turn arrives as the one user OWNER_ID, and only the machine's owner can reach the
    # harness socket (its directory is 0700 and the host checks the kernel's peer credentials at
    # accept). So the platform's allowlist is that one user: never "all users", never a wildcard.
    # hermes.sh writes the same into Hermes's own .env, which is what Hermes 0.21+ reads (#676).
    os.environ.setdefault("YANTRIK_ALLOWED_USERS", OWNER_ID)
    # The desktop has one conversation, so it is home. Without a home, the gateway opens the first
    # answer on a new machine with a paragraph asking the person to type /sethome.
    os.environ.setdefault("YANTRIK_HOME_CHANNEL", CHAT_ID)
    ctx.register_platform(
        name=PLATFORM,
        label="Yantrik OS",
        adapter_factory=lambda cfg: YantrikAdapter(cfg),
        check_fn=check_requirements,
        validate_config=lambda cfg: True,
        required_env=[],
        install_hint="Runs on a Yantrik OS machine; attaches to the desktop's harness socket.",
        allowed_users_env="YANTRIK_ALLOWED_USERS",
        allow_all_env="YANTRIK_ALLOW_ALL_USERS",
        cron_deliver_env_var="YANTRIK_HOME_CHANNEL",
        max_message_length=MAX_MESSAGE_LENGTH,
        emoji="🖥️",
        pii_safe=True,
        platform_hint=(
            "You are the mind answering the Yantrik OS desktop: the chat panel of the computer "
            "you are running on, used by its owner. Markdown renders. Your tools act on this same "
            "machine. The yantrik_os MCP tools read and drive the desktop's own apps (notes, "
            "files, calendar, email, the shell); prefer them for anything a person would do in "
            "those apps. Long work is fine: the person sees your progress as you go and can send "
            "/stop, or /approve and /deny when a command needs their permission."
        ),
    )
