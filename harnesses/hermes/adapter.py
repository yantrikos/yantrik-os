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
import time
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional, Tuple

from gateway.config import Platform, PlatformConfig
from gateway.platforms.base import (
    BasePlatformAdapter,
    MessageEvent,
    MessageType,
    ProcessingOutcome,
)
from gateway.session import build_session_key

from . import bridge_token, desktop, gateway_model
from .steering import PLATFORM_HINT
from .question import Card as _Card  # noqa: F401  (named in a type comment)
from .output import CONFIRMED_BY_ASKING, OutputMixin  # noqa: F401
from .questions import QuestionsMixin, _approval_pending, _question_pending

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
# The commands that end the run in progress, and with it any question it is waiting on.
ENDS_THE_RUN = {"stop", "new", "reset"}




def check_requirements() -> bool:
    """Linux only, since the desktop's socket is a unix socket; off with YANTRIK_HARNESS=off."""
    if not sys.platform.startswith("linux"):
        return False
    return os.environ.get("YANTRIK_HARNESS", "").strip().lower() != "off"


def _detail() -> str:
    """What the mind picker shows under the name: Hermes' version and the model it thinks with.

    Read for display only: Hermes' own setting, and for `picked` the gateway's answer.
    """
    parts = []
    try:
        from hermes_cli import __version__

        parts.append(f"Hermes {__version__}")
    except Exception:
        parts.append("Hermes")
    try:
        from hermes_cli.config import load_config

        model_config = load_config().get("model") or {}
        model = model_config.get("default")
        # `picked` names nothing: the gateway says which model it is.
        if model == gateway_model.PICKED:
            model = gateway_model.real_model(model_config)
        if model:
            parts.append(str(model))
    except Exception:
        pass
    return " · ".join(parts)


class YantrikAdapter(QuestionsMixin, OutputMixin, BasePlatformAdapter):
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
        # Under the mind account (#411) the desktop refuses the tools bridge's acts unless they carry
        # an agent token: the attach answers one for the gateway's own work, and this hands it to the
        # bridge when the bridge asks (bridge_token.py says why not by environment).
        self._tokens: Optional[bridge_token.TokenService] = (
            bridge_token.TokenService() if os.environ.get("YANTRIK_MIND_RUN", "").strip() else None
        )
        self._detail_said = ""
        self._detail_task: Optional["asyncio.Task[None]"] = None
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
        # Questions no one answered in time, by the turn that asked: a second one ends the turn.
        self._unanswered: Dict[str, int] = {}
        # The text-fallback question each session is waiting on: its clarify id, beside `_asked`, and
        # which start of its timer is the live one (a rejected answer starts it again).
        self._asked_ids: Dict[str, str] = {}
        self._text_timer: Dict[str, int] = {}
        # What Hermes said with no conversation open (a background task's result, a scheduled job's):
        # the desktop's protocol has no way to say something unasked, so it is shown, labelled, at
        # the top of the next reply instead of being thrown away.
        self._late: List[Tuple[datetime, str]] = []

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
        if self._tokens is not None:
            try:
                self._tokens.start()
            except OSError as exc:
                logger.warning("[yantrik] the desktop's tools cannot be given a token (%s): they will be refused", exc.__class__.__name__)
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
        if self._tokens is not None:
            self._tokens.stop()
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
        self._detail_said = _detail()
        try:
            result = await self._call(
                desktop.ATTACH,
                {
                    "id": HARNESS_ID,
                    "name": HARNESS_NAME,
                    "detail": self._detail_said,
                    "tools": True,
                    "memory": True,
                    # The gateway's own agent (`hermes:gateway`) and its token, for the tools bridge.
                    "gateway": self._tokens is not None,
                },
            )
        except desktop.HarnessError as exc:
            logger.warning("[yantrik] could not attach to %s: %s", address, exc)
            return False
        self._session = (result or {}).get("session")
        if self._tokens is not None:
            self._tokens.set_token((result or {}).get("gateway_token"))
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
                    if self._tokens is not None:
                        self._tokens.set_token(None)
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

    async def _refresh_detail(self) -> None:
        """The model `picked` names can change while attached, so each turn says it again if it did
        (one loopback read; `harness.detail` changes the line without attaching again)."""
        try:
            detail = await asyncio.to_thread(_detail)
            if detail != self._detail_said and self._session:
                await self._call(desktop.DETAIL, {"session": self._session, "detail": detail}, timeout=3.0)
                self._detail_said = detail
        except Exception as exc:
            logger.debug("[yantrik] could not refresh the detail: %s", exc)

    def _refresh_detail_soon(self) -> None:
        """Say the model again without holding the turn: a slow gateway delays the line, never the
        reply. One refresh at a time; a turn that finds one running leaves it to finish."""
        running = self._detail_task
        if running is not None and not running.done():
            return
        self._detail_task = asyncio.get_running_loop().create_task(self._refresh_detail())

    async def _on_turn(self, assignment: Dict[str, Any]) -> None:
        turn_id = str(assignment["turn_id"])
        self._refresh_detail_soon()
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
        self._asked_ids = {k: v for k, v in self._asked_ids.items() if k in self._asked}
        # A card still waiting when its run ends will not be answered by it; the desktop closes it.
        self._cards = {cid: c for cid, c in self._cards.items() if c.turn_id not in (turn_id, event.message_id)}
        self._unanswered.pop(turn_id, None)
        self._unanswered.pop(event.message_id, None)
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


    # ── Text out ───────────────────────────────────────────────────────────────────────────────












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


def _command_of(text: str) -> str:
    """The slash command a message is, lower-cased and without its slash; empty for prose."""
    word = (text or "").strip().split(maxsplit=1)
    return word[0][1:].lower() if word and word[0].startswith("/") else ""








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
        platform_hint=PLATFORM_HINT,
    )
