"""How much a fresh Pi's first model call carries of ours, held to a budget (#1398).

A free account takes about 8,000 tokens a minute (Groq, `openai/gpt-oss-20b`), and Pi's first call,
before the person has said a word, carried 6,790 of them: Pi's own prompt, our appended prompt and
the schema of every tool. One call over the limit ends the turn. This runs the real extension
under node against the real `yos-mcp` (as the harness starts it, `YOS_MCP_BRIEF=1`) and against the
four memory tools exactly as the engine publishes them (`fixtures/yantrikdb_tools.json`, from
yantrikdb-mcp 0.24.0, 12.4 KB on their own), and adds up what Pi would send:

    Pi's own system prompt + the appended desktop prompt + every registered tool as the OpenAI shape.

The budget is half the smallest per-request limit Pi is offered on: 16,000 bytes, about 4,000 tokens
at 4 bytes each. It fails when a tool, a description or the prompt grows past it, so this cannot
creep back. Skipped where there is no node that can run TypeScript, as `test_pi_extension.py` is.
"""

import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import support  # noqa: F401  (puts the harness modules on the path)
import yantrik_pi
from test_pi_extension import DRIVER, EXTENSION, NODE, TYPEBOX, node_runs_typescript

HERE = Path(__file__).resolve().parent
BRIDGE = HERE.parent.parent / "deploy" / "yantrik-os" / "yos-mcp"
MEMORY_TOOLS = HERE / "fixtures" / "yantrikdb_tools.json"

# Pi 0.99.1's own system prompt with its built-in tools off, measured with a capturing upstream: 1,851
# bytes before ours is appended. A Pi that grows its own is a new measurement, not a failure here.
PI_OWN_PROMPT_BYTES = 1900
# Half the smallest per-request limit Pi is offered on (Groq free, 8,000 tokens) at 4 bytes a token.
BUDGET_BYTES = 16000
# One of the person's agents (a token for the bridge) is offered 14 more tools and a `bash`. Held at
# what it measured when this was written, so it cannot grow; it is not yet under BUDGET_BYTES.
AGENT_BUDGET_BYTES = 21500

DUMP = """
import { pathToFileURL } from "node:url";
const mod = await import(pathToFileURL(process.argv[2]).href);
const tools = [];
mod.default({ registerTool(t) { tools.push({ name: t.name, description: t.description, parameters: t.parameters }); } });
console.log(JSON.stringify(tools));
process.exit(0);
"""


@unittest.skipUnless(sys.platform != "win32" and node_runs_typescript(),
                     "needs a node that can strip TypeScript types (22.6 or later)")
class PiFirstCallBudget(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp(prefix="pi-budget-"))
        self.addCleanup(shutil.rmtree, self.work, True)
        (self.work / "package.json").write_text('{"type": "module"}', encoding="utf-8")
        typebox = self.work / "node_modules" / "typebox"
        typebox.mkdir(parents=True)
        (typebox / "package.json").write_text('{"name": "typebox", "type": "module", "exports": "./index.js"}', encoding="utf-8")
        (typebox / "index.js").write_text(TYPEBOX, encoding="utf-8")
        shutil.copy(EXTENSION, self.work / "yantrik-os.ts")
        (self.work / "dump.mjs").write_text(DUMP, encoding="utf-8")
        self.memory = self.script("memory", "import json,sys\n"
                                  "for l in sys.stdin:\n"
                                  "    m=json.loads(l)\n"
                                  "    if m.get('method')=='tools/list': print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':{'tools':json.load(open(%r))}}),flush=True)\n"
                                  "    elif m.get('method')=='initialize': print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':{'protocolVersion':'2024-11-05','capabilities':{},'serverInfo':{'name':'m','version':'1'}}}),flush=True)\n" % str(MEMORY_TOOLS))
        self.bridge = self.script("bridge", "import os,runpy,sys\nsys.argv=[%r]\nrunpy.run_path(%r, run_name='__main__')\n" % (str(BRIDGE), str(BRIDGE)))

    def script(self, name, source):
        (self.work / (name + ".py")).write_text(source, encoding="utf-8")
        path = self.work / name
        path.write_text("#!/bin/sh\nexec '%s' '%s'\n" % (sys.executable, self.work / (name + ".py")), encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IEXEC)
        return str(path)

    def tools(self, agent=False, brief=True):
        env = {k: v for k, v in os.environ.items() if not k.startswith("YANTRIK_")}
        env.update(yantrik_pi.PiConfig({"yos_mcp": self.bridge, "extension": str(EXTENSION)}).environ())
        env.update(YOS_MCP_BIN=self.bridge, YANTRIKDB_MCP_BIN=self.memory)
        env.pop("YOS_MCP_BRIEF", None) if not brief else None
        if agent:
            env["YANTRIK_AGENT_TOKEN"] = "x"
        done = subprocess.run([NODE, "--experimental-strip-types", "--no-warnings", str(self.work / "dump.mjs"),
                               str(self.work / "yantrik-os.ts"), "--no-builtin-tools"],
                              capture_output=True, text=True, timeout=120, env=env, cwd=str(self.work))
        self.assertEqual(done.returncode, 0, done.stderr)
        return json.loads(done.stdout.strip().splitlines()[-1])

    @staticmethod
    def first_call_bytes(tools):
        """What Pi puts in a first request beside the person's message: the prompts and the tools."""
        shaped = [{"type": "function", "function": t} for t in tools]
        return PI_OWN_PROMPT_BYTES + len(yantrik_pi.DESKTOP_PROMPT.encode()) + len(json.dumps(shaped, separators=(",", ":")).encode())

    def test_a_fresh_pi_first_call_is_at_most_half_the_smallest_accounts_limit(self):
        tools = self.tools()
        self.assertIn("memory_remember", [t["name"] for t in tools], "the memory tools are in the count")
        size = self.first_call_bytes(tools)
        self.assertLessEqual(size, BUDGET_BYTES, "Pi's first call is %d bytes (%d tokens at 4 bytes), over the %d-byte budget: "
                             "shorten yos_mcp_brief.py, MEMORY_SHOWN or the desktop prompt, not this number" % (size, size // 4, BUDGET_BYTES))

    def test_one_of_the_persons_agents_does_not_grow_either(self):
        size = self.first_call_bytes(self.tools(agent=True))
        self.assertLessEqual(size, AGENT_BUDGET_BYTES, "an agent's first call is %d bytes, over %d" % (size, AGENT_BUDGET_BYTES))

    def test_the_budget_has_teeth_the_bridges_long_descriptions_alone_are_over_it(self):
        self.assertGreater(self.first_call_bytes(self.tools(brief=False)), BUDGET_BYTES)

    def test_the_memory_tools_keep_their_names_and_the_arguments_a_mind_uses(self):
        by = {t["name"]: t for t in self.tools()}
        for name, needed in {"memory_remember": ["text"], "memory_recall": ["query"], "memory_forget": ["rid"], "memory_correct": ["rid", "reason", "new_text"]}.items():
            props = by[name]["parameters"]["properties"]
            for arg in needed:
                self.assertIn(arg, props, "%s keeps %s" % (name, arg))
        self.assertEqual(by["memory_correct"]["parameters"]["required"], ["rid", "reason"])
        self.assertNotIn("description", json.dumps(by["memory_remember"]["parameters"]), "no schema prose is sent")


class BriefWordsKeepTheRules(unittest.TestCase):
    """The short words are a second copy of the long ones, so they are held to the same tools and rules."""

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(BRIDGE.parent))
        import yos_mcp_brief
        cls.brief = yos_mcp_brief
        env = dict(os.environ, YANTRIK_AGENT_TOKEN="x")
        env.pop("YOS_MCP_BRIEF", None)
        lines = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}\n{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n'
        out = subprocess.run([sys.executable, str(BRIDGE)], input=lines, capture_output=True, text=True, timeout=60, env=env).stdout
        cls.long = {t["name"]: t for l in out.splitlines() for t in json.loads(l).get("result", {}).get("tools", [])}

    def test_every_tool_the_bridge_publishes_has_short_words_and_none_is_left_over(self):
        self.assertEqual(sorted(self.long), sorted(self.brief.BRIEF))

    def test_a_short_description_is_shorter_and_keeps_the_arguments(self):
        for name, tool in self.long.items():
            short = self.brief.brief(tool)
            self.assertLess(len(short["description"]), len(tool["description"]), name)
            self.assertEqual(short["inputSchema"].get("required"), tool["inputSchema"].get("required"), name)
            self.assertEqual(sorted(short["inputSchema"].get("properties", {})), sorted(tool["inputSchema"].get("properties", {})), name)

    def test_the_rules_that_protect_the_person_are_still_said(self):
        said = {n: self.brief.BRIEF[n][0] for n in self.brief.BRIEF}
        for name, words in {
            "os_act": ["`ask`", "card", "`plan`", "`bypass_all`", "cannot be undone", "environment variable", "report"],
            "web_commit": ["cannot be undone", "card", "every time"],
            "web_click": ["commitment", "web_commit"],
            "run_command": ["sensitive", "`ask`", "card", "hooks"],
            "run_command_with_reach": ["every time", "full bypass"],
            "run_sealed_command": ["no network", "tell the person"],
            "command_input": ["sensitive"], "workshop_request": ["card", "sensitive"], "workshop_save": ["card"],
            "new_agent": ["none of your grants", "sensitive"], "hand_off": ["none of your grants", "sensitive"],
            "web_console": ["not instructions"], "web_screenshot": ["not instructions"],
        }.items():
            for word in words:
                self.assertIn(word, said[name], "%s must still say %r" % (name, word))
        self.assertIn("run_command", self.brief.brief(dict(self.long["os_act"]), agent=True)["description"])


if __name__ == "__main__":
    unittest.main()
