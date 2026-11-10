"""The person's Stop, carried out in Hermes (a part of adapter.py, moved out of it).

When the person stops a turn the desktop settles it for the reader at once and tells the harness
on its next poll (`cancelled: [turn_id]`, docs/harness.md). Nothing but this file makes Hermes
stop. Hermes's own stop is its `/stop` command, which interrupts the running agent and which the
gateway answers inline even while the session is busy, so it is sent exactly as the person's
typed `/stop` is. The order matters: the question the run may be parked on is let go first (a run
parked on it does not notice an interrupt until it does), then `/stop`, and the turn is closed on
the desktop only once `/stop` has returned, so "stopped" is never said of a run that was not.
"""

from __future__ import annotations

import asyncio
import logging
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional

from gateway.platforms.base import MessageEvent, MessageType

from . import desktop

logger = logging.getLogger(__name__)

# The id of the /stop sent for a run: what Hermes says in reply to it is not shown.
STOP_SUFFIX = ".stop"
# How long Hermes's /stop may take before the turn is closed as not stopped. The gateway bounds
# its own cancel at 5 s; the poll loop waits here, so a wedged stop must not hold the next turn.
STOP_SECONDS = 15.0
# How many stopped ids are remembered. Turn ids never repeat while the desktop keeps runs, so the
# only cost of an old id is memory; the oldest are forgotten first.
STOPPED_KEPT = 64
NOT_STOPPED = (
    "Stopped on the desktop, but Hermes did not stop: its /stop {how}. "
    "Its log has the reason: journalctl --user -u hermes-gateway"
)


class StopMixin:
    def _init_stop(self) -> None:
        # Runs the person stopped, the turns that carried them and the /stop sent for each, oldest
        # first: whatever Hermes still says in reply to them is not shown, and a turn that arrives
        # with a stopped id is refused rather than run.
        self._stopped: Dict[str, float] = {}
        # The chat a stop is in flight for, and the run it stops: until that run's end is reported,
        # anything said to the chat with no `reply_to` is the stopped run's, not the next turn's.
        self._stopping: Dict[str, str] = {}

    def _forget_stops(self) -> None:
        """A new desktop session: its turn ids are its own, and nothing of the old one is running."""
        self._stopped.clear()
        self._stopping.clear()

    def _mark_stopped(self, *ids: str) -> None:
        now = datetime.now(timezone.utc).timestamp()
        for turn_id in ids:
            self._stopped[turn_id] = now
        for old in list(self._stopped)[: max(0, len(self._stopped) - STOPPED_KEPT)]:
            del self._stopped[old]

    def _muted(self, chat_id: str, reply_to: Optional[str]) -> bool:
        """Whether a send is the stopped run's: in reply to it, or unaddressed while its stop is in flight."""
        return reply_to in self._stopped or (reply_to is None and chat_id in self._stopping)

    def _stop_done(self, run: str, *carriers: str) -> None:
        """The run's end was reported: nothing more is said to it, and the chat is the next turn's."""
        for chat_id, stopping in list(self._stopping.items()):
            if stopping == run:
                del self._stopping[chat_id]
        for turn_id in (run, f"{run}{STOP_SUFFIX}", *carriers):
            self._stopped.pop(turn_id, None)

    async def _stop_runs(self, turn_ids: List[Any]) -> None:
        """Stop what each cancelled turn started in Hermes."""
        for raw in turn_ids:
            turn_id = str(raw)
            # The turn itself, or the one now carrying its run after a question was answered.
            turn = self._ledger.get(turn_id) or self._ledger.get(self._carried.get(turn_id, ""))
            if turn is None:
                # Closed already, or not yet taken: if it arrives now, it is refused (`_on_turn`).
                self._mark_stopped(turn_id)
                continue
            await self._stop_run(turn)

    async def _stop_run(self, turn: desktop.Turn) -> None:
        run = self._run_of(turn.turn_id)
        stop_id = f"{run}{STOP_SUFFIX}"
        if stop_id in self._stopped:
            return  # stopped already; a second Stop is harmless
        self._mark_stopped(turn.turn_id, run, stop_id)
        self._stopping[turn.chat_id] = run
        logger.info("[yantrik] the person stopped turn %s; stopping Hermes", turn.turn_id)
        self._end_question(turn.session_key)
        error = "Stopped before Hermes answered."
        if turn.dispatched:
            failed = await self._send_stop(stop_id, turn.turn_id)
            if failed:
                # Said in the turn, so it is read whether or not the turn had said anything yet.
                error = NOT_STOPPED.format(how=failed)
                _, delta = self._ledger.sent(turn, error)
                await self._stream(turn, delta)
        else:
            self._stop_done(run)  # never handed to Hermes: nothing of it runs
        await self._finish(turn.turn_id, error=error, why="the person stopped it")
        if run != turn.turn_id:
            await self._finish(run, error=error, why="the person stopped its run")

    def _run_started(self, event: MessageEvent) -> None:
        """A run began: the chat's unaddressed sends are its, unless it is one the person stopped
        before Hermes picked it up, which is told to stop again now that it can hear."""
        run = event.message_id
        if run in self._stopped:
            logger.warning("[yantrik] turn %s started after the person stopped it; stopping it again", run)
            self._tasks.append(asyncio.create_task(self._send_stop(f"{run}{STOP_SUFFIX}", run)))
            return
        chat_id = getattr(event.source, "chat_id", None)
        if self._stopping.get(chat_id) != run:
            self._stopping.pop(chat_id, None)

    async def _send_stop(self, stop_id: str, turn_id: str) -> Optional[str]:
        """Hermes's own /stop for the run, waited for. None when it took; else what went wrong."""
        try:
            await asyncio.wait_for(self.handle_message(self._stop_event(stop_id)), STOP_SECONDS)
        except asyncio.TimeoutError:
            logger.error("[yantrik] Hermes's /stop for turn %s took longer than %ss", turn_id, STOP_SECONDS)
            return f"did not return within {STOP_SECONDS:.0f} seconds"
        except Exception:
            logger.exception("[yantrik] Hermes's /stop failed for turn %s", turn_id)
            return "failed"
        return None

    def _stop_event(self, stop_id: str) -> MessageEvent:
        return MessageEvent(
            text="/stop",
            message_type=MessageType.TEXT,
            source=self._desktop_source(),
            message_id=stop_id,
            timestamp=datetime.now(timezone.utc),
        )
