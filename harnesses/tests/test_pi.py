"""The Pi harness against a fake `pi --mode rpc`.

Driven through the real `Harness` and the fake desktop, because the questions worth asking about
this one are all about turn closing: Pi has four ways to end a turn and two ways to end without
ending it, and every one of them has to reach the desktop exactly once.
"""

import json
import os
import sys
import tempfile
import threading
import time
import unittest

import support
from support import FAKE_PI, FakeDesktop, wait_for

from yantrik_harness import Harness
import yantrik_pi
from yantrik_pi import PiConfig, PiMind, load_config


@unittest.skipUnless(support.HAS_UNIX_SOCKETS, "the harness socket is a unix socket")
class PiHarnessTests(unittest.TestCase):
    def setUp(self):
        self.desktop = FakeDesktop()
        self.addCleanup(self.desktop.stop)
        self.work = tempfile.mkdtemp(prefix="fake-pi-")

    def start(self, scenario, **config):
        config.setdefault("silence_timeout", 10.0)
        config.setdefault("settled_grace", 0.3)
        config["command"] = [sys.executable, FAKE_PI, scenario]
        config.setdefault("extension", "")
        config.setdefault("env", {"FAKE_PI_ARGV_DUMP": os.path.join(self.work, "argv"),
                                  "FAKE_PI_COMMANDS_DUMP": os.path.join(self.work, "commands")})
        mind = PiMind(PiConfig(config), log=lambda message: None)
        self.addCleanup(mind.close)
        harness = Harness("pi", "Pi", mind, address=self.desktop.path, tools=True,
                          poll_interval=0.02, retry_seconds=0.2, heartbeat_seconds=30.0,
                          log=lambda message: None)
        thread = threading.Thread(target=harness.run, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(harness.stop)
        self.assertTrue(wait_for(lambda: bool(self.desktop.attachments)), "never attached")
        return mind

    def commands(self):
        path = os.path.join(self.work, "commands")
        if not os.path.exists(path):
            return []
        with open(path, encoding="utf-8") as handle:
            return [json.loads(line) for line in handle if line.strip()]

    # ── the happy path ──────────────────────────────────────────────────

    def test_text_deltas_reach_the_panel_and_thinking_does_not(self):
        self.start("text")
        turn = self.desktop.ask("what is open?")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "Two windows.")
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)

    def test_a_tool_execution_becomes_the_same_trail_line_every_harness_shows(self):
        self.start("tool")
        turn = self.desktop.ask("add dentist")
        self.desktop.wait_closed(turn)
        text = self.desktop.text(turn)
        self.assertIn("⚙️ os_act calendar.add_event", text)
        self.assertIn("Added it.", text)
        # The trail names what was touched and, since #125, with what: the panel shows the
        # arguments under the call, the way the approval card for the same call does.
        self.assertIn('{"args":{"title":"dentist"}}', text.split("Added it.")[0])

    def test_agent_end_without_agent_settled_still_closes_the_turn(self):
        self.start("noend")
        turn = self.desktop.ask("hello")
        self.assertEqual(self.desktop.wait_closed(turn, timeout=6)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "Nearly done.")

    def test_pi_is_started_with_rpc_mode_and_its_builtin_tools_off(self):
        self.start("text")
        turn = self.desktop.ask("hi")
        self.desktop.wait_closed(turn)
        with open(os.path.join(self.work, "argv"), encoding="utf-8") as handle:
            argv = json.loads(handle.readline())
        self.assertIn("--mode", argv)
        self.assertEqual(argv[argv.index("--mode") + 1], "rpc")
        self.assertIn("--no-session", argv)
        self.assertIn("--no-builtin-tools", argv)
        self.assertIn("--append-system-prompt", argv)
        self.assertIn("REFUSED", argv[argv.index("--append-system-prompt") + 1])

    # ── the ways a turn can go wrong ────────────────────────────────────

    def test_a_dialog_is_declined_and_said_out_loud_never_confirmed(self):
        # The desktop has its own approval card. A harness that confirmed for the person would
        # be a second approval path that nobody can see.
        self.start("dialog")
        turn = self.desktop.ask("tidy my files")
        self.assertEqual(self.desktop.wait_closed(turn, timeout=8)[1], "complete")
        text = self.desktop.text(turn)
        self.assertIn("Delete every file in ~/work?", text)
        self.assertIn("declined", text)

        answers = [c for c in self.commands() if c.get("type") == "extension_ui_response"]
        self.assertEqual(len(answers), 1)
        self.assertTrue(answers[0].get("cancelled"))
        self.assertNotIn("confirmed", answers[0])

    def test_stop_becomes_pis_own_abort(self):
        self.start("abort")
        working = self.desktop.ask("a long job")
        self.assertTrue(wait_for(lambda: "working" in self.desktop.text(working), timeout=5))
        stopping = self.desktop.ask("/stop")
        self.assertEqual(self.desktop.wait_closed(stopping)[1], "complete")
        self.assertEqual(self.desktop.wait_closed(working, timeout=8)[1], "complete")
        self.assertIn("stopped", self.desktop.text(working))
        self.assertTrue(any(c.get("type") == "abort" for c in self.commands()))
        self.assertEqual(len(self.desktop.closes_for(working)), 1)

    def test_new_becomes_pis_own_new_session(self):
        self.start("text")
        first = self.desktop.ask("hello")
        self.desktop.wait_closed(first)
        turn = self.desktop.ask("/new")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertTrue(wait_for(
            lambda: any(c.get("type") == "new_session" for c in self.commands()), timeout=3))

    def test_pi_exiting_mid_turn_fails_the_turn_with_a_sentence(self):
        self.start("exit")
        turn = self.desktop.ask("hello")
        closed = self.desktop.wait_closed(turn, timeout=8)
        # It had already said something, so the turn completes with the failure appended rather
        # than being failed outright — either way it is closed, once.
        self.assertIn("pi exited", self.desktop.text(turn) + str(closed[2]))
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)

    def test_pi_going_silent_fails_the_turn_rather_than_hanging(self):
        self.start("silent", silence_timeout=1.0)
        turn = self.desktop.ask("hello")
        closed = self.desktop.wait_closed(turn, timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("said nothing for 1 seconds", closed[2])

    def test_a_prompt_pi_refuses_is_reported_not_swallowed(self):
        self.start("refuse")
        turn = self.desktop.ask("hello")
        closed = self.desktop.wait_closed(turn, timeout=8)
        self.assertEqual(closed[1], "fail")
        self.assertIn("no provider configured", closed[2])


@unittest.skipUnless(support.HAS_UNIX_SOCKETS, "the harness socket is a unix socket")
class PiEventTests(unittest.TestCase):
    """What pi does inside a turn, as the desktop's cards."""

    def setUp(self):
        self.desktop = FakeDesktop()
        self.addCleanup(self.desktop.stop)
        self.work = tempfile.mkdtemp(prefix="fake-pi-")

    def config(self, scenario, **extra):
        config = {"silence_timeout": 10.0, "settled_grace": 0.3, "extension": "",
                  "command": [sys.executable, FAKE_PI, scenario],
                  "env": {"FAKE_PI_ENV_DUMP": os.path.join(self.work, "env"),
                          "FAKE_PI_PROMPTS_DUMP": os.path.join(self.work, "prompts")}}
        config.update(extra)
        return PiConfig(config)

    def start(self, handler):
        self.addCleanup(handler.close)
        harness = Harness("pi", "Pi", handler, address=self.desktop.path, tools=True,
                          poll_interval=0.02, retry_seconds=0.2, heartbeat_seconds=30.0,
                          log=lambda message: None)
        thread = threading.Thread(target=harness.run, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(harness.stop)
        self.assertTrue(wait_for(lambda: bool(self.desktop.attachments)), "never attached")

    def dump(self, name):
        path = os.path.join(self.work, name)
        if not os.path.exists(path):
            return []
        with open(path, encoding="utf-8") as handle:
            return [json.loads(line) for line in handle if line.strip()]

    def test_a_tool_execution_is_a_card_with_its_output_streamed_into_it(self):
        self.start(PiMind(self.config("tool"), log=lambda message: None))
        turn = self.desktop.ask("add dentist")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        events = self.desktop.events_for(turn)
        calls = [e for e in events if e.get("call") == "t1"]
        self.assertEqual([e["kind"] for e in calls],
                         ["tool_start", "tool_output", "tool_output", "tool_output", "tool_end"])
        start = calls[0]
        self.assertEqual((start["name"], start["target"]), ("os_act", "calendar.add_event"))
        self.assertEqual(start["args"]["args"], {"title": "dentist"}, "the card has the arguments whole")
        # Pi reports the output accumulated; the card is sent only what is new, once.
        self.assertEqual("".join(e["delta"] for e in calls if e["kind"] == "tool_output"),
                         "checking the calendar\nadding dentist\ndone")
        end = calls[-1]
        self.assertEqual((end["ok"], end["summary"], end.get("exit_code")),
                         (True, "checking the calendar", 0))
        # The trail line is still in the text for every reader that draws no cards.
        self.assertIn("⚙️ os_act calendar.add_event", self.desktop.text(turn))

    def test_thinking_is_sent_beside_the_answer_and_usage_after_it(self):
        self.start(PiMind(self.config("tool"), log=lambda message: None))
        turn = self.desktop.ask("add dentist")
        self.desktop.wait_closed(turn)
        events = self.desktop.events_for(turn)
        self.assertIn({"kind": "thinking", "delta": "hmm, notes"}, events)
        usage = [e for e in events if e["kind"] == "usage"]
        self.assertEqual(usage, [{"kind": "usage", "model": "fake-model", "input_tokens": 105,
                                  "output_tokens": 20, "cost_usd": 0.003}])
        self.assertNotIn("hmm, notes", self.desktop.text(turn))

    def test_a_refused_act_settles_its_card_as_not_done(self):
        self.start(PiMind(self.config("refused"), log=lambda message: None))
        turn = self.desktop.ask("delete my files")
        self.desktop.wait_closed(turn)
        end = [e for e in self.desktop.events_for(turn) if e["kind"] == "tool_end"][0]
        self.assertFalse(end["ok"])
        self.assertIn("REFUSED", end["summary"])

    def test_each_conversation_is_its_own_pi_process_with_its_agents_token(self):
        gate = os.path.join(self.work, "gate")
        config = self.config("slow")
        config.env["FAKE_PI_GATE"] = gate
        self.start(yantrik_pi.handler(config, log=lambda message: None))
        a = self.desktop.ask("one", conversation="c-aaaaaa", agent_token="a" * 32)
        b = self.desktop.ask("two", conversation="c-bbbbbb", agent_token="b" * 32)
        # They run at once: both prompts reach a pi while neither has answered. One after the
        # other, the second would never arrive until the gate opened.
        self.assertTrue(wait_for(lambda: len(self.dump("prompts")) == 2, timeout=15),
                        "the second conversation waited for the first")
        prompts = {p["message"]: p for p in self.dump("prompts")}
        self.assertNotEqual(prompts["one"]["pid"], prompts["two"]["pid"])
        self.assertEqual(self.desktop.closes_for(a) + self.desktop.closes_for(b), [])
        open(gate, "w").close()
        for turn in (a, b):
            self.assertEqual(self.desktop.wait_closed(turn, timeout=10)[1], "complete")

        started = self.dump("env")
        self.assertEqual(len(started), 2, started)
        self.assertEqual(sorted(p["token"] for p in started), ["a" * 32, "b" * 32])
        pid_of = {p["token"]: p["pid"] for p in started}
        self.assertIn("answered by %d" % pid_of["a" * 32], self.desktop.text(a))
        self.assertIn("answered by %d" % pid_of["b" * 32], self.desktop.text(b))

        # The same conversation again is the same process, and its history.
        again = self.desktop.ask("three", conversation="c-aaaaaa", agent_token="a" * 32)
        self.desktop.wait_closed(again, timeout=8)
        self.assertIn("answered by %d" % pid_of["a" * 32], self.desktop.text(again))
        self.assertEqual(len(self.dump("env")), 2)

    def test_ending_a_conversation_ends_its_pi_process_and_only_that_one(self):
        handler = yantrik_pi.handler(self.config("text"), log=lambda message: None)
        self.start(handler)
        for conversation in ("c-aaaaaa", "c-bbbbbb"):
            turn = self.desktop.ask("hi", conversation=conversation, agent_token=conversation * 4)
            self.desktop.wait_closed(turn, timeout=8)
        a = handler.mind("c-aaaaaa").proc.proc
        b = handler.mind("c-bbbbbb").proc.proc
        self.assertIsNone(a.poll())

        self.desktop.stop_agent(conversation="c-aaaaaa")
        self.assertTrue(wait_for(lambda: a.poll() is not None, timeout=8), "pi was left running")
        self.assertIsNone(b.poll(), "the other conversation's pi was stopped too")
        self.assertEqual(handler.held(), ["c-bbbbbb"])

    def test_what_the_desktop_has_to_tell_the_agent_goes_in_front_of_its_prompt_once(self):
        # Pi is shown nothing else of a turn's context, so a note about a command that finished
        # after its call returned would otherwise never reach the model.
        self.start(PiMind(self.config("text"), log=lambda message: None))
        note = "Your command `make` (job job-1) finished after the call that started it had returned: exit code 0."
        first = self.desktop.ask("what now?", context=json.dumps(
            {"machine": {"timezone": "UTC"}, "notes": [note]}))
        self.desktop.wait_closed(first, timeout=8)
        second = self.desktop.ask("and then?", context=json.dumps({"machine": {"timezone": "UTC"}}))
        self.desktop.wait_closed(second, timeout=8)
        prompts = [p["message"] for p in self.dump("prompts")]
        self.assertEqual(prompts, ["[From the desktop, since your last turn:\n- %s]\n\nwhat now?" % note,
                                   "and then?"])

    def test_the_token_is_never_inherited_from_the_harness_itself(self):
        os.environ["YANTRIK_AGENT_TOKEN"] = "f" * 32
        self.addCleanup(os.environ.pop, "YANTRIK_AGENT_TOKEN", None)
        self.start(PiMind(self.config("text"), log=lambda message: None))
        self.desktop.wait_closed(self.desktop.ask("hi"), timeout=8)
        self.assertEqual([p["token"] for p in self.dump("env")], [None])

    def test_pi_runs_in_a_directory_of_the_desktops_own_never_in_the_home_folder(self):
        # pi reads the instruction files — CLAUDE.md, AGENTS.md — of its working directory and
        # of every parent, and the harness itself runs in $HOME, so a brief a person left at
        # ~/CLAUDE.md for their own coding work would steer the desktop's mind (#183). Each
        # conversation's pi starts in its own directory under the desktop's data directory.
        data = tempfile.mkdtemp(prefix="pi-data-")
        previous = os.environ["XDG_DATA_HOME"]
        os.environ["XDG_DATA_HOME"] = data
        self.addCleanup(os.environ.__setitem__, "XDG_DATA_HOME", previous)
        self.start(yantrik_pi.handler(self.config("text"), log=lambda message: None))
        for conversation in ("c-aaaaaa", "c-bbbbbb"):
            turn = self.desktop.ask("hi", conversation=conversation,
                                    agent_token=conversation * 4)
            self.desktop.wait_closed(turn, timeout=8)
        started = self.dump("env")
        self.assertEqual(len(started), 2, started)
        minds = os.path.join(data, "yantrik", "minds", "pi")
        home = os.path.realpath(os.path.expanduser("~"))
        self.assertEqual(sorted(os.path.realpath(p["cwd"]) for p in started),
                         sorted(os.path.realpath(os.path.join(minds, c))
                                for c in ("c-aaaaaa", "c-bbbbbb")))
        for process in started:
            cwd = os.path.realpath(process["cwd"])
            self.assertTrue(cwd.startswith(minds + os.sep), cwd)
            self.assertNotEqual(cwd, home)


class PiConfigTests(unittest.TestCase):
    def test_the_silence_budget_exceeds_the_longest_an_approval_card_can_wait(self):
        # An os_act above the ceiling waits ~270s for the person inside a single tool call, with
        # no events at all. A shorter budget would fail turns that were working correctly.
        self.assertGreater(yantrik_pi.DEFAULT_SILENCE_TIMEOUT, 300)

    def test_pis_own_tools_are_off_unless_the_person_turns_them_on(self):
        self.assertFalse(PiConfig({}).builtin_tools)
        self.assertNotIn("--no-builtin-tools", PiConfig({"builtin_tools": True}).argv())

    def test_the_extension_is_passed_with_e_rather_than_copied_anywhere(self):
        argv = PiConfig({"extension": "/opt/yantrik/share/harnesses/pi/extension/yantrik-os.ts",
                         "provider": "ollama", "model": "ollama/deepseek"}).argv()
        self.assertEqual(argv[argv.index("-e") + 1],
                         "/opt/yantrik/share/harnesses/pi/extension/yantrik-os.ts")
        self.assertEqual(argv[argv.index("--provider") + 1], "ollama")
        self.assertEqual(argv[argv.index("--model") + 1], "ollama/deepseek")

    def test_the_extension_beside_this_checkout_is_the_default(self):
        self.assertTrue(PiConfig({}).extension.endswith(os.path.join("extension", "yantrik-os.ts")))

    def test_a_per_user_node_install_can_be_found_by_setting_path(self):
        # A user service does not get a login shell's PATH, and pi and node are usually in
        # ~/.npm-global/bin and ~/.local/node/bin.
        config = PiConfig({"path": "/home/me/.npm-global/bin:/home/me/.local/node/bin"})
        self.assertTrue(config.environ()["PATH"].startswith("/home/me/.npm-global/bin"))

    def test_a_missing_config_is_pis_own_defaults_not_a_failure(self):
        config = load_config(os.path.join(tempfile.mkdtemp(), "pi.json"))
        self.assertEqual(config.command, ["pi"])
        self.assertEqual(config.provider, "")

    def test_a_command_can_be_written_as_a_string(self):
        self.assertEqual(PiConfig({"command": "npx -y pi"}).command, ["npx", "-y", "pi"])


if __name__ == "__main__":
    unittest.main()
