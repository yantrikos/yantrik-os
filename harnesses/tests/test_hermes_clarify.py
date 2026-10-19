"""The Hermes adapter against a fake gateway: a question Hermes asks can be answered from the desktop.

On VM 541 (14 Oct 2026) Hermes asked "which day is this weekend for you?" through its clarify tool.
The question streamed into the open desktop turn and the turn stayed open, so the desktop held the
person's answer behind it, Hermes waited an hour for an answer that never came, and its "⏳ Working —
iteration 1/500, clarify" heartbeat kept growing in the bubble.

The run here is Hermes's as far as these turns go: `handle_message` starts it in the background and
returns, it asks through `send_clarify`, waits in `tools.clarify_gateway`, and is ended by its
processing hook. A message while it is busy goes inline, the way the base platform's
`_handle_message_while_active` sends a clarify answer or a /stop to the gateway's text intercept.

Two desktops: one from before `harness.event` (the question is text and ends its turn, #802), and
one that takes it (the question is a card in the turn that asked, answered on a later poll).
"""

import asyncio
import sys
import types
import unittest
from types import SimpleNamespace
from unittest import mock

import test_hermes_adapter as hermes

ADAPTER, SESSION, ProcessingOutcome = hermes.ADAPTER, hermes.SESSION, hermes.ProcessingOutcome
CANCELLED = "\x00cancelled"
SKIPPED = "\x00skipped"


def clarify_module():
    """`tools.clarify_gateway` as Hermes 0.21 has it, waiting on asyncio rather than a thread."""
    module = types.ModuleType("tools.clarify_gateway")
    entries = {}

    def register(clarify_id, session_key, question, choices):
        entries[clarify_id] = SimpleNamespace(
            clarify_id=clarify_id, session_key=session_key, choices=choices, awaiting_text=not choices,
            answered=asyncio.Event(), response=None,
        )

    async def wait_for_response(clarify_id):
        entry = entries[clarify_id]
        await entry.answered.wait()
        entries.pop(clarify_id, None)
        return entry.response

    def get_pending_for_session(session_key, include_choice_prompts=False):
        for entry in entries.values():
            if entry.session_key == session_key and not entry.answered.is_set():
                if include_choice_prompts or entry.awaiting_text:
                    return entry
        return None

    def mark_awaiting_text(clarify_id):
        if clarify_id in entries:
            entries[clarify_id].awaiting_text = True
        return clarify_id in entries

    def resolve_gateway_clarify(clarify_id, response):
        entry = entries.get(clarify_id)
        if entry is None or entry.answered.is_set():
            return False
        entry.response = response
        entry.answered.set()
        return True

    def clear_session(session_key):
        ended = [e for e in entries.values() if e.session_key == session_key and not e.answered.is_set()]
        for entry in ended:
            resolve_gateway_clarify(entry.clarify_id, CANCELLED)
        return len(ended)

    module.register, module.wait_for_response = register, wait_for_response
    module.get_pending_for_session, module.mark_awaiting_text = get_pending_for_session, mark_awaiting_text
    module.resolve_gateway_clarify, module.clear_session = resolve_gateway_clarify, clear_session
    module.SKIPPED = SKIPPED
    return module


class Gateway:
    """A run that asks which weekend is meant, then answers with the forecast."""

    CHOICES = ["Sat Oct 17 + Sun Oct 18 (Recommended)", "Just the next 2 days (Thu/Fri)"]
    # As one question of a batch arrives, Hermes adds its skip line (gateway.clarify.skip_hint).
    QUESTION = "Which days is this weekend for you?"

    def __init__(self, clarify):
        self.clarify = clarify
        self.answers = []
        self.asked = asyncio.Event()
        self.run = None

    async def handle(self, adapter, event):
        if SESSION in adapter._active_sessions:
            await self._inline(adapter, event)
            return
        adapter._active_sessions[SESSION] = object()
        self.run = asyncio.create_task(self._run(adapter, event))

    async def _inline(self, adapter, event):
        if event.get_command() == "stop":
            # Hermes's /stop interrupts the agent, which only notices once the question lets go.
            await adapter.send(event.source.chat_id, "⚡ Stopped.", reply_to=event.message_id)
            return
        entry = self.clarify.get_pending_for_session(SESSION, include_choice_prompts=True)
        if entry is None:
            return
        text = event.text.strip()
        if text.isdigit() and not 1 <= int(text) <= len(entry.choices or []):
            return  # an invalid pick: Hermes keeps the question armed and says nothing
        answer = entry.choices[int(text) - 1] if text.isdigit() else text
        self.clarify.resolve_gateway_clarify(entry.clarify_id, answer)

    async def _run(self, adapter, event):
        await adapter.on_processing_start(event)
        chat = event.source.chat_id
        await adapter.send(chat, "🔎 Checking the forecast source.", reply_to=event.message_id)
        self.clarify.register("c1", SESSION, self.QUESTION, self.CHOICES)
        await adapter.send_clarify(chat, self.QUESTION, self.CHOICES, "c1", SESSION)
        self.asked.set()
        # The heartbeat Hermes sends while the run waits: once, then edited in place.
        beat = await adapter.send(
            chat, "⏳ Working — 3 min — iteration 1/500, clarify", metadata={"_interim_send": True}
        )
        await adapter.edit_message(chat, beat.message_id, "⏳ Working — 6 min — iteration 1/500, clarify")
        answer = await self.clarify.wait_for_response("c1")
        self.answers.append(answer)
        del adapter._active_sessions[SESSION]
        if answer == CANCELLED:
            await adapter.on_processing_complete(event, ProcessingOutcome.CANCELLED)
            return
        await adapter.send(chat, f"Dallas, {answer}: sunny, 29°C.", reply_to=event.message_id)
        await adapter.on_processing_complete(event, ProcessingOutcome.SUCCESS)


class Desktop(hermes.Desktop):
    """The recorded desktop, with the status events and the cards kept apart from the text.

    `cards=False` is a desktop from before `harness.event`, which answers it as an unknown method;
    `refuse` is one that knows it but will not take a question (it keeps no runs).
    """

    def __init__(self, cards=True, refuse=None):
        super().__init__()
        self.cards, self.refuse = cards, refuse
        self.status = {}
        self.requests = {}
        self.polls = []

    async def call(self, method, params, timeout=10.0):
        if method == "harness.event":
            if not self.cards:
                raise ADAPTER.desktop.HarnessError("unknown method `harness.event`; this service speaks: harness.poll")
            turn, event = str(params["turn_id"]), params["event"]
            if event["kind"] == "request":
                if self.refuse:
                    return {"refused": self.refuse}
                self.requests[turn] = self.requests.get(turn, []) + [event]
            else:
                self.status[turn] = self.status.get(turn, []) + [event["text"]]
            return {}
        if method == "harness.poll":
            if not self.polls:
                raise asyncio.CancelledError()
            return self.polls.pop(0)
        return await super().call(method, params, timeout)


async def settle():
    for _ in range(20):
        await asyncio.sleep(0)


def run(script, cards=False, refuse=None, gateway=None):
    clarify = clarify_module()

    async def go():
        harness = hermes.Harness(None)
        harness.desktop = Desktop(cards=cards, refuse=refuse)
        harness.adapter._call = harness.desktop.call
        harness.gateway = harness.adapter.gateway = (gateway or Gateway)(clarify)
        # A run left waiting on its question is the bug itself: fail rather than wait with it.
        await asyncio.wait_for(script(harness), timeout=5)
        await settle()
        return harness

    package = types.ModuleType("tools")
    package.clarify_gateway = clarify
    modules = {name: m for name, m in hermes._hermes_modules().items() if name.startswith("hermes_cli")}
    modules.update({"tools": package, "tools.clarify_gateway": clarify})
    with mock.patch.dict(sys.modules, modules):
        harness = asyncio.run(go())
    harness.clarify = clarify
    return harness


async def asked(h):
    await h.turn(1, "What will the weather be in Dallas this weekend?")
    await h.gateway.asked.wait()
    await settle()


class QuestionTests(unittest.TestCase):
    def test_the_question_completes_the_turn_that_asked_it(self):
        harness = run(asked)
        said = harness.desktop.said["1"]
        self.assertTrue(said.startswith("🔎 Checking the forecast source."), said)
        self.assertIn("❓ Which days is this weekend for you?", said)
        self.assertIn("1. Sat Oct 17 + Sun Oct 18 (Recommended)\n2. Just the next 2 days (Thu/Fri)", said)
        self.assertTrue(said.endswith("Reply with the number, the option text, or your own answer."), said)
        self.assertEqual(harness.desktop.closed["1"], ["harness.complete"], "a finished bubble, closed once")
        self.assertTrue(harness.clarify.get_pending_for_session(SESSION), "a typed answer resolves it")

    def test_the_answer_is_delivered_inline_and_its_turn_stays_open_until_the_forecast_arrives(self):
        async def script(h):
            await asked(h)
            await h.turn(2, "1")
            self.assertEqual(h.gateway.answers, [], "the run has not taken the answer up yet")
            self.assertNotIn("2", h.desktop.closed, "open for the resumed run's answer")
            await h.gateway.run

        harness = run(script)
        self.assertEqual(harness.gateway.answers, ["Sat Oct 17 + Sun Oct 18 (Recommended)"])
        self.assertEqual(harness.desktop.said["2"], "Dallas, Sat Oct 17 + Sun Oct 18 (Recommended): sunny, 29°C.")
        self.assertNotIn("Done.", harness.desktop.said["2"])
        self.assertEqual(harness.desktop.closed["2"], ["harness.complete"], "closed when the run ended")
        self.assertEqual(harness.desktop.closed["1"], ["harness.complete"], "the asking turn is not closed again")
        self.assertEqual(harness.adapter._carried, {})

    def test_an_answer_in_the_persons_own_words_is_taken_too(self):
        async def script(h):
            await asked(h)
            await h.turn(2, "Saturday only")
            await h.gateway.run

        harness = run(script)
        self.assertEqual(harness.desktop.said["2"], "Dallas, Saturday only: sunny, 29°C.")

    def test_a_pick_that_is_not_a_choice_keeps_the_question_and_says_so(self):
        async def script(h):
            await asked(h)
            await h.turn(2, "7")
            await h.turn(3, "2")
            await h.gateway.run

        harness = run(script)
        self.assertIn("not one of the choices", harness.desktop.said["2"])
        self.assertEqual(harness.desktop.closed["2"], ["harness.complete"])
        self.assertEqual(harness.desktop.said["3"], "Dallas, Just the next 2 days (Thu/Fri): sunny, 29°C.")

    def test_stop_with_a_question_pending_ends_the_question_and_the_run(self):
        async def script(h):
            await asked(h)
            await h.turn(2, "/stop")
            await h.gateway.run

        harness = run(script)
        self.assertEqual(harness.gateway.answers, [CANCELLED], "the run is not left waiting an hour")
        self.assertIsNone(harness.clarify.get_pending_for_session(SESSION, include_choice_prompts=True))
        self.assertEqual(harness.desktop.said["2"], "⚡ Stopped.")
        self.assertEqual(harness.desktop.closed["2"], ["harness.complete"])
        self.assertEqual(harness.adapter._asked, {})

    def test_the_working_heartbeat_does_not_reach_the_reply(self):
        harness = run(asked)
        self.assertNotIn("⏳", harness.desktop.said["1"])
        self.assertNotIn("iteration", harness.desktop.said["1"])

    def test_the_heartbeat_is_shown_as_status_without_hermess_internals(self):
        # Kept open for this one: the question would have closed the turn the heartbeat lands on.
        async def script(h):
            h.adapter._ledger.open("5", "desktop", "compare flights")
            beat = await h.adapter.send(
                "desktop", "⏳ Working — 3 min — iteration 4/500, web_search", metadata={"_interim_send": True}
            )
            await h.adapter.edit_message("desktop", beat.message_id, "⏳ Working — 6 min — iteration 9/500, web_search")
            await h.adapter.send("desktop", "Found three fares.")

        harness = run(script, cards=True)
        self.assertEqual(harness.desktop.status["5"], ["Working — 3 min", "Working — 6 min"])
        self.assertEqual(harness.desktop.said["5"], "Found three fares.")

    def test_a_question_with_no_turn_open_is_logged_not_dropped_silently(self):
        async def script(h):
            with self.assertLogs(ADAPTER.logger, level="INFO") as logs:
                h.result = await h.adapter.send_clarify("desktop", "Which city?", None, "c9", SESSION)
            h.logs = logs.output

        harness = run(script)
        self.assertFalse(harness.result.success)
        self.assertTrue(any("Which city?" in line for line in harness.logs), harness.logs)


async def answered_by_poll(h, answer, turn_id=1, request_id="c1"):
    """The desktop's next poll brings the person's answer to the card, as `harness.poll` does."""
    h.desktop.polls.append({"answers": [{"turn_id": turn_id, "request_id": request_id, "answer": answer}]})
    await h.adapter._run()


class CardTests(unittest.TestCase):
    """A desktop that takes `harness.event`: the question is a card, answered inside its own turn."""

    def test_a_choice_arrives_as_a_card_with_its_options_and_the_recommended_one_marked(self):
        harness = run(asked, cards=True)
        self.assertEqual(
            harness.desktop.requests["1"],
            [{
                "kind": "request", "request_id": "c1", "prompt": "Which days is this weekend for you?",
                "options": ["Sat Oct 17 + Sun Oct 18", "Just the next 2 days (Thu/Fri)"], "recommended": 0,
            }],
        )
        self.assertEqual(harness.desktop.said["1"], "🔎 Checking the forecast source.", "no question in the text")
        self.assertNotIn("1", harness.desktop.closed, "the turn stays open for the answer")
        self.assertEqual(harness.adapter._asked, {}, "no later turn is waited for to carry the run")

    def test_pressing_a_choice_resolves_the_clarify_and_the_forecast_lands_in_the_same_turn(self):
        async def script(h):
            await asked(h)
            await answered_by_poll(h, "Sat Oct 17 + Sun Oct 18")
            await h.gateway.run

        harness = run(script, cards=True)
        self.assertEqual(harness.gateway.answers, ["Sat Oct 17 + Sun Oct 18 (Recommended)"], "Hermes's own choice")
        said = harness.desktop.said["1"]
        self.assertTrue(said.startswith("🔎 Checking the forecast source."), said)
        self.assertTrue(said.endswith("Dallas, Sat Oct 17 + Sun Oct 18 (Recommended): sunny, 29°C."), said)
        self.assertEqual(harness.desktop.closed["1"], ["harness.complete"], "closed once, when the run ended")
        self.assertEqual(harness.adapter._cards, {})

    def test_a_free_answer_in_the_card_is_handed_over_as_words(self):
        async def script(h):
            await asked(h)
            await answered_by_poll(h, "Saturday only")
            await h.gateway.run

        harness = run(script, cards=True)
        self.assertTrue(harness.desktop.said["1"].endswith("Dallas, Saturday only: sunny, 29°C."))

    def test_an_answer_to_another_turn_or_question_releases_nothing(self):
        async def script(h):
            await asked(h)
            await answered_by_poll(h, "Sat Oct 17 + Sun Oct 18", turn_id=2)
            await answered_by_poll(h, "Sat Oct 17 + Sun Oct 18", request_id="c2")
            await settle()

        harness = run(script, cards=True)
        self.assertEqual(harness.gateway.answers, [])
        self.assertIn("c1", harness.adapter._cards, "still waiting for its own answer")
        self.assertTrue(harness.clarify.get_pending_for_session(SESSION, include_choice_prompts=True))

    def test_stop_with_a_card_waiting_ends_the_question_and_closes_the_asking_turn(self):
        async def script(h):
            await asked(h)
            await h.turn(2, "/stop")
            await h.gateway.run

        harness = run(script, cards=True)
        self.assertEqual(harness.gateway.answers, [CANCELLED], "the run is not left waiting an hour")
        self.assertEqual(harness.desktop.closed["1"], ["harness.complete"], "the desktop closes its card with it")
        self.assertEqual(harness.desktop.closed["2"], ["harness.complete"])
        self.assertEqual(harness.adapter._cards, {})

    def test_words_sent_as_a_new_message_still_answer_and_the_reply_goes_on_where_it_was_asked(self):
        # A desktop that takes cards but sends typed words as a message (one from before the
        # composer answered cards), or another platform of the person's.
        async def script(h):
            await asked(h)
            await h.turn(2, "Saturday only")
            await h.gateway.run

        harness = run(script, cards=True)
        self.assertIn("answer to its question", harness.desktop.said["2"])
        self.assertEqual(harness.desktop.closed["2"], ["harness.complete"])
        self.assertTrue(harness.desktop.said["1"].endswith("Dallas, Saturday only: sunny, 29°C."))
        self.assertEqual(harness.desktop.closed["1"], ["harness.complete"])
        self.assertEqual(harness.adapter._cards, {})


class OlderDesktopTests(unittest.TestCase):
    """Where a card cannot be shown, the question is #802's text, exactly."""

    def test_a_desktop_without_harness_event_gets_the_question_as_text_and_is_remembered(self):
        harness = run(asked, cards=False)
        self.assertIn("1. Sat Oct 17 + Sun Oct 18 (Recommended)", harness.desktop.said["1"])
        self.assertEqual(harness.desktop.closed["1"], ["harness.complete"])
        self.assertIs(harness.adapter._events, False, "the next question goes as text without asking again")
        self.assertEqual(harness.adapter._cards, {})

    def test_a_desktop_that_refuses_the_card_gets_the_question_as_text(self):
        harness = run(asked, cards=True, refuse="this desktop keeps no runs, so it cannot take a question")
        self.assertIn("❓ Which days is this weekend for you?", harness.desktop.said["1"])
        self.assertEqual(harness.desktop.closed["1"], ["harness.complete"])
        self.assertEqual(harness.adapter._cards, {})


if __name__ == "__main__":
    unittest.main()


class SkippableGateway(Gateway):
    """The same question as Hermes asks it inside a batch, with its skip line."""

    QUESTION = 'Which days is this weekend for you?\nReply "skip" to skip this question.'


class SkipTests(unittest.TestCase):
    """Hermes's skip line on a card: the card's own Skip button, answered with Hermes's SKIPPED."""

    def test_the_skip_line_is_a_button_not_words_the_card_cannot_keep(self):
        harness = run(asked, cards=True, gateway=SkippableGateway)
        request = harness.desktop.requests["1"][0]
        self.assertEqual(request["prompt"], "Which days is this weekend for you?", "no skip line in the card")
        self.assertEqual(request["options"], ["Sat Oct 17 + Sun Oct 18", "Just the next 2 days (Thu/Fri)", "Skip"])
        self.assertEqual(request["recommended"], 0)

    def test_pressing_skip_skips_the_question_the_way_a_typed_skip_does(self):
        async def script(h):
            await asked(h)
            await answered_by_poll(h, "Skip")
            await h.gateway.run

        harness = run(script, cards=True, gateway=SkippableGateway)
        self.assertEqual(harness.gateway.answers, [SKIPPED])

    def test_skip_typed_into_the_card_skips_too(self):
        async def script(h):
            await asked(h)
            await answered_by_poll(h, " skip ")
            await h.gateway.run

        harness = run(script, cards=True, gateway=SkippableGateway)
        self.assertEqual(harness.gateway.answers, [SKIPPED])

    def test_a_question_hermes_did_not_say_could_be_skipped_offers_no_skip(self):
        harness = run(asked, cards=True)
        self.assertNotIn("Skip", harness.desktop.requests["1"][0]["options"])
