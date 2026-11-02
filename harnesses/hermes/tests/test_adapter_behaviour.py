"""The adapter's behaviour against a fake gateway and a fake desktop: no Hermes needed.

    python3 -m unittest discover -s harnesses/hermes/tests -p test_adapter_behaviour.py

Hermes is third-party, so what the adapter needs of it is stood in for here, the smallest thing
that lets the real adapter code run: `gateway.*` (the base platform, the session key), the
`tools.clarify_gateway` a question is parked in, and the desktop's socket (`desktop.call`).
What is checked is what the battery of 2 Nov found (#1070): a question nobody answers must not hold
a turn, a result that arrives with no conversation open must not be thrown away, and what Hermes
is told about the desktop steers it to the person's own apps.
"""

import asyncio
import importlib.util
import sys
import types
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional

PLUGIN = Path(__file__).resolve().parents[1]


class Clarify:
    """`tools.clarify_gateway`: what a parked question is resolved with."""

    SKIPPED = "SKIPPED"

    def __init__(self):
        self.resolved: List[tuple] = []
        self.cleared: List[str] = []
        self.parked = True

    def resolve_gateway_clarify(self, clarify_id, answer):
        self.resolved.append((clarify_id, answer))
        return self.parked

    def clear_session(self, session_key):
        self.cleared.append(session_key)
        return 1

    def mark_awaiting_text(self, clarify_id):
        pass

    def get_pending_for_session(self, session_key, include_choice_prompts=False):
        return None


CLARIFY = Clarify()


def _install_fake_hermes():
    @dataclass
    class SendResult:
        success: bool = True
        message_id: Optional[str] = None
        error: Optional[str] = None

    class MessageType:
        TEXT = "text"

    class ProcessingOutcome:
        SUCCESS = "success"
        CANCELLED = "cancelled"
        FAILURE = "failure"

    @dataclass
    class MessageEvent:
        text: str = ""
        message_type: Any = None
        source: Any = None
        message_id: Optional[str] = None
        timestamp: Any = None

        def get_command(self):
            return None

    class BasePlatformAdapter:
        def __init__(self, config=None, platform=None):
            self.config = config
            self.platform = platform
            self._active_sessions = set()
            self.handled: List[Any] = []

        def _mark_disconnected(self):
            pass

        def _mark_connected(self):
            pass

        def build_source(self, **kw):
            return types.SimpleNamespace(**kw)

        async def handle_message(self, event):
            self.handled.append(event)

    class PlatformConfig:
        def __init__(self):
            self.extra = {}

    modules = {
        "gateway": types.ModuleType("gateway"),
        "gateway.config": types.ModuleType("gateway.config"),
        "gateway.platforms": types.ModuleType("gateway.platforms"),
        "gateway.platforms.base": types.ModuleType("gateway.platforms.base"),
        "gateway.session": types.ModuleType("gateway.session"),
        "tools": types.ModuleType("tools"),
        "tools.clarify_gateway": CLARIFY,
        "hermes_cli": types.ModuleType("hermes_cli"),
        "hermes_cli.commands": types.ModuleType("hermes_cli.commands"),
    }
    modules["gateway.config"].Platform = lambda name: name
    modules["gateway.config"].PlatformConfig = PlatformConfig
    base = modules["gateway.platforms.base"]
    base.BasePlatformAdapter, base.MessageEvent, base.MessageType = BasePlatformAdapter, MessageEvent, MessageType
    base.ProcessingOutcome, base.SendResult = ProcessingOutcome, SendResult
    modules["gateway.session"].build_session_key = lambda source, **kw: "session-1"
    modules["hermes_cli.commands"].should_bypass_active_session = lambda command: False
    modules["tools"].clarify_gateway = CLARIFY
    saved = {name: sys.modules.get(name) for name in modules}
    sys.modules.update(modules)
    return saved


def _load():
    spec = importlib.util.spec_from_file_location(
        "yantrik_fake_gateway_plugin", PLUGIN / "__init__.py", submodule_search_locations=[str(PLUGIN)]
    )
    package = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = package
    spec.loader.exec_module(package)
    from yantrik_fake_gateway_plugin import adapter, desktop

    return adapter, desktop


adapter_mod = desktop_mod = questions_mod = output_mod = None
_SAVED: Dict[str, Any] = {}


def setUpModule():
    """Hermes's names are faked for this file only: the contract test beside it wants the real ones."""
    global adapter_mod, desktop_mod, questions_mod, output_mod
    _SAVED.update(_install_fake_hermes())
    adapter_mod, desktop_mod = _load()
    questions_mod = sys.modules["yantrik_fake_gateway_plugin.questions"]
    output_mod = sys.modules["yantrik_fake_gateway_plugin.output"]


def tearDownModule():
    for name, module in _SAVED.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module
    for name in [n for n in sys.modules if n.startswith("yantrik_fake_gateway_plugin")]:
        del sys.modules[name]


class FakeDesktop:
    """The desktop's harness socket: records every call, answers the way it does."""

    def __init__(self):
        self.calls: List[tuple] = []

    def __call__(self, address, method, params, timeout=10.0):
        self.calls.append((method, params))
        if method == desktop_mod.ATTACH:
            return {"session": "s1"}
        return {}

    def of(self, method) -> List[Dict[str, Any]]:
        return [p for m, p in self.calls if m == method]

    def said(self) -> str:
        return "".join(p.get("delta", "") for p in self.of(desktop_mod.CHUNK))


class AdapterCase(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.fake = FakeDesktop()
        self._saved = (desktop_mod.call, questions_mod.QUESTION_SECONDS)
        desktop_mod.call = self.fake
        questions_mod.QUESTION_SECONDS = 0.05
        CLARIFY.resolved.clear()
        CLARIFY.cleared.clear()
        CLARIFY.parked = True
        self.adapter = adapter_mod.YantrikAdapter(sys.modules["gateway.config"].PlatformConfig())
        self.adapter._session = "s1"
        self.adapter._address = "/nowhere"
        # Hermes's session store: which session id the Lens chat's key holds now.
        self.store_ids = {"session-1": "20261102_101500_a1b2c3"}
        self.adapter._session_store = types.SimpleNamespace(peek_session_id=lambda key: self.store_ids.get(key))

    def tearDown(self):
        desktop_mod.call, questions_mod.QUESTION_SECONDS = self._saved

    def open_turn(self, turn_id="7"):
        turn = self.adapter._ledger.open(turn_id, adapter_mod.CHAT_ID, "Add a dentist appointment next Thursday")
        turn.session_key = "session-1"
        self.adapter._ledger.mark_started(turn_id)
        return turn

    async def ask(self, clarify_id="c1"):
        return await self.adapter.send_clarify(
            adapter_mod.CHAT_ID, "Which Thursday?", ["Nov 5", "Nov 12 (Recommended)"], clarify_id, "session-1"
        )


class ClarifyTests(AdapterCase):
    async def test_a_question_is_a_card_with_its_choices_in_the_turn_that_asked(self):
        self.open_turn()
        await self.ask()
        events = [p["event"] for p in self.fake.of(desktop_mod.EVENT)]
        self.assertEqual(events[0]["kind"], "request")
        self.assertEqual(events[0]["options"], ["Nov 5", "Nov 12"])
        self.assertEqual(events[0]["recommended"], 1)
        self.assertEqual(self.fake.of(desktop_mod.EVENT)[0]["turn_id"], 7)

    async def test_an_answer_in_time_goes_back_into_the_turn_and_nothing_expires_later(self):
        self.open_turn()
        await self.ask()
        self.adapter._take_answers([{"request_id": "c1", "turn_id": "7", "answer": "Nov 12"}])
        self.assertEqual(CLARIFY.resolved, [("c1", "Nov 12 (Recommended)")])
        await asyncio.sleep(0.2)
        self.assertEqual(len(CLARIFY.resolved), 1, "the answer was given; the timer must not answer again")

    async def test_a_question_nobody_answers_lets_go_of_the_turn_and_says_so(self):
        self.open_turn()
        await self.ask()
        await asyncio.sleep(0.25)
        self.assertEqual(len(CLARIFY.resolved), 1)
        clarify_id, told = CLARIFY.resolved[0]
        self.assertEqual(clarify_id, "c1")
        self.assertIn("did not answer", told)
        self.assertIn("Do not ask the same question again", told)
        self.assertIn("No answer to the question", self.fake.said())
        self.assertNotIn("c1", self.adapter._cards)
        self.assertEqual(self.fake.of(desktop_mod.COMPLETE), [], "the run goes on in the turn: not closed yet")

    async def test_a_second_unanswered_question_ends_the_turn_truthfully(self):
        self.open_turn()
        await self.ask("c1")
        await asyncio.sleep(0.25)
        await self.ask("c2")
        await asyncio.sleep(0.25)
        self.assertEqual(len(CLARIFY.resolved), 1, "only the first is handed a 'no answer'")
        self.assertEqual(CLARIFY.cleared, ["session-1"], "the second drops the question Hermes waits on")
        self.assertIn("stopped here", self.fake.said())
        self.assertEqual(len(self.fake.of(desktop_mod.COMPLETE)), 1)
        self.assertEqual(self.adapter._ledger.open_turns(), [])

    async def test_a_question_hermes_already_dropped_is_cleared_not_left(self):
        self.open_turn()
        CLARIFY.parked = False
        await self.ask()
        await asyncio.sleep(0.25)
        self.assertEqual(CLARIFY.cleared, ["session-1"])

    async def test_a_desktop_without_cards_ends_the_asking_turn_so_nothing_is_blocked(self):
        self.adapter._events = False
        self.open_turn()
        await self.ask()
        self.assertIn("Which Thursday?", self.fake.said())
        self.assertEqual(len(self.fake.of(desktop_mod.COMPLETE)), 1)


class BackgroundResultTests(AdapterCase):
    async def test_a_result_with_no_conversation_open_is_kept_not_thrown_away(self):
        result = await self.adapter.send(adapter_mod.CHAT_ID, "Saturday: 14 / 9 degrees, dry.")
        self.assertTrue(result.success)
        self.assertEqual(self.fake.of(desktop_mod.CHUNK), [])

    async def test_it_is_shown_labelled_at_the_top_of_the_next_reply_once(self):
        await self.adapter.send(adapter_mod.CHAT_ID, "Saturday: 14 / 9 degrees, dry.")
        turn = self.open_turn("8")
        await self.adapter.send(adapter_mod.CHAT_ID, "Here is your calendar.")
        said = self.fake.said()
        self.assertIn(output_mod.LATE_HEADING, said)
        self.assertIn("Saturday: 14 / 9 degrees, dry.", said)
        self.assertLess(said.index("Saturday"), said.index("Here is your calendar."))
        await self.adapter.send(adapter_mod.CHAT_ID, "More.")
        self.assertEqual(self.fake.said().count("Saturday"), 1)
        self.assertTrue(turn.said_anything)

    async def test_a_status_line_with_no_turn_is_still_just_dropped(self):
        result = await self.adapter.send(adapter_mod.CHAT_ID, "⏳ Working — 3 min", metadata={"_interim_send": True})
        self.assertFalse(result.success)
        self.assertEqual(self.adapter._late, [])

    async def test_what_is_kept_is_bounded(self):
        for i in range(output_mod.LATE_KEPT + 5):
            await self.adapter.send(adapter_mod.CHAT_ID, f"result {i} " + "x" * 10000)
        self.assertEqual(len(self.adapter._late), output_mod.LATE_KEPT)
        self.assertTrue(all(len(m) <= output_mod.LATE_CHARS for m in self.adapter._late))


class LensProofTests(AdapterCase):
    def tearDown(self):
        adapter_mod.LENS.stop()
        adapter_mod._ADAPTERS[:] = []
        super().tearDown()

    async def test_a_turn_carries_its_id_into_hermes_and_gives_the_lens_bridge_the_desktops_token(self):
        await self.adapter._on_turn({"turn_id": "9", "text": "hi", "agent_token": "tok-lens-1"})
        self.assertEqual(self.adapter.handled[0].source.message_id, "9")
        self.assertTrue(adapter_mod.LENS.token_current)
        adapter_mod._ADAPTERS[:] = [self.adapter]
        standing = adapter_mod.lens_standing()
        self.assertEqual(standing.lens_key, "session-1")
        self.assertEqual(standing.session_id, "20261102_101500_a1b2c3")
        self.assertEqual(standing.open_ids, frozenset({"9"}))
        self.assertTrue(standing.token_current)

    async def test_the_lens_session_id_follows_the_store_so_a_new_conversation_is_not_the_old_one(self):
        await self.adapter._on_turn({"turn_id": "9", "text": "hi", "agent_token": "tok-lens-1"})
        self.store_ids["session-1"] = "20261102_103000_ffeedd"  # the person's +: Hermes begins a new session
        self.assertEqual(self.adapter.standing().session_id, "20261102_103000_ffeedd")

    async def test_a_store_that_cannot_say_leaves_no_call_proven(self):
        await self.adapter._on_turn({"turn_id": "9", "text": "hi", "agent_token": "tok-lens-1"})
        self.adapter._session_store = None
        self.assertEqual(self.adapter.standing().session_id, "")

    async def test_a_turn_with_no_token_leaves_the_lens_bridge_without_one(self):
        await self.adapter._on_turn({"turn_id": "9", "text": "hi"})
        self.assertFalse(adapter_mod.LENS.token_current)
        self.assertFalse(self.adapter.standing().token_current)

    async def test_a_closed_turn_is_no_longer_the_lenss(self):
        await self.adapter._on_turn({"turn_id": "9", "text": "hi", "agent_token": "tok-lens-1"})
        await self.adapter._finish("9")
        self.assertEqual(self.adapter.standing().open_ids, frozenset())

    async def test_a_run_that_a_later_turn_carries_is_still_known_by_the_turn_that_started_it(self):
        await self.adapter._on_turn({"turn_id": "9", "text": "hi", "agent_token": "tok-lens-1"})
        self.adapter._ledger.open("10", adapter_mod.CHAT_ID, "Nov 12")
        self.adapter._carried["9"] = "10"
        await self.adapter._finish("9")
        self.assertEqual(self.adapter.standing().open_ids, frozenset({"9", "10"}))

    async def test_losing_the_desktop_or_disconnecting_drops_the_token_and_the_bridge(self):
        await self.adapter._on_turn({"turn_id": "9", "text": "hi", "agent_token": "tok-lens-1"})
        await self.adapter.disconnect()
        self.assertFalse(adapter_mod.LENS.token_current)


class OfferTests(unittest.TestCase):
    """The platform's list follows what this process registered, and is asked for as the gateway connects."""

    def setUp(self):
        self.tools = sys.modules["yantrik_fake_gateway_plugin.lens_tools"]
        self._saved = list(self.tools.REGISTERED)
        self.addCleanup(lambda: self.tools.REGISTERED.__setitem__(slice(None), self._saved))

    def test_proxies_registered_means_the_list_is_turned_on(self):
        self.tools.REGISTERED[:] = ["os_apps"]
        asked = []
        self.assertTrue(self.tools.offer_to_platform(lambda on: asked.append(on) or True))
        self.assertEqual(asked, [True])

    def test_none_registered_means_the_plain_list_is_put_back(self):
        self.tools.REGISTERED[:] = []
        asked = []
        self.tools.offer_to_platform(lambda on: asked.append(on) or True)
        self.assertEqual(asked, [False])

    def test_a_failure_to_change_the_list_is_not_an_error_in_the_gateway(self):
        def boom(on):
            raise OSError("read-only")

        self.assertFalse(self.tools.offer_to_platform(boom))

    def test_plugin_load_alone_never_touches_the_list(self):
        # Any `hermes` command loads plugins; a CLI run while an older gateway is still up (after an
        # update, before its restart) must not name tools that gateway does not have.
        asked = []
        saved = self.tools.set_platform_list
        self.tools.set_platform_list = lambda on: asked.append(on) or True
        self.addCleanup(setattr, self.tools, "set_platform_list", saved)
        ctx = types.SimpleNamespace(register_tool=lambda **kw: None)
        self.tools.install(ctx, lambda: None, adapter_mod.LENS)
        self.assertEqual(asked, [])


class ConnectOffersTests(AdapterCase):
    async def test_the_gateways_connect_asks_for_the_list_that_matches_what_it_registered(self):
        tools = sys.modules["yantrik_fake_gateway_plugin.lens_tools"]
        asked = []
        saved = (tools.set_platform_list, list(tools.REGISTERED))
        tools.set_platform_list = lambda on: asked.append(on) or True
        tools.REGISTERED[:] = ["os_apps"]

        def restore():
            tools.set_platform_list = saved[0]
            tools.REGISTERED[:] = saved[1]

        self.addCleanup(restore)
        self.addCleanup(adapter_mod._ADAPTERS.clear)
        await self.adapter.connect()
        self.assertEqual(asked, [True])
        await self.adapter.disconnect()


class SteeringTests(unittest.TestCase):
    def registered_hint(self):
        seen = {}
        ctx = types.SimpleNamespace(register_platform=lambda **kw: seen.update(kw))
        adapter_mod.register(ctx)
        return seen["platform_hint"]

    def test_the_hint_sends_hermes_to_the_persons_own_apps_before_memory_the_web_or_a_terminal(self):
        hint = self.registered_hint()
        for needle in ("os_apps", "`for`", "`opens`", "`describe_as`", "os_describe", "os_act",
                       "'My weather'", "Weather", "not your memory"):
            self.assertIn(needle, hint)
        self.assertLess(hint.index("os_apps"), hint.index("terminal"))
        self.assertIn("Use the terminal only when no app has the action", hint)

    def test_the_hint_never_offers_to_skip_the_persons_approval(self):
        self.assertIn("Never offer a way to run things that skips their approval", self.registered_hint())

    def test_the_hint_stops_hermes_promising_what_the_desktop_cannot_deliver_unasked(self):
        hint = self.registered_hint()
        self.assertIn("never promise to 'report back'", hint)
        self.assertIn("wait for the result inside this turn", hint)

    def test_the_hint_says_what_to_do_when_a_question_goes_unanswered(self):
        self.assertIn("do not ask again", self.registered_hint())


if __name__ == "__main__":
    unittest.main()
