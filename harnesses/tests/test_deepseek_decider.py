"""The decider in front of the DeepSeek loop, against a fake decision model.

Offline, like the rest of `harnesses/tests`: the fake System One server from `fake_systemone`
answers both wire formats, and the fake chat server from `test_deepseek` is the same one the
generative loop is tested against — the point of these tests is what the two of them do
together, so neither is replaced by a stub.

Two tripwire keys, because with a decider there are two, and both leak the same way.
"""

import json
import os
import tempfile
import threading
import unittest
from http.server import ThreadingHTTPServer

import fake_systemone
import support  # noqa: F401 — puts the harness on sys.path
from support import recording_turn, said
from test_deepseek import TRIPWIRE, _Chat

from yantrik_deepseek import (Ask, Config, ConfigError, Decide, DeciderConfig, DeepSeekMind,
                              NONE_OF_THESE, Pick, SystemOne, World, actions_from_describe,
                              apps_from_listing, load_config, pinned_tools, questions_for,
                              read_picks, world_from_messages)

DECIDER_TRIPWIRE = "ts-tripwire-zyxwvutsrqponmlkjihg"

LISTING = """Open now (describe these by name):
    calendar           Calendar — September 2026, nothing on day 21
    terminal           Terminal — /home/yantrik, shell idle at a prompt
Services answering: weather, notifications
    a service shares a name with its app (weather, calendar, email); the app is the
    window, and `describe <name>` means the window whenever it is open.
Can be opened with `act shell open_app name=<name>`:
    notes          quick markdown notes kept in the notes library
    sysmonitor     CPU, memory, disk and processes  (then describe system-monitor)
Screens of the desktop itself (also open_app): files, settings
"""

DESCRIBE = """Calendar — September 2026, nothing on day 21
revision: 75cc6e46b1f56ef9
{
  "events_this_month": 3,
  "selected_day": 21
}
  act: select_day(day)  [standard, settles on return]
       Show what is on one day of the month shown
         day: number — Day of the month, 1-31
  act: add_event(date, time, title, all_day?)  [standard, settles on return]
       Put something on the calendar
         date: string — YYYY-MM-DD
         time: string — HH:MM, 24-hour
  act: delete_event(date?, id?, title?)  [sensitive, settles on return]
       Take an event off the calendar. It is not recoverable
         id?: string — The id the store gave the event
"""


class DesktopTools:
    """The three tools the decider cares about, with the schemas `yos-mcp` publishes.

    `os_act` has to take an `app` and an `action` for there to be anything to pin, so this fake
    carries the real shape rather than an empty object.
    """

    def __init__(self, refuse=False):
        self.calls = []
        self.refuse = refuse
        # A desktop whose `os_apps` answers nothing a listing can be read out of, and one that
        # publishes no tools at all: the two ways the catalogue can stay empty.
        self.empty = False
        self.schemas = None

    def as_openai_tools(self):
        if self.schemas is not None:
            return self.schemas
        return [
            {"type": "function", "function": {
                "name": "os_apps", "description": "What is on this desktop.",
                "parameters": {"type": "object", "properties": {}}}},
            {"type": "function", "function": {
                "name": "os_describe", "description": "The state of one app.",
                "parameters": {"type": "object",
                               "properties": {"app": {"type": "string"}},
                               "required": ["app"]}}},
            {"type": "function", "function": {
                "name": "os_act", "description": "Perform an action an app lists.",
                "parameters": {"type": "object",
                               "properties": {"app": {"type": "string"},
                                              "action": {"type": "string"},
                                              "args": {"type": "object",
                                                       "additionalProperties": True}},
                               "required": ["app", "action"]}}},
        ]

    def call(self, name, arguments, timeout=None):
        self.calls.append((name, arguments))
        if name == "os_apps":
            if self.empty:
                return ("the desktop's app listing is not answering", True)
            return (LISTING, False)
        if name == "os_describe":
            return (DESCRIBE, False)
        if self.refuse:
            # A policy answer: it ran, it was healthy, it said no. Unflagged, like the bridge's.
            return ("REFUSED: plan mode is on, so nothing was changed.", False)
        return ("done: %s.%s" % (arguments.get("app"), arguments.get("action")), False)


def seen_desktop():
    """A conversation in which `os_apps` and `os_describe calendar` have already been answered.

    The apps and their actions are facts about the desktop, so the harness keeps them across
    requests. Seeding them is how a test gets at the step that matters — the one where there is
    something to pick — without driving three steps to reach it.
    """
    return [
        {"role": "user", "content": "what is on my calendar?"},
        {"role": "assistant", "content": "", "tool_calls": [
            {"id": "c1", "type": "function",
             "function": {"name": "os_apps", "arguments": "{}"}},
            {"id": "c2", "type": "function",
             "function": {"name": "os_describe", "arguments": '{"app": "calendar"}'}}]},
        {"role": "tool", "tool_call_id": "c1", "name": "os_apps", "content": LISTING},
        {"role": "tool", "tool_call_id": "c2", "name": "os_describe", "content": DESCRIBE},
        {"role": "assistant", "content": "Three events this month."},
    ]


class DeciderLoopTests(unittest.TestCase):
    """The loop with a decider in front of it, end to end over two fake servers."""

    @classmethod
    def setUpClass(cls):
        # The chat server is the one the generative loop is tested against, run again here: what
        # these tests are about is the two halves together, so neither half is a stub.
        cls.chat = ThreadingHTTPServer(("127.0.0.1", 0), _Chat)
        cls.chat.lock = threading.Lock()
        cls.chat.requests = []
        threading.Thread(target=cls.chat.serve_forever, daemon=True).start()
        cls.chat_base = "http://127.0.0.1:%d/v1" % cls.chat.server_address[1]
        cls.brain = fake_systemone.start()
        cls.brain_base = "http://127.0.0.1:%d" % cls.brain.server_address[1]

    @classmethod
    def tearDownClass(cls):
        for server in (cls.chat, cls.brain):
            server.shutdown()
            server.server_close()

    def setUp(self):
        with self.chat.lock:
            self.chat.requests = []
        with self.brain.lock:
            self.brain.requests = []
            self.brain.scenario = "confident"
        self.logged = []

    # ── driving ─────────────────────────────────────────────────────────

    def mind(self, model, kind="jev", scenario=None, base_url=None, seeded=True, refuse=False,
             **kwargs):
        if scenario:
            with self.brain.lock:
                self.brain.scenario = scenario
        decider = None
        if kind:
            decider = DeciderConfig(kind=kind, base_url=base_url or self.brain_base,
                                    api_key=DECIDER_TRIPWIRE, gate=0.9)
        config = Config(base_url=self.chat_base, model=model, api_key=TRIPWIRE,
                        decider=decider, **kwargs)
        mind = DeepSeekMind(config, DesktopTools(refuse), log=self.logged.append)
        if seeded:
            mind.messages = seen_desktop()
        return mind

    def chat_requests(self, model):
        with self.chat.lock:
            return [b for b in self.chat.requests if b.get("model") == model]

    def decisions(self):
        with self.brain.lock:
            return list(self.brain.requests)

    def log(self):
        return "\n".join(self.logged)

    # ── the config is absent ────────────────────────────────────────────

    def test_no_decider_block_means_the_loop_is_exactly_what_it_was(self):
        mind = self.mind("tools", kind=None)
        turn, recorder = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        self.assertEqual(self.decisions(), [], "nothing should have been asked of a decider")
        sent = self.chat_requests("tools")[0]
        self.assertEqual([t["function"]["name"] for t in sent["tools"]],
                         ["os_apps", "os_describe", "os_act"])
        self.assertNotIn("tool_choice", sent)
        self.assertIn("Added it.", said(recorder))
        self.assertEqual(self.log(), "")

    def test_the_harness_reads_the_app_list_itself_so_the_first_step_can_be_decided(self):
        # Measured live: the model answered "what is on my calendar?" with os_describe and never
        # called os_apps, so the decider had no candidates and was never asked. Whether the
        # cheap half of the loop runs cannot depend on the generator's habits.
        mind = self.mind("tools", seeded=False, max_steps=1)
        turn, recorder = recording_turn("what is on my calendar on 25 September?")
        mind.answer(turn)
        self.assertEqual(mind.tools.calls[0], ("os_apps", {}))
        self.assertEqual(self.catalogue_reads(mind), 1)
        self.assertEqual(len(self.decisions()), 1, "the first step was decided, not skipped")
        # It goes into the conversation as an ordinary call and its answer, so the generator
        # sees it too and does not have to ask again.
        self.assertEqual([m["role"] for m in mind.messages[:3]], ["user", "assistant", "tool"])
        self.assertIn("Open now", mind.messages[2]["content"])
        self.assertIn("⚙️ os_apps", said(recorder), "a tool call the person cannot see is worse")

    def test_the_app_list_is_read_once_and_not_again_when_it_is_already_known(self):
        mind = self.mind("tools")  # seeded: os_apps is already in the conversation
        turn, _ = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        self.assertEqual(self.catalogue_reads(mind), 0, "the catalogue was already there")

    @staticmethod
    def catalogue_reads(mind):
        """How many times the harness read the app list on its own behalf."""
        return sum(1 for m in mind.messages
                   if str(m.get("tool_call_id") or "").startswith("catalogue_"))

    def test_the_live_turn_that_found_this_is_decided_from_end_to_end(self):
        """"What is on my calendar on 25 September?", against a model that skips os_apps.

        The journal this leaves is the whole point: the app list read once, a first step that
        says what it could not pin and why, and a second step where the decider ends the loop
        because the app has been read and the question is answered.
        """
        mind = self.mind("describe-first", scenario="reading", seeded=False)
        turn, recorder = recording_turn("what is on my calendar on 25 September?")
        mind.answer(turn)
        self.assertEqual([name for name, _ in mind.tools.calls], ["os_apps", "os_describe"])
        self.assertEqual(len(self.decisions()), 2, "both steps were decided")
        self.assertIn("nothing to pin on calendar: it has not been described yet",
                      self.logged[0])
        self.assertIn("the generator picked os_describe calendar", self.logged[0])
        self.assertIn("answer — no app", self.logged[1])
        self.assertIn("done? yes 0.96", self.logged[1])
        # The last step was the decider's: no tools were offered, so the model wrote the reply.
        self.assertNotIn("tools", self.chat_requests("describe-first")[1])
        self.assertIn("Nothing on the 25th.", said(recorder))

    def test_nothing_is_read_for_a_loop_that_has_no_decider(self):
        mind = self.mind("plain", kind=None, seeded=False)
        turn, _ = recording_turn("hello")
        mind.answer(turn)
        self.assertEqual(mind.tools.calls, [])

    # ── the decider picks, the generator fills ──────────────────────────

    def test_the_decider_picks_and_the_generator_only_fills_the_arguments(self):
        mind = self.mind("fills")
        turn, recorder = recording_turn("put dentist on friday at 3")
        mind.answer(turn)

        # One request per step, with every question of that step in it. Two steps ran: the one
        # that acted, and the one that said what had been done.
        self.assertEqual([path for path, _ in self.decisions()],
                         ["/v1/systemone", "/v1/systemone"])
        body = self.decisions()[0][1]
        self.assertEqual(set(body["questions"]), {"done", "app", "action:calendar"})
        self.assertEqual(body["model"], "jev-latest")

        # The step went out pinned: one tool, os_act, its app and action already decided.
        sent = self.chat_requests("fills")[0]
        self.assertEqual([t["function"]["name"] for t in sent["tools"]], ["os_act"])
        self.assertEqual(sent["tool_choice"],
                         {"type": "function", "function": {"name": "os_act"}})
        properties = sent["tools"][0]["function"]["parameters"]["properties"]
        self.assertEqual(properties["app"]["enum"], ["calendar"])
        self.assertEqual(properties["action"]["enum"], ["add_event"])
        self.assertEqual(properties["args"]["type"], "object")

        # And the action that ran is the decider's, with the generator's arguments.
        self.assertEqual(mind.tools.calls[0],
                         ("os_act", {"app": "calendar", "action": "add_event",
                                     "args": {"title": "Dentist", "date": "2026-09-25"}}))
        self.assertIn("Put it on the calendar.", said(recorder))

    def test_a_generator_that_writes_its_own_app_is_overruled_and_the_log_says_so(self):
        mind = self.mind("fills")
        turn, _ = recording_turn("put dentist on friday at 3")
        mind.answer(turn)
        self.assertIn("act calendar.add_event", self.log())
        self.assertIn("gate 0.90 held", self.log())
        self.assertIn("the generator wrote os_act notes.new_note and was overruled", self.log())

    def test_the_log_line_carries_the_pick_the_probability_and_the_latency(self):
        mind = self.mind("fills")
        turn, _ = recording_turn("put dentist on friday at 3")
        mind.answer(turn)
        first = [line for line in self.logged if line.startswith("decider step 1")]
        self.assertEqual(len(first), 1, self.log())
        line = first[0]
        self.assertIn("p=0.97", line)
        self.assertIn("app calendar 0.97", line)
        self.assertIn("action add_event 0.97", line)
        self.assertIn("done? no 0.96", line)
        self.assertRegex(line, r"\d+ms")

    # ── below the gate ──────────────────────────────────────────────────

    def test_below_the_gate_the_generator_picks_and_the_disagreement_is_logged(self):
        mind = self.mind("tools", scenario="unsure")
        turn, recorder = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        sent = self.chat_requests("tools")[0]
        # The whole tool list, nothing pinned: the fallback is the loop as it was.
        self.assertEqual([t["function"]["name"] for t in sent["tools"]],
                         ["os_apps", "os_describe", "os_act"])
        self.assertNotIn("tool_choice", sent)
        line = [l for l in self.logged if l.startswith("decider step 1")][0]
        self.assertIn("would have acted on calendar.add_event p=0.62", line)
        self.assertIn("gate 0.90 not held", line)
        self.assertIn("the generator picked os_apps", line)
        self.assertIn("Added it.", said(recorder))

    def test_a_choice_with_no_probabilities_cannot_be_gated_so_the_generator_picks(self):
        # `confidence` is a rescaled statistic and not the probability of anything. Without the
        # distribution there is nothing a gate can honestly compare.
        mind = self.mind("tools", scenario="no-spread")
        turn, _ = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        self.assertNotIn("tool_choice", self.chat_requests("tools")[0])
        self.assertIn("it did not answer which app", self.log())

    def test_a_decider_that_cannot_be_reached_leaves_the_loop_alone(self):
        mind = self.mind("tools", base_url="http://127.0.0.1:1")
        turn, recorder = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        self.assertIn("could not reach", self.log())
        self.assertIn("the generator picks this step", self.log())
        self.assertIn("Added it.", said(recorder))

    def test_a_decider_that_refuses_the_questions_leaves_the_loop_alone(self):
        mind = self.mind("tools", scenario="refuse")
        turn, recorder = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        self.assertIn("refused the questions (422)", self.log())
        self.assertIn("Added it.", said(recorder))

    def test_an_option_that_was_never_offered_is_not_acted_on(self):
        mind = self.mind("tools", scenario="echo-key")
        turn, _ = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        self.assertNotIn("tool_choice", self.chat_requests("tools")[0])

    # ── the loop ends ───────────────────────────────────────────────────

    def test_none_of_these_ends_the_loop_with_a_plain_answer(self):
        mind = self.mind("plain", scenario="none")
        turn, recorder = recording_turn("what do you think of Tuesday?")
        mind.answer(turn)
        sent = self.chat_requests("plain")
        self.assertEqual(len(sent), 1, "one request, and no tool call in it")
        self.assertNotIn("tools", sent[0])
        self.assertNotIn("tool_choice", sent[0])
        self.assertEqual(mind.tools.calls, [])
        self.assertEqual(said(recorder), "Two windows.")
        self.assertIn("answer — no app", self.log())

    def test_a_request_the_decider_calls_finished_ends_the_loop_too(self):
        mind = self.mind("plain", scenario="done")
        turn, recorder = recording_turn("did that work?")
        mind.answer(turn)
        self.assertNotIn("tools", self.chat_requests("plain")[0])
        self.assertIn("done? yes 0.96", self.log())
        self.assertEqual(said(recorder), "Two windows.")

    def test_a_refusal_stands_the_decider_down_for_the_rest_of_the_turn(self):
        # A REFUSED result is the desktop declining an action, and the one thing a mind must not
        # do then is look for another route to the same thing. That judgement is in the system
        # prompt, which the generator reads and the decider is not shown.
        mind = self.mind("tools", refuse=True)
        turn, _ = recording_turn("delete friday's event")
        mind.answer(turn)
        self.assertEqual(len(self.decisions()), 1,
                         "asked on the first step, and not again after the refusal")
        self.assertIn("decider step 2 stands aside: a refusal is standing", self.log())

    # ── standing aside is said out loud ─────────────────────────────────

    def test_a_step_that_was_never_asked_says_so_with_its_reason(self):
        # The difference a person reading the journal has to be able to see: "asked and fell
        # back" is one thing, "never asked at all" is another, and a quiet return is neither.
        mind = self.mind("plain", seeded=False)
        mind.tools.empty = True  # the read was tried and the desktop had nothing to say
        turn, _ = recording_turn("hello")
        mind.answer(turn)
        self.assertEqual(mind.tools.calls, [("os_apps", {})])
        self.assertIn("decider step 1 stands aside: no app catalogue yet", self.log())
        self.assertEqual(self.decisions(), [])

    def test_a_desktop_with_no_tools_at_all_says_that_is_why(self):
        mind = self.mind("plain")
        mind.tools.schemas = []
        turn, _ = recording_turn("hello")
        mind.answer(turn)
        self.assertIn("decider step 1 stands aside: the desktop's tools are unavailable",
                      self.log())

    def test_an_app_with_no_action_list_says_which_of_the_three_reasons_it_is(self):
        mind = self.mind("tools", scenario="other")  # picks `terminal`, which is not described
        turn, _ = recording_turn("show me the log")
        mind.answer(turn)
        self.assertIn("nothing to pin on terminal: it has not been described yet", self.log())

    # ── the keys ────────────────────────────────────────────────────────

    def test_neither_key_appears_in_anything_this_module_produces(self):
        leaked = []
        for scenario, model in (("echo-key", "tools"), ("refuse", "tools"),
                                ("confident", "fills")):
            mind = self.mind(model, scenario=scenario)
            turn, recorder = recording_turn("add dentist to my calendar")
            try:
                mind.answer(turn)
            except Exception as exc:  # noqa: BLE001 — the message is what is on trial
                leaked.append(str(exc))
            leaked.append(said(recorder))
            leaked.append(repr(mind))
            leaked.append(repr(mind.config))
            leaked.append(str(mind.config))
            leaked.append(repr(mind.config.decider))
            leaked.append(repr(mind.decider))
            leaked.append(json.dumps(mind.messages))
        leaked.extend(self.logged)
        for produced in leaked:
            for key in (TRIPWIRE, DECIDER_TRIPWIRE):
                self.assertNotIn(key, produced, "a key leaked into %r" % produced[:160])

    def test_the_decider_sends_its_key_as_a_bearer_header_and_nowhere_else(self):
        mind = self.mind("fills")
        turn, _ = recording_turn("put dentist on friday at 3")
        mind.answer(turn)
        body = json.dumps(self.decisions()[0][1])
        self.assertNotIn(DECIDER_TRIPWIRE, body)
        self.assertNotIn(TRIPWIRE, body)


class TypedReadTests(unittest.TestCase):
    """The `/v1/decide` adapter: the same questions, the narrower endpoint."""

    @classmethod
    def setUpClass(cls):
        cls.brain = fake_systemone.start()
        cls.base = "http://127.0.0.1:%d" % cls.brain.server_address[1]

    @classmethod
    def tearDownClass(cls):
        cls.brain.shutdown()
        cls.brain.server_close()

    def setUp(self):
        with self.brain.lock:
            self.brain.requests = []
            self.brain.scenario = "confident"

    def client(self):
        return Decide(DeciderConfig(kind="decide", base_url=self.base))

    def test_the_catalogue_goes_in_the_preamble_and_the_request_in_the_record(self):
        world = world_from_messages(seen_desktop() + [{"role": "user", "content": "add dentist"}])
        catalogue, situation = world.state()
        picks = self.client().ask(catalogue, situation, questions_for(world))
        path, body = self.brain.requests[0]
        self.assertEqual(path, "/v1/decide")
        self.assertIn("add_event", body["preamble"])
        self.assertIn("add dentist", body["record"])
        self.assertNotIn("add dentist", body["preamble"])
        # Every question carries its own options; a yes/no question is asked as yes/no, because
        # this endpoint has one question type and needs at least two options.
        self.assertEqual([q["opts"] for q in body["questions"]][0], ["yes", "no"])
        self.assertIn(NONE_OF_THESE, body["questions"][1]["opts"])
        self.assertEqual(picks["app"].value, "calendar")
        self.assertEqual(picks["action:calendar"].value, "add_event")
        self.assertAlmostEqual(picks["done"].probability, 0.96)
        self.assertIs(picks["done"].value, False)

    def test_a_question_with_one_option_is_not_asked(self):
        # The endpoint needs two options to choose between, and one option is not a choice: it
        # comes back certain by arithmetic rather than by judgement.
        picks = self.client().ask("c", "s", [Ask("only", "choice", "pick", [("a", "")])])
        self.assertEqual(picks, {})
        self.assertEqual(self.brain.requests, [])


class SystemOneShapeTests(unittest.TestCase):
    """What goes out on `POST /v1/systemone`, in the shape the reference implementation takes."""

    @classmethod
    def setUpClass(cls):
        cls.brain = fake_systemone.start()
        cls.base = "http://127.0.0.1:%d" % cls.brain.server_address[1]

    @classmethod
    def tearDownClass(cls):
        cls.brain.shutdown()
        cls.brain.server_close()

    def setUp(self):
        with self.brain.lock:
            self.brain.requests = []
            self.brain.scenario = "confident"

    def test_every_question_is_typed_and_carries_its_own_criteria(self):
        world = world_from_messages(seen_desktop() + [{"role": "user", "content": "add dentist"}])
        catalogue, situation = world.state()
        SystemOne(DeciderConfig(kind="kev", base_url=self.base)).ask(
            catalogue, situation, questions_for(world))
        _, body = self.brain.requests[0]
        self.assertEqual(body["model"], "kev-latest")
        self.assertEqual(body["questions"]["done"]["type"], "noul")
        self.assertEqual(set(body["questions"]["done"]["criteria"]), {"true", "false"})
        self.assertEqual(body["questions"]["app"]["type"], "choice")
        self.assertIn(NONE_OF_THESE, body["questions"]["app"]["criteria"])
        self.assertIn("calendar", body["questions"]["app"]["criteria"])
        self.assertIn("add_event", body["questions"]["action:calendar"]["criteria"])
        self.assertIsInstance(body["state"], str)

    def test_the_question_ids_are_not_what_carries_the_meaning(self):
        # The id is never sent to the model, so every question has to stand on its own
        # instructions — including the premise that the app question has already been settled.
        world = world_from_messages(seen_desktop() + [{"role": "user", "content": "add dentist"}])
        asks = {ask.id: ask for ask in questions_for(world)}
        self.assertIn("finished", asks["done"].instructions)
        self.assertIn("Which app", asks["app"].instructions)
        self.assertIn("Suppose", asks["action:calendar"].instructions)
        self.assertIn("`calendar`", asks["action:calendar"].instructions)

    def test_the_endpoint_is_the_api_root_however_the_person_wrote_it(self):
        for base in ("http://box:8009", "http://box:8009/", "http://box:8009/v1"):
            self.assertEqual(DeciderConfig(kind="kev", base_url=base).endpoint,
                             "http://box:8009/v1/systemone")
        self.assertEqual(DeciderConfig(kind="decide", base_url="http://box:8080/v1").endpoint,
                         "http://box:8080/v1/decide")


class StateTests(unittest.TestCase):
    """The state the decider is shown, read out of the conversation."""

    def test_the_apps_are_read_by_the_name_the_tools_take(self):
        apps = apps_from_listing(LISTING)
        self.assertEqual([(name, where) for name, _, where in apps],
                         [("calendar", "open"), ("terminal", "open"),
                          ("weather", "service"), ("notifications", "service"),
                          ("notes", "closed"), ("system-monitor", "closed")])
        # `sysmonitor` is what opens it and `system-monitor` is what describes it. The listing
        # is the only place that says so, which is why this parses the listing.
        self.assertNotIn("sysmonitor", [name for name, _, _ in apps])

    def test_a_closed_row_is_read_for_what_the_app_is_for(self):
        # `yos ls` marks a closed app `(closed)` and lists its other names after its purpose; the
        # decider is shown the purpose, not the markup around it.
        listing = ("Can be opened with `act shell open_app name=<name>`, then described by that name:\n"
                   "    calendar           (closed)  events and appointments\n"
                   "    system-monitor     (closed)  CPU, memory, disk and processes  (also sysmonitor)\n")
        self.assertEqual(apps_from_listing(listing), [
            ("calendar", "events and appointments", "closed"),
            ("system-monitor", "CPU, memory, disk and processes", "closed")])

    def test_an_open_app_listed_by_name_takes_its_purpose_from_the_catalogue(self):
        # `os_apps` names the running apps and nothing more (their summaries are their state), and
        # says what each app is for in the catalogue rows after (#1062).
        rows = [
            "Open now (describe these by name):",
            "    calendar",
            "Services answering: (none)",
            "Every app on this machine, open or not (name, what it is for):",
            '    {"name": "calendar", "opens": "app", "describe_as": "calendar", "for": "events and appointments"}',
            '    {"name": "notes", "opens": "app", "describe_as": "notes", "for": "quick notes"}',
        ]
        self.assertEqual(apps_from_listing(chr(10).join(rows) + chr(10)),
                         [("calendar", "events and appointments", "open")])

    def test_prose_under_a_heading_is_not_read_as_an_app(self):
        # Two indented lines of prose sit under "Services answering", and a reader that takes
        # every indented line as an entry reads `a` and `window,` as apps of this desktop.
        names = [name for name, _, _ in apps_from_listing(LISTING)]
        self.assertNotIn("a", names)
        self.assertNotIn("window,", names)
        self.assertNotIn("files", names, "a screen of the desktop is not an app to act on")

    def test_the_actions_are_read_with_the_line_the_app_says_they_are_for(self):
        self.assertEqual(actions_from_describe(DESCRIBE), [
            ("select_day", "Show what is on one day of the month shown"),
            ("add_event", "Put something on the calendar"),
            ("delete_event", "Take an event off the calendar. It is not recoverable")])

    def test_an_arguments_line_is_not_mistaken_for_a_purpose(self):
        text = "  act: only(day)  [standard, settles on return]\n         day: number — the day"
        self.assertEqual(actions_from_describe(text), [("only", "")])

    def test_a_describe_that_was_cut_short_is_not_a_list_to_choose_from(self):
        # A partial list does not make the answer uncertain, it makes it confidently wrong: a
        # decision model can only choose an option it was given.
        cut = DESCRIBE[:200] + "\n… (cut: 900 more characters)"
        world = world_from_messages([
            {"role": "user", "content": "add dentist"},
            {"role": "assistant", "content": "", "tool_calls": [
                {"id": "a", "type": "function",
                 "function": {"name": "os_apps", "arguments": "{}"}},
                {"id": "b", "type": "function",
                 "function": {"name": "os_describe", "arguments": '{"app": "calendar"}'}}]},
            {"role": "tool", "tool_call_id": "a", "name": "os_apps", "content": LISTING},
            {"role": "tool", "tool_call_id": "b", "name": "os_describe", "content": cut},
        ])
        self.assertEqual(world.known_actions("calendar"), [])
        self.assertEqual([ask.id for ask in questions_for(world)], ["done", "app"])

    def test_the_apps_outlive_a_request_and_the_steps_do_not(self):
        world = world_from_messages(seen_desktop() + [
            {"role": "user", "content": "and add dentist on friday"}])
        self.assertEqual(world.ask, "and add dentist on friday")
        self.assertEqual(world.steps, [], "a new question starts the account of steps over")
        self.assertIn("calendar", [name for name, _, _ in world.apps])
        self.assertTrue(world.known_actions("calendar"))

    def test_what_has_been_done_is_in_order_and_named_by_what_it_touched(self):
        world = world_from_messages(seen_desktop()[:4])
        _, situation = world.state()
        self.assertIn("1. os_apps →", situation)
        self.assertIn("2. os_describe calendar →", situation)
        self.assertIn("what is on my calendar?", situation)

    def test_the_state_is_bounded_whatever_the_conversation_did(self):
        messages = [{"role": "user", "content": "x" * 5000}]
        for index in range(40):
            messages.append({"role": "assistant", "content": "", "tool_calls": [
                {"id": str(index), "type": "function",
                 "function": {"name": "os_apps", "arguments": "{}"}}]})
            messages.append({"role": "tool", "tool_call_id": str(index), "name": "os_apps",
                             "content": LISTING + "y" * 9000})
        catalogue, situation = world_from_messages(messages).state()
        self.assertLess(len(catalogue) + len(situation), 10_000)
        self.assertLessEqual(situation.count("\n  "), 12)

    def test_no_apps_means_no_questions(self):
        world = World("add dentist", [], [])
        self.assertEqual(questions_for(world), [])


class GateTests(unittest.TestCase):
    """What the answers mean for the step, one row at a time."""

    def read(self, **picks):
        return read_picks({k.replace("action_", "action:"): v for k, v in picks.items()}, 0.9, 12)

    def test_a_confident_yes_to_done_ends_the_loop(self):
        self.assertEqual(self.read(done=Pick(True, 0.96)).route, "answer")

    def test_a_confident_app_and_action_is_pinned(self):
        decision = self.read(done=Pick(False, 0.95), app=Pick("calendar", 0.95),
                             action_calendar=Pick("add_event", 0.93))
        self.assertEqual((decision.route, decision.app, decision.action),
                         ("act", "calendar", "add_event"))
        self.assertTrue(decision.held)

    def test_an_unsure_done_is_never_something_to_act_on(self):
        # Above even chance that the work is finished, but not confident enough to stop. The
        # generator has the whole conversation to read and this is where that matters.
        decision = self.read(done=Pick(True, 0.85), app=Pick("calendar", 0.99),
                             action_calendar=Pick("add_event", 0.99))
        self.assertEqual(decision.route, "generate")
        self.assertIn("may already be finished", decision.line(1))

    def test_an_unsure_action_falls_back_but_is_still_reported(self):
        decision = self.read(done=Pick(False, 0.95), app=Pick("calendar", 0.95),
                             action_calendar=Pick("delete_event", 0.7))
        self.assertEqual(decision.route, "generate")
        self.assertIn("would have acted on calendar.delete_event p=0.70", decision.line(3))
        self.assertIn("gate 0.90 not held", decision.line(3))

    def test_a_confident_none_of_these_ends_the_loop_and_an_unsure_one_does_not(self):
        self.assertEqual(self.read(app=Pick(NONE_OF_THESE, 0.95)).route, "answer")
        unsure = self.read(app=Pick(NONE_OF_THESE, 0.7))
        self.assertEqual(unsure.route, "generate")
        self.assertIn("unsure whether any app applies", unsure.line(1))

    def test_an_app_whose_actions_have_not_been_read_has_nothing_to_pin(self):
        decision = self.read(done=Pick(False, 0.95), app=Pick("notes", 0.99))
        self.assertEqual(decision.route, "generate")
        self.assertIn("nothing to pin on notes", decision.line(2))

    def test_the_three_ways_an_app_has_no_action_list_are_told_apart(self):
        # They all end at the same fallback, and a journal that calls all three the same thing
        # cannot be read.
        world = World("x", [], [("a", "", "open")],
                      {"cut": [("one", ""), ("two", "")], "thin": [("only", "")]},
                      incomplete=["cut"])
        self.assertIn("cut short", world.why_no_actions("cut"))
        self.assertIn("only one action", world.why_no_actions("thin"))
        self.assertIn("not been described", world.why_no_actions("unknown"))
        self.assertEqual(world.why_no_actions("fine"), "it has not been described yet")
        picked = read_picks({"app": Pick("cut", 0.99)}, 0.9, 1, world)
        self.assertIn("nothing to pin on cut: its describe was cut short", picked.line(1))

    def test_no_answer_at_all_is_a_fallback_and_not_a_guess(self):
        decision = self.read()
        self.assertEqual(decision.route, "generate")
        self.assertIn("it did not answer which app", decision.line(1))


class PinTests(unittest.TestCase):
    def schemas(self):
        return DesktopTools().as_openai_tools()

    def test_pinning_leaves_only_os_act_with_its_app_and_action_decided(self):
        pinned = pinned_tools(self.schemas(), "calendar", "add_event")
        self.assertEqual(len(pinned), 1)
        function = pinned[0]["function"]
        self.assertEqual(function["name"], "os_act")
        self.assertEqual(function["parameters"]["properties"]["action"]["enum"], ["add_event"])
        self.assertIn("fill in `args`", function["description"])
        # The description the bridge published is still there: it is what says what a refusal
        # means and what the person's mode does to an action.
        self.assertIn("Perform an action an app lists.", function["description"])

    def test_the_tool_list_is_not_changed_by_pinning_it(self):
        schemas = self.schemas()
        pinned_tools(schemas, "calendar", "add_event")
        self.assertEqual(schemas[2]["function"]["parameters"]["properties"]["app"],
                         {"type": "string"})

    def test_a_tool_list_with_nothing_to_pin_says_so(self):
        self.assertIsNone(pinned_tools([], "calendar", "add_event"))
        self.assertIsNone(pinned_tools(
            [{"type": "function", "function": {"name": "os_act", "parameters": {
                "type": "object", "properties": {}}}}], "calendar", "add_event"))


class DeciderConfigTests(unittest.TestCase):
    def written(self, block):
        where = os.path.join(tempfile.mkdtemp(), "deepseek.json")
        with open(where, "w", encoding="utf-8") as handle:
            json.dump({"api_key": TRIPWIRE, "decider": block}, handle)
        return where

    def test_no_decider_block_is_the_normal_case(self):
        where = os.path.join(tempfile.mkdtemp(), "deepseek.json")
        with open(where, "w", encoding="utf-8") as handle:
            json.dump({"api_key": TRIPWIRE}, handle)
        config = load_config(where)
        self.assertIsNone(config.decider)
        self.assertEqual(config.detail, "deepseek-chat · api.deepseek.com")

    def test_a_jev_block_needs_only_its_kind_and_the_name_of_the_key(self):
        os.environ["TYPESAFE_KEY_FOR_TEST"] = DECIDER_TRIPWIRE
        try:
            config = load_config(self.written(
                {"kind": "jev", "api_key_env": "TYPESAFE_KEY_FOR_TEST"}))
        finally:
            del os.environ["TYPESAFE_KEY_FOR_TEST"]
        self.assertEqual(config.decider.endpoint, "https://api.typesafe.ai/v1/systemone")
        self.assertEqual(config.decider.model, "jev-latest")
        self.assertEqual(config.decider.gate, 0.9)
        self.assertIn("jev picks", config.detail)
        self.assertIn(DECIDER_TRIPWIRE, config.secrets)
        self.assertIn(TRIPWIRE, config.secrets)
        self.assertNotIn(DECIDER_TRIPWIRE, repr(config))

    def test_a_local_kev_needs_no_key_at_all(self):
        config = load_config(self.written({"kind": "kev"}))
        self.assertEqual(config.decider.endpoint, "http://127.0.0.1:8009/v1/systemone")
        self.assertEqual(config.decider.api_key, "")

    def test_a_key_written_into_the_file_is_refused_rather_than_used(self):
        with self.assertRaises(ConfigError) as caught:
            load_config(self.written({"kind": "jev", "api_key": DECIDER_TRIPWIRE}))
        self.assertIn("api_key_env", str(caught.exception))
        self.assertNotIn(DECIDER_TRIPWIRE, str(caught.exception))

    def test_a_key_env_var_that_is_not_set_names_the_variable(self):
        with self.assertRaises(ConfigError) as caught:
            load_config(self.written({"kind": "jev", "api_key_env": "NOT_SET_ANYWHERE_XYZ"}))
        self.assertIn("NOT_SET_ANYWHERE_XYZ", str(caught.exception))

    def test_an_unknown_kind_lists_the_ones_there_are(self):
        with self.assertRaises(ConfigError) as caught:
            load_config(self.written({"kind": "gpt"}))
        for kind in ("jev", "kev", "decide"):
            self.assertIn(kind, str(caught.exception))

    def test_a_gate_that_is_not_a_probability_is_refused(self):
        for gate in (0, 1.5, -1, "soon"):
            with self.assertRaises(ConfigError, msg=repr(gate)):
                load_config(self.written({"kind": "kev", "gate": gate}))
        self.assertEqual(load_config(self.written({"kind": "kev", "gate": 1})).decider.gate, 1.0)

    def test_a_decider_that_is_not_an_object_is_refused(self):
        with self.assertRaises(ConfigError):
            load_config(self.written("jev"))


if __name__ == "__main__":
    unittest.main()
