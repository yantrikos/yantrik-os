"""The person's Stop, carried out in Hermes (a part of adapter.py, moved out of it).

When the person stops a turn the desktop settles it for the reader at once, and tells the harness
on its next poll (`cancelled: [turn_id]`) or, if the harness is mid-turn and only sending, by
answering a chunk `{"dropped": true}` (docs/harness.md). Both are advisory: nothing but this file
makes Hermes stop. Hermes's own stop is its `/stop` command, which interrupts the running agent
and which the gateway answers inline even while the session is busy, so it is sent exactly as the
person's typed `/stop` is, and the question the run may be parked on is let go, so the run notices.
"""

from __future__ import annotations

import logging
from datetime import datetime, timezone
from typing import Any, List

from gateway.platforms.base import MessageEvent, MessageType

from . import desktop

logger = logging.getLogger(__name__)

# The id of the /stop sent for a run: what Hermes says in reply to it is not shown.
STOP_SUFFIX = ".stop"


class StopMixin:
    async def _stop_runs(self, turn_ids: List[Any]) -> None:
        """Stop what each cancelled turn started in Hermes."""
        for raw in turn_ids:
            turn = self._ledger.get(str(raw))
            if turn is None:
                continue  # closed already: nothing of ours is running for it
            await self._stop_run(turn)

    async def _stop_run(self, turn: desktop.Turn) -> None:
        run = self._run_of(turn.turn_id)
        stop_id = f"{run}{STOP_SUFFIX}"
        if stop_id in self._stopped:
            return
        self._stopped |= {turn.turn_id, run, stop_id}
        logger.info("[yantrik] the person stopped turn %s; stopping Hermes", turn.turn_id)
        # The question first: a run parked on it does not notice an interrupt until it lets go.
        self._end_question(turn.session_key)
        await self._finish(turn.turn_id, error="Stopped before Hermes answered.", why="the person stopped it")
        if run != turn.turn_id:
            await self._finish(run, error="Stopped before Hermes answered.", why="the person stopped its run")
        if turn.session_key not in self._active_sessions:
            return  # Hermes was not working on it (never picked up, or already finished)
        try:
            await self.handle_message(self._stop_event(stop_id))
        except Exception:
            logger.exception("[yantrik] Hermes's /stop failed for turn %s", turn.turn_id)

    def _stop_event(self, stop_id: str) -> MessageEvent:
        return MessageEvent(
            text="/stop",
            message_type=MessageType.TEXT,
            source=self._desktop_source(),
            message_id=stop_id,
            timestamp=datetime.now(timezone.utc),
        )
