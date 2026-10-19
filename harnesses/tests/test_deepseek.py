"""The DeepSeek loop against a fake OpenAI-compatible server.

No network, no key, no provider. The scenario is chosen by the `model` in the request, so one
server covers split tool-call deltas, two calls in one message, `reasoning_content`, and every
HTTP failure worth a sentence.

The tripwire key is the point of half of this file. A key leaks through an error body echoed
verbatim or through a debug print of the config, so both are driven here and asserted against.
"""

import json
import os
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import support
from support import recording_turn, said

import yantrik_deepseek
from yantrik_deepseek import Config, ConfigError, DeepSeekMind, ProviderError, load_config, redact

TRIPWIRE = "sk-tripwire-01234567890abcdefghij"


def frame(delta):
    return "data: " + json.dumps({"choices": [{"index": 0, "delta": delta}]}) + "\n\n"


DONE = "data: [DONE]\n\n"


class _Chat(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length) or b"{}")
        auth = self.headers.get("Authorization") or ""
        model = str(body.get("model") or "")
        with self.server.lock:
            self.server.requests.append(body)
            nth = sum(1 for b in self.server.requests if b.get("model") == model)

        if model in ("401", "402", "429", "500"):
            # Providers do echo the Authorization header back in debug fields. That is the leak
            # this test exists for.
            payload = json.dumps({"error": {"message": "rejected request with %s" % auth}}).encode()
            self.send_response(int(model))
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        frames = self._frames(model, nth, auth)
        if (body.get("stream_options") or {}).get("include_usage") and frames[-1] == DONE:
            # What OpenAI-compatible servers send when asked: one last chunk, no choices.
            frames.insert(-1, "data: " + json.dumps({
                "choices": [], "model": model + "-served",
                "usage": {"prompt_tokens": 321, "completion_tokens": 45, "total_tokens": 366},
            }) + "\n\n")
        for piece in frames:
            self.wfile.write(piece.encode("utf-8"))
            self.wfile.flush()

    def _frames(self, model, nth, auth):
        if model == "plain":
            return [
                frame({"role": "assistant", "content": ""}),
                frame({"reasoning_content": "the user wants a count, let me think"}),
                frame({"content": "Two "}),
                frame({"content": "windows."}),
                DONE,
            ]
        if model == "echo-key":
            return [frame({"content": "your header was %s" % auth}), DONE]
        if model == "describe-first":
            # What a model that already knows this desktop does with "what is on my calendar on
            # the 25th": no os_apps at all, straight to the app it knows the answer is in. It is
            # a good answer, and it is the shape that left the decider with nothing to be asked
            # until the harness started reading the app list itself. See test_deepseek_decider.
            if nth == 1:
                return [
                    frame({"tool_calls": [{"index": 0, "id": "call_d", "type": "function",
                                           "function": {"name": "os_describe",
                                                        "arguments": '{"app": "calendar"}'}}]}),
                    DONE,
                ]
            return [frame({"content": "Nothing on the 25th."}), DONE]
        if model == "fills":
            # A model asked for the arguments of one action, which writes the arguments and
            # also writes an app and an action of its own — what a provider that reads the
            # pinned schema as a hint does. See test_deepseek_decider.
            if nth == 1:
                return [
                    frame({"tool_calls": [{"index": 0, "id": "call_pin", "type": "function",
                                           "function": {"name": "os_act", "arguments": ""}}]}),
                    frame({"tool_calls": [{"index": 0, "function": {
                        "arguments": '{"app": "notes", "action": "new_note", '
                                     '"args": {"title": "Dentist", "date": "2026-09-25"}}'}}]}),
                    DONE,
                ]
            return [frame({"content": "Put it on the calendar."}), DONE]
        if model == "loop":
            return [
                frame({"tool_calls": [{"index": 0, "id": "call_%d" % nth, "type": "function",
                                       "function": {"name": "os_apps", "arguments": "{}"}}]}),
                DONE,
            ]
        if model == "tools":
            if nth == 1:
                return [
                    frame({"role": "assistant", "content": ""}),
                    frame({"reasoning_content": "I should look at the desktop first"}),
                    # One tool call split across three deltas, name included.
                    frame({"tool_calls": [{"index": 0, "id": "call_a", "type": "function",
                                           "function": {"name": "os_", "arguments": ""}}]}),
                    frame({"tool_calls": [{"index": 0,
                                           "function": {"name": "apps", "arguments": "{"}}]}),
                    frame({"tool_calls": [{"index": 0, "function": {"arguments": "}"}}]}),
                    # A second call in the same assistant message, also split.
                    frame({"tool_calls": [{"index": 1, "id": "call_b", "type": "function",
                                           "function": {"name": "os_act", "arguments": ""}}]}),
                    frame({"tool_calls": [{"index": 1,
                                           "function": {"arguments": '{"app": "cal'}}]}),
                    frame({"tool_calls": [{"index": 1,
                                           "function": {"arguments": 'endar", "action": "add_event"}'}}]}),
                    DONE,
                ]
            return [frame({"content": "Added it."}), DONE]
        return [frame({"content": "(no scenario %s)" % model}), DONE]


class FakeTools:
    def __init__(self):
        self.calls = []

    def as_openai_tools(self):
        return [
            {"type": "function", "function": {"name": "os_apps", "description": "what is open",
                                              "parameters": {"type": "object", "properties": {}}}},
            {"type": "function", "function": {"name": "os_act", "description": "act",
                                              "parameters": {"type": "object", "properties": {}}}},
        ]

    def call(self, name, arguments, timeout=None):
        self.calls.append((name, arguments))
        if name == "os_act":
            return ("REFUSED: plan mode is on, so nothing was changed.", False)
        return ("open: notes, calendar", False)


class DeepSeekTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), _Chat)
        cls.server.lock = threading.Lock()
        cls.server.requests = []
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = "http://127.0.0.1:%d/v1" % cls.server.server_address[1]

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        with self.server.lock:
            self.server.requests = []
        self.logged = []

    def mind(self, model, **kwargs):
        config = Config(base_url=self.base, model=model, api_key=TRIPWIRE, **kwargs)
        return DeepSeekMind(config, FakeTools(), log=self.logged.append)

    def requests_for(self, model):
        with self.server.lock:
            return [b for b in self.server.requests if b.get("model") == model]

    # ── the loop ────────────────────────────────────────────────────────

    def test_only_the_answer_is_streamed_and_the_thinking_is_not(self):
        # reasoning_content is the model talking to itself. On a desktop panel it reads as the
        # mind rambling, and the person asked a question.
        mind = self.mind("plain")
        turn, recorder = recording_turn("what is open?")
        mind.answer(turn)
        self.assertEqual(said(recorder), "Two windows.")

    def test_tool_calls_split_across_chunks_are_reassembled_and_run(self):
        mind = self.mind("tools")
        turn, recorder = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        self.assertEqual(mind.tools.calls,
                         [("os_apps", {}), ("os_act", {"app": "calendar", "action": "add_event"})])
        text = said(recorder)
        self.assertIn("⚙️ os_apps", text)
        self.assertIn("⚙️ os_act calendar.add_event", text)
        self.assertTrue(text.endswith("Added it."))
        self.assertNotIn("I should look", text)

    def test_the_tool_results_go_back_to_the_model_and_not_to_the_panel(self):
        mind = self.mind("tools")
        turn, recorder = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        results = [m for m in mind.messages if m["role"] == "tool"]
        self.assertEqual(len(results), 2)
        self.assertIn("REFUSED", results[1]["content"])
        # A refusal is an answer, so it is not dressed up as a failure on the way back.
        self.assertFalse(results[1]["content"].startswith("failed:"))
        self.assertNotIn("REFUSED", said(recorder))

    def test_the_second_request_carries_the_assistant_message_and_both_results(self):
        mind = self.mind("tools")
        turn, _ = recording_turn("add dentist")
        mind.answer(turn)
        second = self.requests_for("tools")[1]["messages"]
        roles = [m["role"] for m in second]
        self.assertEqual(roles, ["system", "user", "assistant", "tool", "tool"])
        self.assertEqual(len(second[2]["tool_calls"]), 2)
        self.assertEqual([m["tool_call_id"] for m in second[3:]], ["call_a", "call_b"])

    def test_the_tools_reach_the_provider_as_function_schemas(self):
        mind = self.mind("plain")
        turn, _ = recording_turn("hi")
        mind.answer(turn)
        sent = self.requests_for("plain")[0]
        self.assertEqual([t["function"]["name"] for t in sent["tools"]], ["os_apps", "os_act"])
        self.assertTrue(sent["stream"])

    def test_a_model_that_never_stops_calling_tools_is_stopped(self):
        mind = self.mind("loop", max_steps=2)
        turn, recorder = recording_turn("go forever")
        mind.answer(turn)
        self.assertEqual(len(self.requests_for("loop")), 2)
        self.assertIn("stopped after 2 steps", said(recorder))

    def test_stop_ends_the_loop_between_steps(self):
        mind = self.mind("loop", max_steps=20)
        turn, recorder = recording_turn("go forever")
        turn.cancelled.set()
        mind.answer(turn)
        self.assertEqual(self.requests_for("loop"), [])
        self.assertIn("stopped", said(recorder))

    def test_new_forgets_the_conversation(self):
        mind = self.mind("plain")
        turn, _ = recording_turn("hi")
        mind.answer(turn)
        self.assertTrue(mind.messages)
        mind.reset()
        self.assertEqual(mind.messages, [])

    def test_what_the_desktop_knows_about_the_machine_reaches_the_model(self):
        mind = self.mind("plain")
        turn, _ = recording_turn("where am I?", context='{"machine": {"timezone": "Asia/Kolkata"}}')
        mind.answer(turn)
        system = self.requests_for("plain")[0]["messages"][0]["content"]
        self.assertIn("Asia/Kolkata", system)
        self.assertIn("REFUSED", system, "the system prompt must say what a refusal means")

    def test_a_picked_model_and_effort_go_to_the_gateway_and_nowhere_else(self):
        import yantrik_deepseek
        from urllib.parse import urlsplit
        from unittest import mock

        picked = {"model": "plain", "effort": "high"}
        mind = self.mind("tools")
        turn, recorder = recording_turn("hi", options=picked)
        with mock.patch.object(yantrik_deepseek, "GATEWAY_HOSTS", (urlsplit(self.base).netloc,)):
            mind.answer(turn)
        self.assertEqual(said(recorder), "Two windows.", "the picked model answered, not the configured one")
        self.assertEqual(self.requests_for("plain")[0]["effort"], "high")
        self.assertEqual(self.requests_for("tools"), [])
        # Its own endpoint: a gateway id means nothing there, so its own model is asked.
        with self.server.lock:
            self.server.requests = []
        own = self.mind("plain")
        turn, _ = recording_turn("hi", options={"model": "tools", "effort": "high"})
        own.answer(turn)
        self.assertEqual(len(self.requests_for("plain")), 1)
        self.assertNotIn("effort", self.requests_for("plain")[0])

    # ── what the agent is doing ─────────────────────────────────────────

    def test_each_tool_call_is_a_card_that_settles_with_its_result(self):
        mind = self.mind("tools")
        turn, recorder = recording_turn("add dentist to my calendar")
        mind.answer(turn)
        calls = recorder.calls()
        self.assertEqual(list(calls), ["call_a", "call_b"])
        self.assertEqual([e["kind"] for e in calls["call_a"]], ["tool_start", "tool_output", "tool_end"])
        start_b = calls["call_b"][0]
        self.assertEqual((start_b["name"], start_b["target"]), ("os_act", "calendar.add_event"))
        self.assertEqual(start_b["args"], {"app": "calendar", "action": "add_event"})
        self.assertEqual(calls["call_a"][1]["delta"], "open: notes, calendar")
        self.assertTrue(calls["call_a"][-1]["ok"])
        # A refusal answered, and nothing was done: the card says so.
        self.assertFalse(calls["call_b"][-1]["ok"])
        self.assertIn("REFUSED", calls["call_b"][-1]["summary"])

    def test_the_thinking_goes_beside_the_answer_not_into_it(self):
        mind = self.mind("tools")
        turn, recorder = recording_turn("add dentist")
        mind.answer(turn)
        thinking = [e["delta"] for e in recorder.events if e["kind"] == "thinking"]
        self.assertEqual(thinking, ["I should look at the desktop first"])
        self.assertNotIn("I should look", said(recorder))

    def test_what_each_request_cost_comes_from_the_api_response(self):
        mind = self.mind("tools")
        turn, recorder = recording_turn("add dentist")
        mind.answer(turn)
        usage = [e for e in recorder.events if e["kind"] == "usage"]
        # Two requests in this turn, one usage each; they add up.
        self.assertEqual(usage, [{"kind": "usage", "model": "tools-served",
                                  "input_tokens": 321, "output_tokens": 45}] * 2)
        self.assertEqual(self.requests_for("tools")[0]["stream_options"], {"include_usage": True})

    def test_usage_can_be_turned_off_for_a_server_that_rejects_the_field(self):
        mind = self.mind("plain", include_usage=False)
        turn, recorder = recording_turn("hi")
        mind.answer(turn)
        self.assertNotIn("stream_options", self.requests_for("plain")[0])
        self.assertNotIn("usage", recorder.kinds())

    def test_each_conversation_has_its_own_history_and_its_own_bridge(self):
        made = []

        def make_tools(token):
            tools = FakeTools()
            made.append((token, tools))
            return tools

        config = Config(base_url=self.base, model="plain", api_key=TRIPWIRE)
        handler = yantrik_deepseek.handler(config, make_tools=make_tools, log=self.logged.append)
        self.assertTrue(handler.conversations)
        first, _ = recording_turn("about the photos", conversation="c-aaaaaa", agent_token="a" * 32)
        second, _ = recording_turn("about the release", conversation="c-bbbbbb", agent_token="b" * 32)
        handler.answer(first)
        handler.answer(second)

        self.assertEqual([token for token, _ in made], ["a" * 32, "b" * 32])
        requests = self.requests_for("plain")
        users = [[m["content"] for m in r["messages"] if m["role"] == "user"] for r in requests]
        self.assertEqual(users, [["about the photos"], ["about the release"]])
        history = handler.mind("c-aaaaaa").messages
        self.assertNotIn("about the release", json.dumps(history))

    def test_the_bridge_for_a_conversation_carries_its_agents_token(self):
        config = Config(base_url=self.base, model="plain", api_key=TRIPWIRE)
        handler = yantrik_deepseek.handler(config, log=self.logged.append)
        turn, _ = recording_turn("hi", conversation="c-aaaaaa", agent_token="a" * 32)
        tools = handler._for(turn).tools
        self.assertEqual(tools.env, {"YANTRIK_AGENT_TOKEN": "a" * 32})
        handler.close()

    # ── failures a person can act on ────────────────────────────────────

    def test_each_http_failure_is_a_sentence_about_what_to_do(self):
        for model, expected in (("401", "key"), ("402", "billed"),
                                ("429", "rate-limiting"), ("500", "server error")):
            mind = self.mind(model)
            turn, _ = recording_turn("hi")
            with self.assertRaises(ProviderError) as caught:
                mind.answer(turn)
            self.assertIn(expected, str(caught.exception))
            self.assertIn(model, str(caught.exception))

    def test_an_unreachable_endpoint_says_so(self):
        config = Config(base_url="http://127.0.0.1:1/v1", model="plain", api_key=TRIPWIRE)
        mind = DeepSeekMind(config, FakeTools(), log=self.logged.append)
        turn, _ = recording_turn("hi")
        with self.assertRaises(ProviderError) as caught:
            mind.answer(turn)
        self.assertIn("could not reach", str(caught.exception))

    # ── the key ─────────────────────────────────────────────────────────

    def test_the_key_never_appears_in_anything_this_module_produces(self):
        leaked = []
        for model in ("echo-key", "401", "402", "429", "500"):
            mind = self.mind(model)
            turn, recorder = recording_turn("hi")
            try:
                mind.answer(turn)
            except ProviderError as exc:
                leaked.append(str(exc))
            leaked.append(said(recorder))
            leaked.append(json.dumps(recorder.events))
            leaked.append(repr(mind))
            leaked.append(repr(mind.config))
            leaked.append(str(mind.config))
            leaked.append(json.dumps(mind.messages))
        leaked.extend(self.logged)
        for produced in leaked:
            self.assertNotIn(TRIPWIRE, produced, "the key leaked into %r" % produced[:120])

    def test_redact_leaves_ordinary_text_alone_and_ignores_trivial_secrets(self):
        self.assertEqual(redact("all fine", (TRIPWIRE,)), "all fine")
        self.assertEqual(redact("bearer %s" % TRIPWIRE, (TRIPWIRE,)), "bearer <redacted>")
        # A one-character "secret" would redact every sentence into nothing.
        self.assertEqual(redact("a secret", ("a",)), "a secret")

    # ── the config file ─────────────────────────────────────────────────

    def test_a_missing_config_says_what_to_write_in_it(self):
        with self.assertRaises(ConfigError) as caught:
            load_config(os.path.join(tempfile.mkdtemp(), "deepseek.json"))
        self.assertIn("api_key_env", str(caught.exception))

    def test_a_key_env_var_that_is_not_set_names_the_variable(self):
        where = os.path.join(tempfile.mkdtemp(), "deepseek.json")
        with open(where, "w", encoding="utf-8") as handle:
            json.dump({"api_key_env": "NOT_SET_ANYWHERE_XYZ"}, handle)
        with self.assertRaises(ConfigError) as caught:
            load_config(where)
        self.assertIn("NOT_SET_ANYWHERE_XYZ", str(caught.exception))

    def test_the_defaults_are_deepseek_and_anything_else_is_the_persons_choice(self):
        where = os.path.join(tempfile.mkdtemp(), "deepseek.json")
        with open(where, "w", encoding="utf-8") as handle:
            json.dump({"api_key": TRIPWIRE}, handle)
        config = load_config(where)
        self.assertEqual(config.base_url, "https://api.deepseek.com")
        self.assertEqual(config.model, "deepseek-chat")
        self.assertEqual(config.endpoint, "https://api.deepseek.com/chat/completions")
        self.assertEqual(config.detail, "deepseek-chat · api.deepseek.com")
        self.assertNotIn(TRIPWIRE, repr(config))

    def test_an_openai_compatible_endpoint_needs_nothing_but_the_file(self):
        # The thing this harness has to get right to be testable at all: it is not DeepSeek
        # specific, and a machine with no DeepSeek key can still run it.
        where = os.path.join(tempfile.mkdtemp(), "deepseek.json")
        with open(where, "w", encoding="utf-8") as handle:
            json.dump({"base_url": "https://ollama.com/v1", "model": "deepseek-v3.1:671b",
                       "api_key": TRIPWIRE}, handle)
        config = load_config(where)
        self.assertEqual(config.endpoint, "https://ollama.com/v1/chat/completions")
        self.assertEqual(config.detail, "deepseek-v3.1:671b · ollama.com")

    def test_a_local_endpoint_with_no_key_is_allowed(self):
        where = os.path.join(tempfile.mkdtemp(), "deepseek.json")
        with open(where, "w", encoding="utf-8") as handle:
            json.dump({"base_url": "http://127.0.0.1:11434/v1", "model": "x"}, handle)
        self.assertEqual(load_config(where).api_key, "")


class AccreteTests(unittest.TestCase):
    def test_a_name_split_across_chunks_is_joined(self):
        self.assertEqual(yantrik_deepseek._accrete("os_", "apps"), "os_apps")

    def test_a_name_resent_whole_on_every_chunk_is_not_doubled(self):
        # Some providers repeat the whole name in each delta; concatenating blindly turns
        # `os_act` into `os_actos_act` and the call is refused as an unknown tool.
        self.assertEqual(yantrik_deepseek._accrete("os_act", "os_act"), "os_act")


if __name__ == "__main__":
    unittest.main()
