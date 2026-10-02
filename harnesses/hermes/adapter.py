"""Yantrik OS desktop — a Hermes gateway platform.

Yantrik OS never reaches for a mind. It offers a socket; a harness attaches, polls for what the
person typed, and streams the answer back. This plugin makes Hermes one of those harnesses, the
way its Telegram adapter makes it a Telegram bot: the gateway runs the agent, its tools, its
approvals and its memory exactly as it always does, and this file carries text between the gateway
and the desktop's chat.

Everything that makes Hermes Hermes — the model, the endpoint, the keys — stays in ~/.hermes, where
Hermes keeps it. Nothing here reads or passes any of it to the OS, and the socket protocol has no
field that could carry it. Its memory is the one exception, in the other direction: on this machine
it is the person's YantrikDB, the memory Yantrik Mind keeps, and each turn of a Hermes the person
granted it brings the credential Hermes's YantrikDB provider presents to the memory server. This
file registers it with the provider, in this process and for the turn's gateway session alone, and
clears it with the turn that no longer brings it.

Two things differ from a chat app, and both come from the desktop thinking in turns:

- A turn is closed exactly once. The gateway's processing hooks close it; a command the gateway
  answers inline (/approve, /deny, /stop) is closed when its reply has been sent.
- While Hermes is working, a plain message is not queued behind it or used to interrupt it. The
  gateway would merge it into the next turn, and the desktop turn it came from would never be
  answered. It is told what Hermes is doing and how to stop or approve instead.
"""

from __future__ import annotations

import asyncio
import getpass
import logging
import os
import sys
import time
from datetime import datetime, timezone
from typing import Any, Dict, Optional

from gateway.config import Platform, PlatformConfig
from gateway.platforms.base import (
    BasePlatformAdapter,
    MessageEvent,
    MessageType,
    ProcessingOutcome,
    SendResult,
)
from gateway.session import build_session_key

from . import desktop, guard

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
        # Whether the last turn carried a memory credential, so a change is logged once; and
        # whether a missing memory provider has been said, so it is said once.
        self._carried_memory: Optional[bool] = None
        self._said_no_registry = False
        # Every gateway session key a credential was registered under, so a revoke reaches each.
        self._memory_keys: set = set()
        # Why this adapter will not attach, said once; empty when it may.
        self._refusal: Optional[str] = None
        self._said_refusal: Optional[str] = None
        self._said_key_mismatch = False

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
        if not is_reconnect:
            # Gateway start: re-assert the desktop platform's allowlist and the desktop's own MCP
            # server, which `hermes update` or a plugin installed since may have changed. A
            # widened one is not attached to. See guard.py.
            self._refusal = await asyncio.to_thread(guard.start)
            self._said_refusal = self._refusal
            if self._refusal:
                logger.error("[yantrik] not attaching to the desktop: %s", self._refusal)
            guard.decide(self._refusal)
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
        self._forget_memory()
        self._mark_disconnected()

    async def _call(self, method: str, params: Dict[str, Any], timeout: float = 10.0) -> Any:
        if not self._address:
            raise desktop.HarnessError("not attached to a desktop")
        return await asyncio.to_thread(desktop.call, self._address, method, params, timeout)

    async def _attach(self) -> bool:
        # Asked at every attach, not only at start: the file may have been widened since, and
        # what is refused here is said on the desktop's own row.
        # The refusal made at start stands until the gateway restarts: that gateway may have read
        # the widened file. One found here is lifted when the file is put right.
        reason = self._refusal or await asyncio.to_thread(guard.check)
        if reason != self._said_refusal:
            if reason:
                logger.error("[yantrik] not attaching to the desktop: %s", reason)
            self._said_refusal = reason
        if not guard.decide(reason):
            return False
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
                    # The credential named an agent of that session; whatever attaches next is
                    # handed its own with its first turn.
                    self._forget_memory()
                    for turn in self._ledger.open_turns():
                        self._ledger.close(turn.turn_id)
                    await asyncio.sleep(RETRY_SECONDS)
                    continue
                if isinstance(got, dict) and got.get(desktop.MEMORY_REVOKED):
                    # The person took Hermes's memory away, or it is being removed: every
                    # credential held for any session is void now, not at its next turn.
                    self._revoke_memory()
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
        session_key, agreed = self._gateway_session_key(source)
        turn.session_key = session_key
        busy = session_key in self._active_sessions
        if not busy:
            # Before the gateway sees the message, so the memory provider's prefetch for this turn
            # already presents this turn's credential, or none when the grant was taken away. Not
            # for a message that only queues behind a running turn: its credential would replace
            # the one the running turn is presenting, under the same key.
            self._carry_memory(assignment, session_key if agreed else "")

        try:
            if busy:
                await self._while_busy(event, turn, session_key)
                return
            await self.handle_message(event)
        except Exception as exc:
            logger.exception("[yantrik] turn %s failed to dispatch", turn_id)
            await self._finish(turn_id, error=f"Hermes could not take this message: {exc}")
            return
        asyncio.create_task(self._watch_pickup(turn_id))

    def _gateway_session_key(self, source) -> "tuple[str, bool]":
        """The gateway's session key for the desktop's chat, `agent:main:yantrik:dm:desktop`.

        One path, `desktop.session_key`: the gateway's own `build_session_key` with this
        platform's settings, the same inputs every time. Then checked against the gateway's own
        answer where it can be asked (its `_session_key_for_source`): a key that differs is one
        the memory provider would never be asked about, or worse, one another session's turn
        could be. It is said once, and the turn gets no memory. Answers `(key, agreed)`.
        """
        key = desktop.session_key(source, self.config.extra, build_session_key)
        for owner in (self, getattr(self._message_handler, "__self__", None)):
            resolve = getattr(owner, "_session_key_for_source", None)
            if not callable(resolve):
                continue
            try:
                theirs = resolve(source)
            except Exception:
                continue
            if isinstance(theirs, str) and theirs and theirs != key:
                if not self._said_key_mismatch:
                    logger.error("[yantrik] the gateway keys the desktop's session differently than this adapter; turns have no memory")
                    self._said_key_mismatch = True
                return key, False
            break
        return key, True

    def _carry_memory(self, assignment: Dict[str, Any], session_key: str) -> None:
        """Register this turn's memory credential with the YantrikDB provider, in this process.

        Never through the environment: the gateway runs every platform in one process, and what
        is in its environment is every platform's and every subprocess's to read.

        Fails closed. Whatever goes wrong, the session ends holding no credential: a failure that
        left an earlier turn's registered would let a turn after a revoke present the old one.
        """
        register = desktop.find_registry()
        if register is None:
            if not self._said_no_registry:
                logger.info("[yantrik] no YantrikDB memory provider to hand turns' memory credentials to")
                self._said_no_registry = True
            return
        self._said_no_registry = False
        if not session_key:
            # No trustworthy key (the gateway's differs from ours): nothing is registered, and
            # whatever any session holds is taken back.
            self._revoke_memory()
            return
        self._memory_keys.add(session_key)
        try:
            carried = desktop.carry_memory(assignment, session_key, register)
        except ValueError:
            # The address is not loopback http:// or unix:/an/absolute path, and the session was
            # cleared as it was refused. Neither the address nor the credential is repeated here.
            logger.warning("[yantrik] this turn's memory address was refused; this turn has no memory")
            self._carried_memory = None
            return
        except Exception as exc:
            # The kind only: the provider's own message could quote what it was given.
            logger.warning("[yantrik] the memory provider would not take this turn's credential (%s)", type(exc).__name__)
            self._carried_memory = None
            # Clear this session's, in a call of its own: the failed one may have left it set.
            if desktop.revoke_all({session_key}, register, None):
                # Even that failed: take back everything the provider holds.
                self._forget_memory()
            return
        if carried != self._carried_memory:
            # Whether, never what: the credential is never written to a log.
            logger.info(
                "[yantrik] %s",
                "turns now carry a credential for the machine's memory"
                if carried
                else "turns carry no credential for the machine's memory",
            )
            self._carried_memory = carried

    def _revoke_memory(self) -> None:
        """The person took the memory away: void the credential of every session, now.

        `set_desktop_credential(key, None)` for each key this adapter ever registered one under,
        then the provider's clear-everything where it has one. Failures are counted, not quoted.
        """
        keys, self._memory_keys = set(self._memory_keys), set()
        self._carried_memory = None
        failed = desktop.revoke_all(
            keys, desktop.find_registry(), desktop.find_registry(name=desktop.CLEAR_FUNCTION)
        )
        if failed:
            logger.warning("[yantrik] some memory credentials could not be taken back (%s)", ", ".join(sorted(set(failed))))
        else:
            logger.info("[yantrik] the desktop revoked Hermes's memory; every credential was taken back")

    def _forget_memory(self) -> None:
        """Take back every credential the provider holds for the desktop, which is gone or going."""
        self._carried_memory = None
        self._memory_keys = set()
        clear = desktop.find_registry(name=desktop.CLEAR_FUNCTION)
        if clear is None:
            return
        try:
            clear()
        except Exception as exc:
            logger.warning("[yantrik] the memory provider would not take its credentials back (%s)", type(exc).__name__)

    async def _while_busy(self, event: MessageEvent, turn: desktop.Turn, session_key: str) -> None:
        """A message that arrived while Hermes is working on an earlier one."""
        from hermes_cli.commands import should_bypass_active_session

        command = event.get_command()
        if (command and should_bypass_active_session(command)) or _question_pending(session_key):
            # The gateway answers these inline — an approval, a stop, the answer to a question
            # Hermes asked — and its reply has been streamed by the time this returns.
            await self.handle_message(event)
            if not turn.said_anything:
                await self._stream(turn, "Done.")
            await self._finish(turn.turn_id, why=f"/{command} answered inline")
            if command in {"stop", "new", "reset"}:
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
        if outcome == ProcessingOutcome.SUCCESS:
            await self._finish(event.message_id, why="the gateway finished it")
        elif outcome == ProcessingOutcome.CANCELLED:
            await self._finish(
                event.message_id, error="Stopped before Hermes answered.", why="cancelled"
            )
        else:
            await self._finish(
                event.message_id,
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
        turn = self._ledger.route(chat_id, reply_to)
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
        edited = self._ledger.edited(message_id, content)
        if edited is None:
            return SendResult(success=False, error="that turn is closed")
        turn, delta = edited
        if delta and not await self._stream(turn, delta):
            return SendResult(success=False, error="the desktop closed this turn")
        return SendResult(success=True, message_id=message_id)

    async def send_typing(self, chat_id: str, metadata=None) -> None:
        # Presence is the heartbeat's job; the gateway asks for typing every two seconds.
        return None

    async def get_chat_info(self, chat_id: str) -> Dict[str, Any]:
        return {"name": "Yantrik desktop", "type": "dm", "chat_id": chat_id}


def _question_pending(session_key: str) -> bool:
    """Whether Hermes asked the person something and is waiting for a plain-text answer."""
    try:
        from tools import clarify_gateway

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


def register(ctx) -> None:
    """Plugin entry point — called by the Hermes plugin system at startup."""
    # The desktop's socket lives in a directory the OS creates at mode 0700, so only processes
    # already running as the machine's owner can reach it. Whoever is typing there is that person;
    # there is no one to pair with and no one else to keep out.
    if os.environ.get("YANTRIK_HARNESS", "").strip().lower() in ("", "off") or os.environ["YANTRIK_HARNESS"].strip().startswith(("unix:", "/")):
        os.environ.setdefault("YANTRIK_ALLOW_ALL_USERS", "true")
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
