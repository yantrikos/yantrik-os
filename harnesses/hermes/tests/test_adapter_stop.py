"""The person's Stop reaches Hermes and stops it (#1087), against a stub Hermes and a fake desktop.

    python3 -m unittest discover -s harnesses/hermes/tests -p test_adapter_stop.py

The desktop tells a harness of a Stop on its next poll (`cancelled: [turn_id]`, docs/harness.md).
What is checked is what VM 520 showed: tool calls went on for minutes after the press, and a
question Hermes held took the person's next message as its answer. The stub is the smallest Hermes
that can be stopped: a run that makes tool calls until it is sent `/stop`, which the gateway
answers inline, as it does for a typed one.
"""

import asyncio
import json
import os
import socket
import tempfile
import threading
import unittest
from typing import Any, List
from unittest import mock

import test_adapter_behaviour as base
from test_adapter_behaviour import CLARIFY, AdapterCase, setUpModule, tearDownModule  # noqa: F401

SESSION = "session-1"
# How soon after the poll that carried the Stop the run must have ended. The poll loop idles 0.2 s.
WITHIN_SECONDS = 2.0


class Hermes:
    """A run that makes tool calls until it is interrupted, and the gateway's /stop."""

    def __init__(self, adapter):
        self.adapter = adapter
        self.tool_calls = 0
        self.handled: List[str] = []
        self.interrupted = asyncio.Event()
        # The end of a stopped run is reported a moment after its last words, as the gateway does.
        self.ended = asyncio.Event()
        self.ended.set()
        adapter.handle_message = self.handle_message

    async def handle_message(self, event):
        self.handled.append(event.text)
        if event.text.strip() == "/stop":
            self.interrupted.set()
            await self.adapter.send(event.source.chat_id, "Stopped.", reply_to=event.message_id)

    async def run(self, turn_id: str):
        """The run for `turn_id`: busy until interrupted, then what the gateway does as it ends."""
        gateway = base.sys.modules["gateway.platforms.base"]
        source = self.adapter._desktop_source()
        await self.adapter.on_processing_start(gateway.MessageEvent(text="long task", message_id=turn_id, source=source))
        self.adapter._active_sessions.add(SESSION)
        while not self.interrupted.is_set():
            self.tool_calls += 1
            await asyncio.sleep(0.02)
        self.adapter._active_sessions.discard(SESSION)
        await self.adapter.send(base.adapter_mod.CHAT_ID, "Stopped partway.", reply_to=turn_id)
        # Hermes's "⚡ Interrupted" notice and its progress lines name no turn.
        await self.adapter.send(base.adapter_mod.CHAT_ID, "⚡ Interrupted.", reply_to=None)
        await self.ended.wait()
        event = gateway.MessageEvent(text="long task", message_id=turn_id, source=source)
        await self.adapter.on_processing_complete(event, gateway.ProcessingOutcome.CANCELLED)


class PollingDesktop(base.FakeDesktop):
    """A desktop whose polls answer from a queue, then with nothing."""

    def __init__(self, polls: List[Any]):
        super().__init__()
        self.polls = list(polls)

    def __call__(self, address, method, params, timeout=10.0):
        if method == base.desktop_mod.POLL:
            self.calls.append((method, params))
            return self.polls.pop(0) if self.polls else {}
        return super().__call__(address, method, params, timeout)


class StopTests(AdapterCase):
    def setUp(self):
        super().setUp()
        self.hermes = Hermes(self.adapter)
        self.pending = False

        def clear(session_key):
            CLARIFY.cleared.append(session_key)
            was, self.pending = self.pending, False
            return 1 if was else 0

        for patch in (
            mock.patch.object(CLARIFY, "get_pending_for_session", lambda *a, **k: {"id": "c1"} if self.pending else None),
            mock.patch.object(CLARIFY, "clear_session", clear),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    async def press_stop(self, turn_id=7):
        """Hermes polls and is told of the Stop, as the desktop sends it."""
        self.fake = base.desktop_mod.call = PollingDesktop([{"cancelled": [turn_id]}])
        poller = asyncio.create_task(self.adapter._run())
        try:
            await asyncio.wait_for(self.hermes.interrupted.wait(), WITHIN_SECONDS)
        finally:
            poller.cancel()

    async def test_stop_mid_run_ends_the_tool_calls_within_seconds(self):
        self.open_turn()
        run = asyncio.create_task(self.hermes.run("7"))
        await asyncio.sleep(0.1)
        self.assertGreater(self.hermes.tool_calls, 0)
        await self.press_stop()
        await asyncio.wait_for(run, WITHIN_SECONDS)
        after = self.hermes.tool_calls
        await asyncio.sleep(0.2)
        self.assertEqual(self.hermes.tool_calls, after, "no tool call after the stop")
        self.assertIn("/stop", self.hermes.handled, "Hermes's own stop was used")
        self.assertEqual(self.adapter._ledger.open_turns(), [], "the turn is closed")
        self.assertEqual(len(self.fake.of(base.desktop_mod.FAIL)), 1)

    async def test_what_hermes_says_to_a_stopped_run_does_not_reach_the_next_turn(self):
        self.open_turn()
        run = asyncio.create_task(self.hermes.run("7"))
        await self.press_stop()
        await asyncio.wait_for(run, WITHIN_SECONDS)
        self.open_turn("8")
        await self.adapter.send(base.adapter_mod.CHAT_ID, "It is three o'clock.", reply_to="8")
        self.assertEqual(self.fake.said(), "It is three o'clock.")
        self.assertEqual(self.adapter._late, [])

    async def test_a_question_open_at_stop_is_cleared_and_the_next_message_is_a_new_turn(self):
        self.open_turn()
        self.pending = True
        self.adapter._asked[SESSION] = "7"
        run = asyncio.create_task(self.hermes.run("7"))
        await self.press_stop()
        await asyncio.wait_for(run, WITHIN_SECONDS)
        self.assertEqual(CLARIFY.cleared, [SESSION])
        self.assertFalse(self.pending)
        self.assertNotIn(SESSION, self.adapter._asked)

        await self.adapter._on_turn({"turn_id": 8, "text": "what time is it"})
        self.assertEqual(self.hermes.handled[-1], "what time is it", "handed to Hermes as a message of its own")
        self.assertEqual(CLARIFY.resolved, [], "and not taken as the answer to the old question")

    async def test_a_stop_for_a_turn_never_handed_to_hermes_closes_it_and_sends_no_stop(self):
        self.adapter._ledger.open("7", base.adapter_mod.CHAT_ID, "long task")
        await self.adapter._stop_runs([7])
        self.assertEqual(self.hermes.handled, [])
        self.assertEqual(self.adapter._ledger.open_turns(), [])
        self.assertEqual(self.adapter._stopping, {}, "nothing of it runs, so nothing is muted")

    async def test_a_stop_for_a_dispatched_turn_hermes_has_not_started_still_sends_stop(self):
        """Finding 2: between the hand-over and `on_processing_start`, `_active_sessions` says nothing."""
        turn = self.adapter._ledger.open("7", base.adapter_mod.CHAT_ID, "long task")
        turn.session_key, turn.dispatched = SESSION, True
        self.assertNotIn(SESSION, self.adapter._active_sessions)
        await self.adapter._stop_runs([7])
        self.assertEqual(self.hermes.handled, ["/stop"])
        self.assertEqual(self.adapter._ledger.open_turns(), [])

    async def test_a_run_that_starts_after_its_stop_is_stopped_again(self):
        """Finding 2, the gateway that only scheduled the run: it is told to stop once it can hear."""
        turn = self.adapter._ledger.open("7", base.adapter_mod.CHAT_ID, "long task")
        turn.session_key, turn.dispatched = SESSION, True
        await self.adapter._stop_runs([7])
        self.hermes.interrupted.clear()
        run = asyncio.create_task(self.hermes.run("7"))
        await asyncio.wait_for(run, WITHIN_SECONDS)
        self.assertEqual(self.hermes.handled, ["/stop", "/stop"])
        self.assertEqual(self.fake.said(), "", "nothing it said reached the desktop")

    async def test_a_turn_stopped_before_it_was_taken_is_never_handed_to_hermes(self):
        """Finding 2: a Stop for a turn the poll loop has not opened yet."""
        await self.adapter._stop_runs([7])
        await self.adapter._on_turn({"turn_id": 7, "text": "long task"})
        self.assertEqual(self.hermes.handled, [])
        self.assertEqual(self.adapter._ledger.open_turns(), [])
        self.assertEqual(len(self.fake.of(base.desktop_mod.FAIL)), 1)

    async def test_unaddressed_words_of_the_stopped_run_do_not_reach_the_next_turn(self):
        """Finding 1: the person stops 7 and sends 8 at once; the old run's last lines name no turn."""
        self.open_turn()
        self.hermes.ended.clear()
        run = asyncio.create_task(self.hermes.run("7"))
        await self.press_stop()
        self.open_turn("8")
        await asyncio.sleep(0.1)  # the stopped run says its last words, 8 open, its end not yet reported
        self.assertEqual(self.fake.said(), "")
        self.hermes.ended.set()
        await asyncio.wait_for(run, WITHIN_SECONDS)
        await self.adapter.send(base.adapter_mod.CHAT_ID, "On it.", reply_to=None)
        self.assertEqual(self.fake.said(), "On it.", "after the run's end, unaddressed words are the new turn's")
        self.assertEqual(self.adapter._late, [])

    async def test_a_new_run_starting_takes_the_chat_back_even_if_the_old_end_was_never_reported(self):
        self.open_turn()
        self.hermes.ended.clear()
        asyncio.create_task(self.hermes.run("7"))
        await self.press_stop()
        self.open_turn("8")
        gateway = base.sys.modules["gateway.platforms.base"]
        await self.adapter.on_processing_start(gateway.MessageEvent(text="next", message_id="8", source=self.adapter._desktop_source()))
        await self.adapter.send(base.adapter_mod.CHAT_ID, "On it.", reply_to=None)
        self.assertEqual(self.fake.said(), "On it.")
        self.hermes.ended.set()

    async def test_the_turn_is_closed_only_after_stop_returned_and_a_failed_stop_is_said(self):
        """Finding 3: a run whose /stop raised is not reported stopped."""
        order: List[str] = []

        async def failing(event):
            order.append("stop")
            raise RuntimeError("the gateway is wedged")

        self.adapter.handle_message = failing
        self.open_turn()
        self.fake.calls.clear()
        await self.adapter._stop_runs([7])
        self.assertEqual(order, ["stop"])
        self.assertEqual([m for m, _ in self.fake.calls], [base.desktop_mod.CHUNK, base.desktop_mod.COMPLETE])
        self.assertIn("Hermes did not stop: its /stop failed", self.fake.said())
        self.assertEqual(self.adapter._ledger.open_turns(), [])

    async def test_a_stop_that_hangs_is_given_up_on_and_said(self):
        async def hanging(event):
            await asyncio.sleep(60)

        self.adapter.handle_message = hanging
        self.open_turn()
        with mock.patch.object(base.sys.modules["yantrik_fake_gateway_plugin.stop"], "STOP_SECONDS", 0.05):
            await self.adapter._stop_runs([7])
        self.assertIn("did not return within 0 seconds", self.fake.said())
        self.assertEqual(self.adapter._ledger.open_turns(), [])

    async def test_a_cancelled_original_turn_stops_the_run_its_answer_carries(self):
        """Finding 5: 7 asked, 8 answered and carries the run; the desktop cancels 7."""
        self.open_turn("8")
        self.adapter._carried["7"] = "8"
        run = asyncio.create_task(self.hermes.run("7"))
        await asyncio.sleep(0.05)
        await self.adapter._stop_runs([7])
        await asyncio.wait_for(run, WITHIN_SECONDS)
        self.assertEqual(self.hermes.handled, ["/stop"])
        self.assertEqual(self.adapter._ledger.open_turns(), [])

    async def test_stopped_ids_are_bounded_and_forgotten_with_the_desktop_session(self):
        """Finding 4."""
        kept = base.sys.modules["yantrik_fake_gateway_plugin.stop"].STOPPED_KEPT
        await self.adapter._stop_runs(list(range(1, kept + 10)))
        self.assertEqual(len(self.adapter._stopped), kept)
        self.assertNotIn("1", self.adapter._stopped)
        self.assertIn(str(kept + 9), self.adapter._stopped)

        def lost(address, method, params, timeout=10.0):
            raise base.desktop_mod.HarnessError("unknown session")

        base.desktop_mod.call = lost
        poller = asyncio.create_task(self.adapter._run())
        await asyncio.sleep(0.05)
        poller.cancel()
        self.assertEqual(self.adapter._stopped, {})
        self.assertIsNone(self.adapter._session)

    async def test_a_second_stop_for_the_same_run_sends_nothing_more(self):
        self.open_turn()
        run = asyncio.create_task(self.hermes.run("7"))
        await asyncio.sleep(0.05)
        await self.adapter._stop_runs([7, 7])
        await asyncio.wait_for(run, WITHIN_SECONDS)
        await self.adapter._stop_runs([7])
        self.assertEqual(self.hermes.handled, ["/stop"])

    async def test_a_stop_for_a_turn_already_closed_does_nothing(self):
        await self.adapter._stop_runs([7])
        self.assertEqual(self.hermes.handled, [])
        self.assertEqual(self.fake.calls, [])


@unittest.skipUnless(hasattr(socket, "AF_UNIX"), "the harness socket is a unix socket")
class ProtocolRoundTripTests(AdapterCase):
    """`cancelled` as the desktop writes it, read by the real socket client."""

    def setUp(self):
        super().setUp()
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        path = os.path.join(self.dir.name, "harness.sock")
        self.seen: List[dict] = []
        self.told = False
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(path)
        self.server.listen(8)
        self.addCleanup(self.server.close)
        threading.Thread(target=self._serve, daemon=True).start()
        base.desktop_mod.call = self._saved[0]  # the real client, not the fake
        self.adapter._address = path

    def _serve(self):
        while True:
            try:
                conn, _ = self.server.accept()
            except OSError:
                return
            with conn:
                request = json.loads(conn.makefile("rb").readline())
                self.seen.append(request)
                result: Any = {}
                if request["method"] == "harness.poll" and not self.told:
                    self.told = True
                    result = {"cancelled": [7]}
                reply = {"jsonrpc": "2.0", "id": request["id"], "result": result}
                conn.sendall(json.dumps(reply).encode() + b"\n")

    async def test_a_cancelled_turn_in_a_poll_reply_stops_the_run_and_closes_the_turn(self):
        hermes = Hermes(self.adapter)
        self.open_turn()
        run = asyncio.create_task(hermes.run("7"))
        poller = asyncio.create_task(self.adapter._run())
        try:
            await asyncio.wait_for(run, WITHIN_SECONDS)
        finally:
            poller.cancel()
        methods = [r["method"] for r in self.seen]
        self.assertEqual(methods[0], "harness.poll")
        fail = next(r for r in self.seen if r["method"] == "harness.fail")
        self.assertEqual(fail["params"]["turn_id"], 7, "the turn is closed on the desktop")


if __name__ == "__main__":
    unittest.main()
