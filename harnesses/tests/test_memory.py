"""YantrikDB as every mind's memory (#677) and the three install proofs.

- `WithMemory`: the desktop's tools and the memory tools as one set, the memory tools renamed so
  they can never shadow a desktop tool, and every remember stamped with the harness that wrote it.
- `proofs.py`: "Can it answer?", "Can it remember?", "Can it reach the desktop?", each seen, and
  a failure always carries its one-press fix.
- Pi's extension registers the memory tools, one fresh server per call, provenance stamped.

The memory server is `fake_memory_mcp.py`, which keeps what it is told in a JSON file, so a server
started per call still remembers. Stdlib only, offline.
"""

import importlib.util
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import support  # noqa: F401  (puts harnesses/lib on the path)
import yantrik_harness
from yantrik_harness import McpTools, WithMemory, memory_on, stamped

HERE = Path(__file__).resolve().parent
FAKE_MEMORY = str(HERE / "fake_memory_mcp.py")
PROOFS = HERE.parent / "lib" / "proofs.py"
_spec = importlib.util.spec_from_file_location("proofs", PROOFS)
proofs = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(proofs)


class FakeDesktop:
    def __init__(self):
        self.calls = []

    def list(self, refresh=False):
        return [{"name": "os_apps", "inputSchema": {"type": "object", "properties": {}}}]

    def as_openai_tools(self):
        return [{"type": "function", "function": {"name": "os_apps", "parameters": {}}}]

    def call(self, name, arguments=None, timeout=None):
        self.calls.append((name, arguments))
        return ("desktop says hi", False)


class MemoryCase(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp(prefix="yantrik-memory-"))
        self.addCleanup(shutil.rmtree, self.work, True)
        self.store = self.work / "memory.json"
        os.environ["FAKE_MEMORY_FILE"] = str(self.store)
        self.addCleanup(os.environ.pop, "FAKE_MEMORY_FILE", None)

    def memory(self):
        tools = McpTools("test", "1", command=[sys.executable, FAKE_MEMORY], timeout=30,
                         log=lambda _m: None)
        self.addCleanup(tools.close)
        return tools

    def rows(self):
        return json.loads(self.store.read_text(encoding="utf-8")) if self.store.exists() else []


class WithMemoryTests(MemoryCase):
    def test_the_memory_tools_are_offered_renamed_and_only_the_four_a_mind_needs(self):
        tools = WithMemory(FakeDesktop(), self.memory(), "deepseek")
        names = [t["function"]["name"] for t in tools.as_openai_tools()]
        self.assertEqual(names, ["os_apps", "memory_remember", "memory_recall", "memory_forget",
                                 "memory_correct"])
        self.assertNotIn("memory_think", names, "the store's administration is not a mind's")

    def test_every_remember_carries_the_harness_whatever_the_model_said(self):
        tools = WithMemory(FakeDesktop(), self.memory(), "deepseek")
        text, error = tools.call("memory_remember", {"text": "tea not coffee", "source": "user",
                                                     "metadata": {"provenance": "mind"}})
        self.assertFalse(error, text)
        [row] = self.rows()
        self.assertEqual(row["args"]["source"], "deepseek")
        self.assertEqual(row["args"]["metadata"]["provenance"], "deepseek")
        found, _ = tools.call("memory_recall", {"query": "tea"})
        self.assertIn("tea not coffee", found)

    def test_a_desktop_tool_and_an_unoffered_memory_tool_go_to_the_desktop(self):
        desktop = FakeDesktop()
        tools = WithMemory(desktop, self.memory(), "pi")
        tools.call("os_apps", {})
        tools.call("memory_think", {})
        self.assertEqual([c[0] for c in desktop.calls], ["os_apps", "memory_think"])
        self.assertEqual(self.rows(), [])

    def test_a_batch_of_memories_is_stamped_one_by_one(self):
        args = stamped({"memories": [{"text": "a"}, {"text": "b", "metadata": "junk"}]}, "hermes")
        self.assertEqual([m["source"] for m in args["memories"]], ["hermes", "hermes"])
        self.assertEqual(args["memories"][1]["metadata"], {"provenance": "hermes"})

    def test_memory_is_on_by_default_only_where_the_image_carries_it(self):
        saved = yantrik_harness.YANTRIKDB_MCP
        self.addCleanup(setattr, yantrik_harness, "YANTRIKDB_MCP", saved)
        yantrik_harness.YANTRIKDB_MCP = FAKE_MEMORY
        self.assertTrue(memory_on(None))
        self.assertTrue(memory_on("yantrikdb"))
        self.assertFalse(memory_on("off"))
        self.assertFalse(memory_on(False))
        yantrik_harness.YANTRIKDB_MCP = str(self.work / "not-here")
        self.assertFalse(memory_on(None))


class ProofTests(MemoryCase):
    def setUp(self):
        super().setUp()
        saved = os.environ.get("HOME")
        os.environ["HOME"] = str(self.work)
        self.addCleanup(os.environ.__setitem__, "HOME", saved or "/")
        self.state = self.work / ".local" / "state"

    def test_a_harness_whose_model_was_not_wired_keeps_its_own_and_that_is_not_a_failure(self):
        row = proofs.prove_answer("pi")
        self.assertTrue(row["ok"])
        self.assertEqual((row["status"], row["says"]), ("own", "model: its own"))

    def test_a_wiring_that_failed_is_a_failure_with_its_fix(self):
        (self.state / "yantrik" / "harnesses" / "pi").mkdir(parents=True)
        (self.state / "yantrik" / "harnesses" / "pi" / "model").write_text("failed\n")
        row = proofs.prove_answer("pi")
        self.assertFalse(row["ok"])
        self.assertEqual(row["fix"]["action"], "configure")

    def wired(self, harness):
        (self.state / "yantrik" / "harnesses" / harness).mkdir(parents=True, exist_ok=True)
        (self.state / "yantrik" / "harnesses" / harness / "model").write_text("picked\n")

    def test_a_wired_harness_answers_through_the_gateway_with_its_own_token(self):
        # #675's model step writes the token into the harness's own config; the proof makes one
        # real call as that harness, through a stand-in gateway that checks the bearer token.
        import http.server
        import threading

        token = "ygw-" + "ab" * 32
        seen = {}

        class Gateway(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                seen["auth"] = self.headers.get("Authorization")
                seen["body"] = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                reply = json.dumps({"model": "deepseek-v4-pro",
                                    "choices": [{"message": {"content": "OK"}}]}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(reply)))
                self.end_headers()
                self.wfile.write(reply)

            def log_message(self, *args):
                pass

        server = http.server.HTTPServer(("127.0.0.1", 0), Gateway)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        saved = os.environ.get("YANTRIK_GATEWAY_URL")
        os.environ["YANTRIK_GATEWAY_URL"] = "http://127.0.0.1:%d/v1" % server.server_port
        self.addCleanup(lambda: os.environ.pop("YANTRIK_GATEWAY_URL", None) if saved is None
                        else os.environ.__setitem__("YANTRIK_GATEWAY_URL", saved))

        self.wired("deepseek")
        config = self.work / ".config" / "yantrik"
        config.mkdir(parents=True)
        (config / "deepseek.json").write_text(json.dumps({"base_url": "http://127.0.0.1:7460/v1",
                                                         "api_key": token, "model": "picked"}))
        row = proofs.prove_answer("deepseek")
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["says"], "connected · deepseek-v4-pro · checked with one short call to your account")
        self.assertEqual(seen["auth"], "Bearer " + token)
        self.assertEqual(seen["body"]["model"], "picked")
        self.assertNotIn(token, json.dumps(row), "the token is never in what the row shows")

    def test_a_wired_harness_with_no_token_or_a_refusal_fails_with_its_fix(self):
        self.wired("pi")
        row = proofs.prove_answer("pi", call=lambda _t: {"model": "x"})
        self.assertFalse(row["ok"])
        self.assertIn("no gateway token", row["says"])
        agent = self.work / ".pi" / "agent"
        agent.mkdir(parents=True)
        (agent / "models.json").write_text('{"providers": {"yantrik": {"apiKey": "ygw-%s"}}}' % ("0" * 64))
        row = proofs.prove_answer("pi", call=lambda _t: {"error": "No model is picked for this mind yet"})
        self.assertFalse(row["ok"])
        self.assertEqual(row["fix"]["action"], "configure")
        self.assertIn("No model is picked", row["says"])
        row = proofs.prove_answer("pi", call=lambda _t: {
            "error": "Set up a private lane, or allow sharing with Groq in AI accounts.",
            "code": "private_context_not_shared"})
        self.assertFalse(row["ok"])
        self.assertEqual(row["fix"], proofs.SHARING_FIX, "a refused share is fixed in AI accounts")

    def test_the_gateway_address_is_only_ever_loopback(self):
        self.assertEqual(proofs.gateway_url(""), proofs.GATEWAY_DEFAULT)
        for good in ("http://127.0.0.1:7460/v1", "http://127.5.6.7:9000/v1", "http://[::1]:7460/v1"):
            self.assertEqual(proofs.gateway_url(good), good)
        for bad in ("http://evil.example:7460/v1", "https://127.0.0.1:7460/v1", "http://localhost:7460/v1",
                    "http://user:pw@127.0.0.1:7460/v1", "http://192.168.4.28:7460/v1", "http://127.0.0.1/v1",
                    "not a url"):
            self.assertEqual(proofs.gateway_url(bad), proofs.GATEWAY_DEFAULT, bad)

    def test_remember_writes_recalls_and_deletes_through_the_harness_memory_server(self):
        row = proofs.remember_through_mcp("pi", command=[FAKE_MEMORY], env={})
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["says"], "memory · YantrikDB · recalled")
        self.assertEqual(self.rows(), [], "the probe note is deleted")

    def test_no_memory_in_the_image_is_a_failure_that_says_install_again(self):
        row = proofs.remember_through_mcp("pi", command=[str(self.work / "nothing")], env={})
        self.assertFalse(row["ok"])
        self.assertEqual(row["fix"], proofs.INSTALL_FIX)

    def test_paired_is_the_desktops_word_after_an_attach(self):
        seen = iter(["not listed", "ready", "attached"])
        row = proofs.prove_desktop("deepseek", 30, state=lambda _h: next(seen), sleep=lambda _s: None)
        self.assertTrue(row["ok"])
        self.assertEqual(row["says"], "paired")

    def test_no_attach_is_a_failure_with_a_restart_for_that_harness(self):
        row = proofs.prove_desktop("hermes", 0, state=lambda _h: "ready", sleep=lambda _s: None)
        self.assertFalse(row["ok"])
        self.assertEqual(row["fix"]["command"], "systemctl --user restart hermes-gateway")
        self.assertEqual(proofs.restart_fix("pi")["command"], "systemctl --user restart yantrik-pi.service")

    def test_the_rows_use_the_experience_words(self):
        self.assertEqual((proofs.ANSWER, proofs.REMEMBER, proofs.DESKTOP),
                         ("Can it answer?", "Can it remember?", "Can it reach the desktop?"))

    def test_the_json_document_is_what_the_settings_row_reads(self):
        out = subprocess.run([sys.executable, str(PROOFS), "pi", "--json", "--only", "answer"],
                             capture_output=True, text=True, timeout=60,
                             env=dict(os.environ, HOME=str(self.work)))
        doc = json.loads(out.stdout)
        self.assertEqual(out.returncode, 0)
        self.assertEqual(doc["harness"], "pi")
        self.assertTrue(doc["passed"])
        self.assertEqual(set(doc["proofs"][0]), {"id", "question", "ok", "status", "says", "fix"})


class InstallerTests(unittest.TestCase):
    """What every installer must do, read from the scripts themselves."""

    INSTALL = HERE.parent / "lib" / "install"

    def test_every_installer_wires_the_model_step_and_ends_with_the_proofs(self):
        for name in ("hermes", "openclaw", "pi", "deepseek"):
            script = (self.INSTALL / ("%s.sh" % name)).read_text(encoding="utf-8")
            self.assertIn("wire_model %s" % name, script, name)
            self.assertIn("prove %s" % name, script, name)
            self.assertLess(script.index("wire_model %s" % name), script.index("prove %s" % name))

    def test_no_installer_allows_every_user_or_a_wildcard(self):
        for path in self.INSTALL.glob("*.sh"):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("ALLOW_ALL_USERS true", text, path.name)
            self.assertNotIn("ALLOW_ALL_USERS=true", text, path.name)
            self.assertNotIn("ALLOWED_USERS *", text, path.name)

    def test_wire_model_is_skipped_with_a_line_when_the_model_half_is_absent(self):
        work = tempfile.mkdtemp(prefix="wire-model-")
        self.addCleanup(shutil.rmtree, work, True)
        lib = Path(work) / "install"
        lib.mkdir()
        shutil.copy(self.INSTALL / "common.sh", lib / "common.sh")
        (lib / "probe.sh").write_text('. "$(dirname "$0")/common.sh"\nwire_model pi\n')
        env = dict(os.environ, HOME=work, YANTRIK_UI=str(Path(work) / "no-such-desktop"))
        done = subprocess.run(["sh", str(lib / "probe.sh")], capture_output=True, text=True, env=env)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("model: its own", done.stdout)
        self.assertFalse((Path(work) / ".local" / "state" / "yantrik" / "harnesses" / "pi" / "model").exists())
        # The model half (#675): the desktop's `yantrik-ui --wire-model`, here a stand-in.
        ui = Path(work) / "yantrik-ui"
        ui.write_text("#!/bin/sh\n# --wire-model\necho \"$2 now uses your model\"\n")
        ui.chmod(0o700)
        env["YANTRIK_UI"] = str(ui)
        done = subprocess.run(["sh", str(lib / "probe.sh")], capture_output=True, text=True, env=env)
        self.assertIn("model: pi now uses your model", done.stdout)
        self.assertEqual((Path(work) / ".local" / "state" / "yantrik" / "harnesses" / "pi" / "model").read_text(),
                         "picked\n")


def node_runs_typescript() -> bool:
    node = shutil.which("node")
    if not node or sys.platform == "win32":
        return False
    try:
        probe = subprocess.run([node, "--experimental-strip-types", "--no-warnings", "-e", "0"],
                               capture_output=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return False
    return probe.returncode == 0


PI_DRIVER = """
import { pathToFileURL } from "node:url";
const mod = await import(pathToFileURL(process.argv[2]).href);
const tools = {};
mod.default({ registerTool(tool) { tools[tool.name] = tool; } });
const out = { names: Object.keys(tools).sort(), results: [] };
if (tools.memory_remember) {
  out.results.push(await tools.memory_remember.execute("1", { text: "the kettle is blue", source: "user" }));
  out.results.push(await tools.memory_recall.execute("2", { query: "kettle" }));
}
console.log(JSON.stringify(out));
process.exit(0);
"""


@unittest.skipUnless(node_runs_typescript(), "needs a node that can strip TypeScript types (22.6 or later)")
class PiMemoryTests(MemoryCase):
    def test_pi_gets_the_memory_tools_one_server_per_call_with_pi_as_the_source(self):
        (self.work / "package.json").write_text('{"type": "module"}')
        typebox = self.work / "node_modules" / "typebox"
        typebox.mkdir(parents=True)
        (typebox / "package.json").write_text('{"name": "typebox", "type": "module", "exports": "./index.js"}')
        (typebox / "index.js").write_text("export const Type = { Unsafe: (s) => s, Object: (p) => ({ type: 'object', properties: p }), String: () => ({ type: 'string' }), Number: () => ({ type: 'number' }), Optional: (s) => s };\n")
        shutil.copy(HERE.parent / "pi" / "extension" / "yantrik-os.ts", self.work / "yantrik-os.ts")
        (self.work / "driver.mjs").write_text(PI_DRIVER)
        memory = self.work / "memory-server"
        memory.write_text("#!/bin/sh\nexec '%s' '%s'\n" % (sys.executable, FAKE_MEMORY))
        memory.chmod(memory.stat().st_mode | stat.S_IEXEC)
        env = dict(os.environ, YOS_MCP_BIN=str(self.work / "no-bridge"), YANTRIKDB_MCP_BIN=str(memory),
                   YANTRIK_MEMORY_SOURCE="pi")
        done = subprocess.run([shutil.which("node"), "--experimental-strip-types", "--no-warnings",
                               str(self.work / "driver.mjs"), str(self.work / "yantrik-os.ts")],
                              capture_output=True, text=True, timeout=60, env=env, cwd=str(self.work))
        self.assertEqual(done.returncode, 0, done.stderr)
        out = json.loads(done.stdout.strip().splitlines()[-1])
        for name in ("memory_remember", "memory_recall", "memory_forget", "memory_correct"):
            self.assertIn(name, out["names"])
        self.assertNotIn("memory_think", out["names"])
        self.assertFalse(out["results"][0]["isError"], out["results"][0])
        self.assertIn("the kettle is blue", out["results"][1]["content"][0]["text"])
        [row] = self.rows()
        self.assertEqual(row["args"]["source"], "pi")
        self.assertEqual(row["args"]["metadata"]["provenance"], "pi")


if __name__ == "__main__":
    unittest.main()
