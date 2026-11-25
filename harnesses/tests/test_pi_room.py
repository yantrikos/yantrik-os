"""Pi is told a changed model room without losing its conversation (#1402)."""

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
from yantrik_pi import PiConfig, PiMind
from pi_room import COMMAND, RoomReload


class RoomReloadTests(unittest.TestCase):
    def setUp(self):
        self.desktop = FakeDesktop()
        self.addCleanup(self.desktop.stop)
        self.work = tempfile.mkdtemp(prefix="fake-pi-room-")
        self.agent = os.path.join(self.work, "agent")
        os.makedirs(self.agent)
        self.models = os.path.join(self.agent, "models.json")
        self.settings = os.path.join(self.agent, "settings.json")
        for path in (self.models, self.settings):
            with open(path, "w") as f:
                f.write("{}")
        self.config = PiConfig({
            "silence_timeout": 10.0, "settled_grace": 0.3, "extension": "",
            "command": [sys.executable, FAKE_PI, "text"],
            "env": {"PI_CODING_AGENT_DIR": self.agent,
                    "FAKE_PI_PROMPTS_DUMP": os.path.join(self.work, "prompts"),
                    "FAKE_PI_ARGV_DUMP": os.path.join(self.work, "argv")}})
        mind = PiMind(self.config, log=lambda message: None)
        self.addCleanup(mind.close)
        harness = Harness("pi", "Pi", mind, address=self.desktop.path, tools=True, poll_interval=0.02,
                          retry_seconds=0.2, heartbeat_seconds=30.0, log=lambda message: None)
        thread = threading.Thread(target=harness.run, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(harness.stop)
        self.assertTrue(wait_for(lambda: bool(self.desktop.attachments)), "never attached")

    def prompts(self):
        path = os.path.join(self.work, "prompts")
        if not os.path.exists(path):
            return []
        with open(path) as f:
            return [json.loads(line) for line in f if line.strip()]

    def turn(self, text):
        self.desktop.wait_closed(self.desktop.ask(text), timeout=8)

    def rewrite(self):
        with open(self.models, "w") as f:
            f.write('{"providers": {}}')

    def test_the_command_is_sent_between_turns_only_when_the_files_changed_and_pi_is_not_restarted(self):
        self.turn("one")
        self.turn("two")
        self.assertEqual([p["message"] for p in self.prompts()], ["one", "two"], "nothing changed, nothing sent")
        self.rewrite()
        self.turn("three")
        self.turn("four")
        sent = self.prompts()
        self.assertEqual([p["message"] for p in sent], ["one", "two", COMMAND, "three", "four"])
        self.assertEqual(len({p["pid"] for p in sent}), 1, "the same Pi process: its conversation is kept")
        self.assertLess(sent[2]["at"], sent[3]["at"], "the command is answered before the next turn is sent")

    def test_a_pi_that_has_just_started_has_read_the_files_and_is_sent_nothing(self):
        self.rewrite()
        self.turn("first")
        self.assertEqual([p["message"] for p in self.prompts()], ["first"])

    def test_the_extension_that_answers_the_command_is_loaded(self):
        self.turn("hi")
        with open(os.path.join(self.work, "argv")) as f:
            argv = f.read()
        self.assertIn("yantrik-room.ts", argv)

    def test_a_pi_without_the_extension_is_never_sent_the_command(self):
        room = RoomReload({"PI_CODING_AGENT_DIR": self.agent}, enabled=False)
        room.before_turn(False, lambda c: self.fail("sent"), lambda m: None)
        self.rewrite()
        room.before_turn(True, lambda c: self.fail("sent"), lambda m: None)

    def test_a_pi_that_never_answers_does_not_hold_the_turn_for_ever(self):
        room = RoomReload({"PI_CODING_AGENT_DIR": self.agent}, enabled=True)
        room.before_turn(False, lambda c: 1, lambda m: None)
        self.rewrite()
        said = []
        import pi_room
        pi_room.WAIT = 0.2
        self.addCleanup(setattr, pi_room, "WAIT", 20.0)
        room.before_turn(True, lambda c: 7, said.append)
        self.assertEqual(len(said), 1)
        self.assertFalse(room.saw({"type": "response", "id": "8"}), "someone else's response")
        self.assertTrue(room.saw({"type": "response", "id": "7"}))


if __name__ == "__main__":
    unittest.main()
