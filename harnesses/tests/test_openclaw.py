"""The OpenClaw harness against a fake gateway and a fake CLI.

Driven through the real `Harness` and the real fake desktop, because the question worth asking
about a harness is not "did it print the answer" but "was the turn the desktop handed over closed
exactly once". OpenClaw can end a turn six ways — the run finishing, the CLI exiting, the stream
dropping, /stop, silence, and never having connected at all — and each one is asserted here to
reach the desktop once and only once.

Both fakes now answer the way a live OpenClaw 2026.9.1 answered: `openclaw agent --json` prints
one indented JSON document when the turn is over, and the gateway's streaming surface is
`POST /v1/chat/completions` with Server-Sent Events. The tests that pin those shapes are the ones
that would have caught what this harness got wrong before anybody ran it — a JSON-lines reader
pointed at a document, a `--session` flag that does not exist, and a message passed as a
positional argument the CLI ignores.
"""

import json
import os
import sys
import tempfile
import threading
import unittest
from pathlib import Path

import support
from support import FakeDesktop, wait_for

_OPENCLAW = Path(__file__).resolve().parents[1] / "openclaw"
if str(_OPENCLAW) not in sys.path:
    sys.path.insert(0, str(_OPENCLAW))

from fake_openclaw import FakeGateway, free_port  # noqa: E402
from yantrik_harness import Harness  # noqa: E402

import yantrik_openclaw  # noqa: E402
from yantrik_openclaw import (  # noqa: E402
    ConfigError,
    OpenClawConfig,
    OpenClawMind,
    _advance,
    chat_request,
    cli_output,
    decode,
    load_config,
    run_document,
)

FAKE_OPENCLAW = str(Path(__file__).resolve().parent / "fake_openclaw.py")


class HarnessCase(unittest.TestCase):
    """Everything that needs a desktop, a mind and a thread running the real poll loop."""

    def setUp(self):
        self.desktop = FakeDesktop()
        self.addCleanup(self.desktop.stop)
        self.work = tempfile.mkdtemp(prefix="fake-openclaw-")

    def start(self, **config):
        config.setdefault("silence_timeout", 10.0)
        config.setdefault("connect_attempts", 1)
        config.setdefault("connect_backoff", 0.05)
        config.setdefault("connect_timeout", 2.0)
        # Off unless a test is about it: with the preamble on, the first message is the desktop
        # prompt plus the question, and every assertion about what was sent gets longer.
        config.setdefault("preamble", "")
        mind = OpenClawMind(OpenClawConfig(config), log=lambda message: None)
        self.addCleanup(mind.close)
        harness = Harness("openclaw", "OpenClaw", mind, address=self.desktop.path, tools=True,
                          memory=True, poll_interval=0.02, retry_seconds=0.2,
                          heartbeat_seconds=30.0, log=lambda message: None)
        thread = threading.Thread(target=harness.run, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(harness.stop)
        self.assertTrue(wait_for(lambda: bool(self.desktop.attachments)), "never attached")
        return mind

    def gateway(self, scenario="text", **kwargs):
        gw = FakeGateway(scenario, **kwargs)
        self.addCleanup(gw.stop)
        return gw

    def cli(self, scenario, **config):
        config["route"] = "cli"
        config["command"] = [sys.executable, FAKE_OPENCLAW, scenario]
        config.setdefault("env", {"FAKE_OPENCLAW_ARGV_DUMP": os.path.join(self.work, "argv")})
        return self.start(**config)

    def argv(self):
        path = os.path.join(self.work, "argv")
        if not os.path.exists(path):
            return []
        with open(path, encoding="utf-8") as handle:
            return [json.loads(line) for line in handle if line.strip()]


@unittest.skipUnless(support.HAS_UNIX_SOCKETS, "the harness socket is a unix socket")
class GatewayRouteTests(HarnessCase):
    """The gateway's OpenAI-compatible route: `POST /v1/chat/completions`, streamed as SSE."""

    def test_text_deltas_reach_the_panel(self):
        gw = self.gateway("text")
        self.start(route="gateway", gateway_url=gw.url)
        turn = self.desktop.ask("what is open?")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "Two windows.")
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)
        self.assertEqual(gw.messages(), ["what is open?"])

    def test_the_request_is_the_one_the_live_route_wants(self):
        # The three things a live gateway actually reads: the route, the agent target in `model`,
        # and the session key in a header. Getting the session wrong is a harness whose second
        # turn has forgotten the first.
        gw = self.gateway("text")
        self.start(route="gateway", gateway_url=gw.url)
        self.desktop.wait_closed(self.desktop.ask("hello"))
        request = gw.sent()[0]
        self.assertEqual(request["path"], "/v1/chat/completions")
        self.assertEqual(request["headers"]["x-openclaw-session-key"], "yantrik-desktop")
        self.assertEqual(request["body"]["model"], "openclaw/default")
        self.assertTrue(request["body"]["stream"])

    def test_a_named_agent_is_the_model_target_not_a_flag(self):
        gw = self.gateway("text")
        self.start(route="gateway", gateway_url=gw.url, agent="ops")
        self.desktop.wait_closed(self.desktop.ask("hello"))
        self.assertEqual(gw.sent()[0]["body"]["model"], "openclaw/ops")

    def test_a_model_becomes_the_override_header(self):
        gw = self.gateway("text")
        self.start(route="gateway", gateway_url=gw.url, model="ollama-cloud/kimi-k3")
        self.desktop.wait_closed(self.desktop.ask("hello"))
        self.assertEqual(gw.sent()[0]["headers"]["x-openclaw-model"], "ollama-cloud/kimi-k3")

    def test_a_token_is_sent_as_a_bearer_header(self):
        gw = self.gateway("text")
        self.start(route="gateway", gateway_url=gw.url, token="s3cret-gateway-token")
        self.desktop.wait_closed(self.desktop.ask("hello"))
        self.assertEqual(gw.sent()[0]["headers"]["authorization"], "Bearer s3cret-gateway-token")

    def test_a_tool_event_becomes_the_same_trail_line_every_harness_shows(self):
        gw = self.gateway("tool")
        self.start(route="gateway", gateway_url=gw.url)
        turn = self.desktop.ask("add dentist")
        self.desktop.wait_closed(turn)
        text = self.desktop.text(turn)
        self.assertIn("⚙️ os_act calendar.add_event", text)
        self.assertIn("Added it.", text)
        # The trail names what was touched and, since #125, with what: the panel shows the
        # arguments under the call, the way the approval card for the same call does.
        self.assertIn('{"args":{"title":"dentist"}}', text.split("Added it.")[0])

    def test_stop_closes_the_stream_and_the_turn_once(self):
        gw = self.gateway("abort")
        self.start(route="gateway", gateway_url=gw.url)
        working = self.desktop.ask("a long job")
        self.assertTrue(wait_for(lambda: "working" in self.desktop.text(working), timeout=5))
        stopping = self.desktop.ask("/stop")
        self.assertEqual(self.desktop.wait_closed(stopping)[1], "complete")
        # A stream this harness closed itself is an ending, not a gateway that dropped the answer.
        self.assertEqual(self.desktop.wait_closed(working, timeout=8)[1], "complete")
        self.assertEqual(len(self.desktop.closes_for(working)), 1)
        self.assertTrue(wait_for(lambda: gw.hangups >= 1, timeout=5),
                        "the gateway never saw the connection close")

    def test_a_gateway_that_is_down_says_so_and_works_once_it_is_up(self):
        # The failure this replaces is a harness that hangs on connect while the person watches
        # a cursor, so the first turn must come back with the sentence naming the fix.
        port = free_port()
        self.start(route="gateway", gateway_url="http://127.0.0.1:%d" % port)
        first = self.desktop.ask("hello")
        closed = self.desktop.wait_closed(first, timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("gateway is not running", closed[2])
        self.assertIn("openclaw daemon start", closed[2])

        self.gateway("text", port=port)
        second = self.desktop.ask("hello again")
        self.assertEqual(self.desktop.wait_closed(second, timeout=8)[1], "complete")
        self.assertEqual(self.desktop.text(second), "Two windows.")

    def test_the_route_being_switched_off_names_the_setting_that_turns_it_on(self):
        # The endpoint ships disabled, so a 404 from a gateway that answered is not a wrong URL —
        # it is one config key, and saying which one is the difference between a two-minute fix
        # and an afternoon.
        gw = self.gateway("off")
        self.start(route="gateway", gateway_url=gw.url)
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("chatCompletions", closed[2])

    def test_a_refused_token_names_the_config_key_that_holds_it(self):
        gw = self.gateway("unauthorized")
        self.start(route="gateway", gateway_url=gw.url)
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("token_env", closed[2])

    def test_an_error_in_the_stream_becomes_a_sentence(self):
        gw = self.gateway("error")
        self.start(route="gateway", gateway_url=gw.url)
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("no model configured", closed[2])

    def test_the_stream_dropping_mid_turn_fails_the_turn_once(self):
        gw = self.gateway("drop")
        self.start(route="gateway", gateway_url=gw.url)
        turn = self.desktop.ask("hello")
        closed = self.desktop.wait_closed(turn, timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("dropped this answer", closed[2])
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)

    def test_a_gateway_that_goes_quiet_fails_the_turn_rather_than_hanging(self):
        gw = self.gateway("silent")
        self.start(route="gateway", gateway_url=gw.url, silence_timeout=1.0)
        turn = self.desktop.ask("hello")
        closed = self.desktop.wait_closed(turn, timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("said nothing for 1 seconds", closed[2])
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)

    def test_new_moves_the_session_key_on(self):
        gw = self.gateway("text")
        self.start(route="gateway", gateway_url=gw.url)
        self.desktop.wait_closed(self.desktop.ask("hello"))
        self.assertEqual(gw.sent()[0]["headers"]["x-openclaw-session-key"], "yantrik-desktop")

        fresh = self.desktop.ask("/new")
        self.assertEqual(self.desktop.wait_closed(fresh)[1], "complete")

        self.desktop.wait_closed(self.desktop.ask("hello again"))
        self.assertEqual(gw.sent()[-1]["headers"]["x-openclaw-session-key"], "yantrik-desktop-1")

    def test_a_stream_that_resends_the_whole_answer_does_not_print_it_twice(self):
        gw = self.gateway("snapshot")
        self.start(route="gateway", gateway_url=gw.url)
        turn = self.desktop.ask("what is open?")
        self.desktop.wait_closed(turn)
        self.assertEqual(self.desktop.text(turn), "Two windows.")

    def test_the_desktop_preamble_is_sent_once_per_session_not_on_every_turn(self):
        gw = self.gateway("text")
        self.start(route="gateway", gateway_url=gw.url,
                   preamble=yantrik_openclaw.DESKTOP_PROMPT)
        self.desktop.wait_closed(self.desktop.ask("hello"))
        self.desktop.wait_closed(self.desktop.ask("and again"))
        sent = gw.messages()
        self.assertIn("REFUSED", sent[0])
        self.assertTrue(sent[0].endswith("hello"))
        self.assertEqual(sent[1], "and again")


@unittest.skipUnless(support.HAS_UNIX_SOCKETS, "the harness socket is a unix socket")
class CliRouteTests(HarnessCase):
    def test_the_run_document_becomes_the_answer(self):
        self.cli("text")
        turn = self.desktop.ask("what is open?")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "Two windows.")
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)

    def test_the_tool_summary_becomes_the_trail_line(self):
        self.cli("tool")
        turn = self.desktop.ask("add dentist")
        self.desktop.wait_closed(turn)
        self.assertIn("⚙️ yantrik-os__os_act", self.desktop.text(turn))
        self.assertIn("Added it.", self.desktop.text(turn))

    def test_openclaw_is_run_with_the_flags_it_actually_has(self):
        # What this pins: `--message` (the message is not a positional, and a build given one
        # ignores it and answers nothing) and `--session-key` (there is no `--session`).
        self.cli("text")
        self.desktop.wait_closed(self.desktop.ask("hello"))
        argv = self.argv()[0]
        self.assertIn("agent", argv)
        self.assertIn("--json", argv)
        self.assertNotIn("--session", argv)
        self.assertEqual(argv[argv.index("--session-key") + 1], "yantrik-desktop")
        self.assertEqual(argv[-2:], ["--message", "hello"])

    def test_a_build_with_no_json_mode_still_shows_its_answer(self):
        # A wrong `--json` spelling must not turn an answering CLI into a silent one.
        self.cli("plain")
        turn = self.desktop.ask("hello")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertIn("Two windows.", self.desktop.text(turn))

    def test_a_build_that_streams_json_lines_is_still_read(self):
        self.cli("lines")
        turn = self.desktop.ask("hello")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "Two windows.")

    def test_a_failed_run_says_why_even_though_it_printed_a_document(self):
        # A failing run prints its JSON and *then* exits non-zero. Treating the exit code as the
        # whole story throws away the only sentence the person can act on.
        self.cli("error")
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("model unavailable", closed[2])

    def test_a_turn_that_is_already_running_is_named_as_such(self):
        self.cli("inflight")
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("already working", closed[2])

    def test_a_flag_it_does_not_recognise_names_the_config_key_to_fix(self):
        self.cli("fail")
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("args", closed[2])
        self.assertIn("unknown option", closed[2])

    def test_a_run_that_printed_nothing_is_not_an_empty_answer(self):
        # The shape that would have caught the original bug: exit 0, nothing readable on stdout,
        # and a panel left with an empty bubble nobody can tell from a hang.
        self.cli("empty")
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("without printing an answer", closed[2])

    def test_a_cli_that_cannot_reach_the_daemon_gets_the_same_sentence(self):
        self.cli("down")
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("gateway is not running", closed[2])

    def test_stop_ends_the_child_and_closes_the_turn_once(self):
        self.cli("silent")
        working = self.desktop.ask("a long job")
        self.assertTrue(wait_for(lambda: working in self.desktop.open_turns, timeout=5))
        stopping = self.desktop.ask("/stop")
        self.assertEqual(self.desktop.wait_closed(stopping)[1], "complete")
        # Killed by this harness, so the turn ends rather than failing: the person asked for it.
        self.assertEqual(self.desktop.wait_closed(working, timeout=8)[1], "complete")
        self.assertEqual(len(self.desktop.closes_for(working)), 1)

    def test_a_cli_that_goes_quiet_fails_the_turn_rather_than_hanging(self):
        self.cli("silent", silence_timeout=1.0)
        closed = self.desktop.wait_closed(self.desktop.ask("hello"), timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("said nothing for 1 seconds", closed[2])

    def test_new_moves_the_session_on_for_the_next_invocation(self):
        self.cli("text")
        self.desktop.wait_closed(self.desktop.ask("hello"))
        fresh = self.desktop.ask("/new")
        self.assertEqual(self.desktop.wait_closed(fresh)[1], "complete")
        self.desktop.wait_closed(self.desktop.ask("hello again"))
        argv = self.argv()[-1]
        self.assertEqual(argv[argv.index("--session-key") + 1], "yantrik-desktop-1")

    def test_the_cli_runs_in_a_directory_of_the_desktops_own_never_in_the_home_folder(self):
        # `openclaw agent` is a coding agent like pi: the directory it runs in is the project
        # whose instruction files it reads, and the harness's own directory is $HOME under the
        # user service, so the default must be a directory of the desktop's own (#183).
        self.cli("text", env={"FAKE_OPENCLAW_CWD_DUMP": os.path.join(self.work, "cwd")})
        turn = self.desktop.ask("what is open?")
        self.assertEqual(self.desktop.wait_closed(turn, timeout=8)[1], "complete")
        with open(os.path.join(self.work, "cwd"), encoding="utf-8") as handle:
            cwds = [os.path.realpath(line.strip()) for line in handle if line.strip()]
        expected = os.path.realpath(os.path.join(os.environ["XDG_DATA_HOME"],
                                                 "yantrik", "minds", "openclaw", "main"))
        self.assertEqual(cwds, [expected])
        self.assertNotEqual(expected, os.path.realpath(os.path.expanduser("~")))


class ConfigTests(unittest.TestCase):
    def test_the_silence_budget_exceeds_the_longest_an_approval_card_can_wait(self):
        # An os_act above the ceiling waits ~270s for the person inside a single tool call, with
        # no events at all. A shorter budget would fail turns that were working correctly.
        self.assertGreater(yantrik_openclaw.DEFAULT_SILENCE_TIMEOUT, 300)

    def test_the_cli_route_is_the_default_because_it_needs_no_extra_openclaw_config(self):
        self.assertEqual(OpenClawConfig({}).route, "cli")
        # HTTP, not ws: the streaming route is the gateway's OpenAI-compatible surface.
        self.assertEqual(OpenClawConfig({}).gateway_url, "http://127.0.0.1:18789")
        self.assertEqual(OpenClawConfig({}).session, "yantrik-desktop")

    def test_an_unknown_route_is_refused_rather_than_guessed_at(self):
        with self.assertRaises(ConfigError):
            OpenClawConfig({"route": "grpc"})

    def test_a_token_env_that_is_not_set_says_why_a_user_service_would_not_have_it(self):
        with self.assertRaises(ConfigError) as caught:
            OpenClawConfig({"token_env": "NOT_SET_ANYWHERE_OPENCLAW"}, source="openclaw.json")
        self.assertIn("environment.d", str(caught.exception))

    def test_a_token_env_that_is_set_is_read_from_this_process(self):
        os.environ["OPENCLAW_TEST_TOKEN"] = "abc123"
        self.addCleanup(os.environ.pop, "OPENCLAW_TEST_TOKEN", None)
        self.assertEqual(OpenClawConfig({"token_env": "OPENCLAW_TEST_TOKEN"}).token, "abc123")

    def test_a_missing_config_is_openclaws_own_defaults_not_a_failure(self):
        config = load_config(os.path.join(tempfile.mkdtemp(), "openclaw.json"))
        self.assertEqual(config.command, ["openclaw"])
        self.assertEqual(config.route, "cli")

    def test_a_command_can_be_written_as_a_string(self):
        self.assertEqual(OpenClawConfig({"command": "npx -y openclaw"}).command,
                         ["npx", "-y", "openclaw"])

    def test_local_runs_the_embedded_agent_on_the_command_line(self):
        argv = OpenClawConfig({"local": True, "agent": "primary"}).cli_argv("hi", "s1")
        self.assertIn("--local", argv)
        self.assertEqual(argv[argv.index("--agent") + 1], "primary")

    def test_a_model_is_a_flag_on_one_route_and_a_header_on_the_other(self):
        config = OpenClawConfig({"model": "ollama-cloud/kimi-k3"})
        argv = config.cli_argv("hi", "s1")
        self.assertEqual(argv[argv.index("--model") + 1], "ollama-cloud/kimi-k3")
        headers, _ = chat_request(config, "s1", "hi")
        self.assertEqual(headers["x-openclaw-model"], "ollama-cloud/kimi-k3")

    def test_args_replaces_the_flags_wholesale(self):
        argv = OpenClawConfig({"args": ["agent", "--output-format", "stream-json"]}).cli_argv(
            "hi", "s1")
        self.assertNotIn("--json", argv)
        self.assertEqual(argv[1:4], ["agent", "--output-format", "stream-json"])

    def test_a_ws_gateway_url_from_an_older_config_still_finds_the_route(self):
        # The first version of this harness told people to write ws://; a config file should not
        # break because the transport underneath it was corrected.
        from yantrik_openclaw import GatewayRoute

        route = GatewayRoute(OpenClawConfig({"gateway_url": "ws://127.0.0.1:18789"}),
                             log=lambda message: None)
        self.assertEqual(route._target(), (False, "127.0.0.1", 18789, "/v1/chat/completions"))

    def test_the_picker_detail_names_the_model_and_the_version_it_could_read(self):
        config = OpenClawConfig({"model": "claw-primary",
                                 "command": [sys.executable, FAKE_OPENCLAW]})
        self.assertEqual(config.detail, "claw-primary · OpenClaw 2026.9.1")


class RunDocumentTests(unittest.TestCase):
    """What `openclaw agent --json` prints, which is one document and not a stream."""

    def test_the_document_is_read_whole_rather_than_line_by_line(self):
        # Indented on purpose: every line of this is invalid JSON on its own, which is how a
        # JSON-lines reader turns a working CLI into a panel full of raw JSON.
        raw = json.dumps({"runId": "r1", "status": "ok", "summary": "completed",
                          "result": {"payloads": [{"text": "Two windows."}], "meta": {}}},
                         indent=2)
        self.assertEqual(cli_output(raw), [("text", "Two windows.", False), ("end", None, None)])

    def test_the_tool_summary_is_the_only_trail_this_route_has(self):
        document = {"status": "ok", "result": {
            "payloads": [{"text": "Added it."}],
            "meta": {"toolSummary": {"calls": 2, "tools": ["yantrik-os__os_describe",
                                                           "yantrik-os__os_act"]}}}}
        self.assertEqual(run_document(document), [
            ("tool", "yantrik-os__os_describe", None),
            ("tool", "yantrik-os__os_act", None),
            ("text", "Added it.", False),
            ("end", None, None),
        ])

    def test_a_failed_status_becomes_an_error_carrying_the_summary(self):
        signals = run_document({"status": "error", "summary": "model unavailable",
                                "result": {"payloads": []}})
        self.assertEqual(signals[-1][0], "error")
        self.assertIn("model unavailable", signals[-1][1])

    def test_a_run_already_in_flight_says_so_in_words(self):
        signals = run_document({"status": "in_flight", "result": {}})
        self.assertIn("already working", signals[-1][1])

    def test_an_answer_with_no_payloads_falls_back_to_the_final_text_in_meta(self):
        signals = run_document({"status": "ok", "result": {
            "payloads": [], "meta": {"finalAssistantVisibleText": "READY"}}})
        self.assertEqual(signals[0], ("text", "READY", False))

    def test_plain_text_is_still_an_answer(self):
        self.assertEqual(cli_output("Two windows.\n"), [("text", "Two windows.\n", False)])

    def test_json_lines_are_still_read(self):
        raw = '{"type": "assistant", "delta": "Two "}\n{"type": "assistant", "delta": "x"}\n'
        self.assertEqual(cli_output(raw),
                         [("text", "Two ", False), ("text", "x", False)])

    def test_nothing_at_all_is_no_signals(self):
        self.assertEqual(cli_output("   \n"), [])


class DecoderTests(unittest.TestCase):
    """The permissive half: whatever a build calls its events, they land as the same signals."""

    def signals(self, event):
        return decode(event)

    def test_a_flat_delta(self):
        self.assertEqual(self.signals({"type": "assistant", "delta": "hi"}),
                         [("text", "hi", False)])

    def test_an_anthropic_shaped_content_block(self):
        self.assertEqual(
            self.signals({"type": "content_block_delta",
                          "delta": {"type": "text_delta", "text": "hi"}}),
            [("text", "hi", False)])

    def test_the_chunk_the_gateways_own_route_sends(self):
        self.assertEqual(
            self.signals({"object": "chat.completion.chunk",
                          "choices": [{"index": 0, "delta": {"content": "hi"},
                                       "finish_reason": None}]}),
            [("text", "hi", False)])

    def test_a_role_only_chunk_is_proof_of_life_and_nothing_on_screen(self):
        self.assertEqual(
            self.signals({"object": "chat.completion.chunk",
                          "choices": [{"delta": {"role": "assistant"}}]}),
            [("alive", None, None)])

    def test_a_non_streamed_completion_is_a_snapshot(self):
        self.assertEqual(
            self.signals({"object": "chat.completion",
                          "choices": [{"message": {"role": "assistant", "content": "hi"}}]}),
            [("text", "hi", True)])

    def test_a_tool_call_under_any_of_its_names(self):
        for event in ({"type": "tool_use", "name": "os_act", "input": {"app": "notes"}},
                      {"type": "tool_call", "tool": "os_act", "arguments": {"app": "notes"}},
                      {"type": "tool_execution_start", "toolName": "os_act",
                       "args": {"app": "notes"}}):
            self.assertEqual(self.signals(event), [("tool", "os_act", {"app": "notes"})])

    def test_a_result_event_carries_both_the_last_text_and_the_ending(self):
        self.assertEqual(self.signals({"type": "result", "text": "done."}),
                         [("text", "done.", True), ("end", None, None)])

    def test_an_error_becomes_a_sentence_not_a_shrug(self):
        self.assertEqual(self.signals({"type": "error", "error": {"message": "no model"}}),
                         [("error", "no model", None)])

    def test_the_openai_error_object_has_no_type_to_match_on(self):
        # `{"error": {…}}` and nothing else — the shape a refused request arrives in on the
        # gateway's own route. A decoder that needed a `type` here would answer a refusal with
        # silence.
        self.assertEqual(self.signals({"error": {"message": "no model configured",
                                                 "type": "invalid_request_error"}}),
                         [("error", "no model configured", None)])

    def test_thinking_is_proof_of_life_and_nothing_on_screen(self):
        # A model talking to itself is not the answer, and on a desktop panel it reads as
        # rambling. It still resets the silence clock.
        self.assertEqual(self.signals({"type": "thinking_delta", "delta": "hmm"}),
                         [("alive", None, None)])

    def test_an_unrecognised_event_is_reported_as_unrecognised(self):
        # The failure mode this exists for: a protocol mismatch that looks exactly like an agent
        # which has gone quiet.
        self.assertEqual(self.signals({"type": "quantum_flux"}),
                         [("unknown", "quantum_flux", None)])

    def test_a_snapshot_is_trimmed_to_what_is_new(self):
        self.assertEqual(_advance("Two ", "Two windows."), "windows.")
        self.assertEqual(_advance("Two ", "windows."), "windows.")
        self.assertEqual(_advance("", "Two "), "Two ")


if __name__ == "__main__":
    unittest.main()
