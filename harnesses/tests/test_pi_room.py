"""Pi is told a changed model room without losing its conversation (#1402)."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

import support
from support import FAKE_PI, FakeDesktop, wait_for

from yantrik_harness import Harness
from yantrik_pi import PiConfig, PiMind
from pi_room import COMMAND, RoomReload
from fake_openai import Upstream
import yantrik_pi


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


REPO = Path(__file__).resolve().parent.parent.parent
PI = os.environ.get("PI_0991_BIN") or shutil.which("pi") or ""


def pi_is_0991():
    try:
        return subprocess.run([PI, "--version"], capture_output=True, text=True, timeout=30).stdout.strip().endswith("0.99.1") if PI else False
    except (OSError, subprocess.SubprocessError):
        return False


@unittest.skipUnless(pi_is_0991(), "needs Pi 0.99.1 (PI_0991_BIN, or `pi` on the PATH)")
class RealPiAnswersInASmallRoom(unittest.TestCase):
    """Pi 0.99.1 itself, started with the harness's real argv (both extensions, the desktop's tools and
    the memory tools as the engine publishes them), told the room of Groq's free tier by the numbers
    the handoff writes (contextWindow 131,072, maxTokens 2,000, compaction overrides of 125,072 and
    1,000), against an upstream that takes 8,000 tokens and stops an answer at its `max_tokens`.
    Round 6: told 8,000 for the window, Pi's own 4,096 margin left it ONE token to answer with."""

    def test_every_call_has_room_to_answer_and_a_turn_with_a_tool_completes(self):
        work = Path(tempfile.mkdtemp(prefix="pi-room-real-"))
        self.addCleanup(shutil.rmtree, work, True)
        upstream = Upstream(8000)
        self.addCleanup(upstream.stop)
        agent = work / "home" / ".pi" / "agent"
        agent.mkdir(parents=True)
        room, window, reserve = 8000, 131072, 131072 - 6000
        (agent / "models.json").write_text(json.dumps({"providers": {"yantrik": {"baseUrl": upstream.url, "api": "openai-completions", "apiKey": "fake-test-token", "models": [
            {"id": "picked", "name": "picked", "reasoning": False, "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}, "contextWindow": window, "maxTokens": room // 4}]}}}))
        (agent / "settings.json").write_text(json.dumps({"compaction": {"modelOverrides": {"yantrik/picked": {"reserveTokens": reserve, "keepRecentTokens": room // 8}}}}))

        def script(name, source):
            (work / (name + ".py")).write_text(source)
            (work / name).write_text("#!/bin/sh\nexec '%s' '%s'\n" % (sys.executable, work / (name + ".py")))
            (work / name).chmod(0o755)
            return str(work / name)
        tools = REPO / "harnesses" / "tests" / "fixtures" / "yantrikdb_tools.json"
        memory = script("memory", "import json,sys\nfor l in sys.stdin:\n    m=json.loads(l)\n    if m.get('method')=='tools/list': print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':{'tools':json.load(open(%r))}}),flush=True)\n    elif m.get('method')=='initialize': print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':{'protocolVersion':'2024-11-05','capabilities':{},'serverInfo':{'name':'m','version':'1'}}}),flush=True)\n    elif m.get('method')=='tools/call': print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':{'content':[{'type':'text','text':'nothing found'}]}}),flush=True)\n" % str(tools))
        bridge_path = str(REPO / "deploy" / "yantrik-os" / "yos-mcp")
        bridge = script("bridge", "import runpy,sys\nsys.argv=[%r]\nrunpy.run_path(%r, run_name='__main__')\n" % (bridge_path, bridge_path))
        config = yantrik_pi.PiConfig({"command": [PI], "provider": "yantrik", "model": "picked", "yos_mcp": bridge})
        env = config.environ()
        env.update(HOME=str(work / "home"), PI_OFFLINE="1", PI_SKIP_VERSION_CHECK="1", NO_COLOR="1", YOS_MCP_BIN=bridge, YANTRIKDB_MCP_BIN=memory)
        for key in [k for k in env if k.startswith("YANTRIK_") and k != "YANTRIK_MEMORY_SOURCE"]:
            env.pop(key)
        proc = subprocess.Popen(config.argv(), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, env=env, cwd=str(work / "home"))
        self.addCleanup(proc.terminate)
        events = []
        threading.Thread(target=lambda: [events.append(json.loads(l)) for l in proc.stdout if l.startswith("{")], daemon=True).start()

        def turn(n, text):
            seen = len(events)
            proc.stdin.write(json.dumps({"id": "t%d" % n, "type": "prompt", "message": text}) + "\n")
            proc.stdin.flush()
            end = time.time() + 90
            while time.time() < end and not any(e.get("type") == "agent_settled" for e in events[seen:]):
                time.sleep(0.2)
            return [e for e in events[seen:] if e.get("type") == "agent_settled"]
        filler = " ".join(["alpha", "bravo", "charlie", "delta"] * 56)
        for n in range(1, 11):
            before = len(upstream.log)
            self.assertTrue(turn(n, "Turn %d%s. Remember this: %s" % (n, " USE_TOOL" if n in (3, 6) else "", filler)), "turn %d never settled" % n)
            calls = upstream.log[before:]
            self.assertTrue(calls and all(c["status"] == 200 for c in calls), "turn %d: %s" % (n, calls))
            if n in (3, 6):
                self.assertEqual([c["finish"] for c in calls if not c["summary"]], ["tool_calls", "stop"], "a turn with a tool completes: %s" % calls)
        self.assertGreater(upstream.log[0]["tokens"], 3000, "the production tools are in the first call")
        asked = [c["max_tokens"] for c in upstream.log]
        self.assertGreaterEqual(min(asked), 1024, "Pi asked for %s tokens to answer with" % asked)
        self.assertNotIn("length", [c.get("finish") for c in upstream.log], "an answer was cut short")
        self.assertLessEqual(max(c["tokens"] for c in upstream.log), 8000)
        self.assertTrue(any(c["summary"] for c in upstream.log), "Pi compacted a conversation that outgrew the room")


if __name__ == "__main__":
    unittest.main()
