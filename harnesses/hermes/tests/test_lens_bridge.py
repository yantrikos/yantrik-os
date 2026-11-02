"""The Lens conversation's own bridge (#1070, item 4): routing, taint, token, cleanup.

    python3 -m unittest discover -s harnesses/hermes/tests -p test_lens_bridge.py

No Hermes and no desktop. `yos-mcp` is stood in for by a small stdio MCP server that keeps its own
"taint" (set by os_describe, as the real bridge's is per process) and can say whether it was given
a token. The plugin's routing, bridge and proxy modules are the real ones.
"""

import contextvars
import importlib.util
import json
import logging
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
import types
import unittest
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]

FAKE_YOS_MCP = textwrap.dedent(
    '''
    import json, os, sys, time
    tainted = False
    TOOLS = [{"name": n, "description": "d", "inputSchema": {"type": "object", "properties": {}}}
             for n in ("os_apps", "os_describe", "taint_status", "token_echo", "slow")]
    for line in sys.stdin:
        m = json.loads(line)
        i, method = m.get("id"), m.get("method")
        if method == "initialize":
            out = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "fake"}}
        elif method == "tools/list":
            out = {"tools": TOOLS}
        elif method == "tools/call":
            name = m["params"]["name"]
            if name == "os_describe":
                tainted = True
                text = "described"
            elif name == "taint_status":
                text = json.dumps({"tainted": tainted, "pid": os.getpid(),
                                   "has_token": "YANTRIK_AGENT_TOKEN" in os.environ})
            elif name == "token_echo":
                text = "my token is " + os.environ.get("YANTRIK_AGENT_TOKEN", "")
            elif name == "slow":
                time.sleep(2.0)
                text = "done"
            else:
                text = "ok " + name
            out = {"content": [{"type": "text", "text": text}], "isError": False}
        else:
            continue
        if i is not None:
            sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": i, "result": out}) + "\\n")
            sys.stdout.flush()
    '''
)

mods = {}


def setUpModule():
    spec = importlib.util.spec_from_file_location(
        "yantrik_lens_plugin", PLUGIN / "__init__.py", submodule_search_locations=[str(PLUGIN)]
    )
    package = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = package
    # __init__ imports the adapter, which needs Hermes; the lens modules do not, so load them alone.
    for name in ("lens_route", "lens_bridge", "lens_tools"):
        sub = importlib.util.spec_from_file_location(f"yantrik_lens_plugin.{name}", PLUGIN / f"{name}.py")
        module = importlib.util.module_from_spec(sub)
        sys.modules[sub.name] = module
        package.__path__ = [str(PLUGIN)]
        sub.loader.exec_module(module)
        mods[name] = module
    # A stand-in for Hermes's session context: the two ContextVars the proof reads.
    sc = types.ModuleType("gateway.session_context")
    sc._SESSION_KEY = contextvars.ContextVar("key", default=object())
    sc._SESSION_MESSAGE_ID = contextvars.ContextVar("mid", default=object())
    sc._SESSION_ID = contextvars.ContextVar("sid", default=object())
    gw = types.ModuleType("gateway")
    gw.session_context = sc
    mods["saved"] = {k: sys.modules.get(k) for k in ("gateway", "gateway.session_context")}
    sys.modules["gateway"], sys.modules["gateway.session_context"] = gw, sc
    mods["sc"] = sc


def tearDownModule():
    for k, v in mods["saved"].items():
        if v is None:
            sys.modules.pop(k, None)
        else:
            sys.modules[k] = v
    for k in [k for k in sys.modules if k.startswith("yantrik_lens_plugin")]:
        del sys.modules[k]


LENS_KEY = "agent:main:yantrik:dm:desktop"
LENS_SESSION = "20261102_101500_a1b2c3"
BG_SESSION = "bg_101730_d4e5f6"


class Case(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.script = Path(self.dir.name) / "fake-yos-mcp.py"
        self.script.write_text(FAKE_YOS_MCP, encoding="utf-8")
        self.command = [sys.executable, str(self.script)]
        self.lens = mods["lens_bridge"].LensBridge(self.command)
        self.shared = mods["lens_bridge"].Bridge(self.command, None)
        self.open_ids = {"7"}
        self.addCleanup(self.lens.stop)
        self.addCleanup(self.shared.stop)
        self.addCleanup(self.dir.cleanup)

    def standing(self):
        return mods["lens_route"].Standing(LENS_KEY, LENS_SESSION, frozenset(self.open_ids), self.lens.token_current)

    def shared_call(self, tool, args):
        text, is_error = self.shared.call(tool, args)
        return json.dumps({"result": text})

    def handler(self, tool):
        return mods["lens_tools"].make_handler(tool, self.lens, self.standing, self.shared_call)

    def run_as(self, key, message_id, tool, args=None, session_id=LENS_SESSION):
        """The handler as a worker thread of a turn with these session values would run it."""
        sc = mods["sc"]
        ctx = contextvars.copy_context()
        ctx.run(sc._SESSION_KEY.set, key)
        ctx.run(sc._SESSION_MESSAGE_ID.set, message_id)
        ctx.run(sc._SESSION_ID.set, session_id)
        return ctx.run(self.handler(tool), args or {})

    def status(self, raw):
        return json.loads(json.loads(raw)["result"])


class RouteTests(unittest.TestCase):
    def setUp(self):
        self.r = mods["lens_route"]
        self.s = self.r.Standing(LENS_KEY, LENS_SESSION, frozenset({"7", "8"}), True)

    def test_a_call_from_an_open_lens_turn_with_a_current_token_is_the_lenss(self):
        self.assertEqual(self.r.route(LENS_KEY, "7", LENS_SESSION, self.s), self.r.LENS)

    def test_everything_else_is_the_gateways(self):
        S = self.r.Standing
        cases = {
            "cron (another key)": ("cron:job-1", "7", LENS_SESSION, self.s),
            "another platform": ("agent:main:telegram:dm:42", "7", LENS_SESSION, self.s),
            "a /bg task after its turn closed (same key, message no longer open)": (LENS_KEY, "6", BG_SESSION, self.s),
            # At Hermes 38d3dbc a /bg task runs under copy_context and never rebinds the session vars, so
            # it carries the Lens's key AND the open turn's message id: only its agent's session id differs.
            "a /bg task started in the turn that is still open": (LENS_KEY, "7", BG_SESSION, self.s),
            "the Lens's session id from before the person's + (a stale conversation)": (LENS_KEY, "7", "20261102_090000_old", self.s),
            "no session id": (LENS_KEY, "7", "", self.s),
            "no message id": (LENS_KEY, "", LENS_SESSION, self.s),
            "a key that was never set (the sentinel object)": (object(), "7", LENS_SESSION, self.s),
            "a message id never set": (LENS_KEY, object(), LENS_SESSION, self.s),
            "a session id never set": (LENS_KEY, "7", object(), self.s),
            "nothing read at all": (None, None, None, self.s),
            "no standing (no adapter connected)": (LENS_KEY, "7", LENS_SESSION, None),
            "token not current": (LENS_KEY, "7", LENS_SESSION, S(LENS_KEY, LENS_SESSION, frozenset({"7"}), False)),
            "the adapter has not seen a Lens turn yet": (LENS_KEY, "7", LENS_SESSION, S("", LENS_SESSION, frozenset({"7"}), True)),
            "Hermes's store could not say which session is the Lens's": (LENS_KEY, "7", LENS_SESSION, S(LENS_KEY, "", frozenset({"7"}), True)),
            "a near-miss key": (LENS_KEY + "x", "7", LENS_SESSION, self.s),
            "an empty key": ("", "7", LENS_SESSION, self.s),
        }
        for label, (key, message_id, session_id, standing) in cases.items():
            with self.subTest(label):
                self.assertEqual(self.r.route(key, message_id, session_id, standing), self.r.SHARED)

    def test_a_value_that_merely_compares_equal_is_not_the_lenss(self):
        class Equal:
            def __eq__(self, other):
                return True

            def __hash__(self):
                return hash("7")

        for key, message_id, session_id in ((Equal(), "7", LENS_SESSION), (LENS_KEY, Equal(), LENS_SESSION),
                                            (LENS_KEY, "7", Equal())):
            with self.subTest(key=type(key).__name__):
                self.assertEqual(self.r.route(key, message_id, session_id, self.s), self.r.SHARED)

    def test_a_standing_that_cannot_be_computed_is_the_gateways(self):
        def boom():
            raise RuntimeError

        self.assertEqual(self.r.route_here(boom), self.r.SHARED)


class RoutingThroughHandlersTests(Case):
    def test_lens_calls_reach_the_lens_bridge_and_the_rest_reach_the_shared_one(self):
        self.lens.set_token("tok-lens-1")
        # The Lens bridge answers with its own pid and says it has a token.
        raw = self.run_as(LENS_KEY, "7", "taint_status")
        lens_state = json.loads(json.loads(raw)["result"])
        self.assertTrue(lens_state["has_token"])
        for label, key, mid in (("cron", "cron:job", "7"), ("a /bg task", LENS_KEY, "99"), ("unset", None, None)):
            shared_state = self.status(self.run_as(key, mid, "taint_status"))
            self.assertFalse(shared_state["has_token"], label)
            self.assertNotEqual(shared_state["pid"], lens_state["pid"], label)

    def test_two_turns_at_once_each_go_where_their_own_context_says(self):
        self.lens.set_token("tok-lens-1")
        out = {}

        def turn(label, key, mid):
            out[label] = self.status(self.run_as(key, mid, "taint_status"))

        threads = [threading.Thread(target=turn, args=a) for a in (
            ("lens", LENS_KEY, "7"), ("cron", "cron:job", "7"), ("bg", LENS_KEY, "5"), ("lens2", LENS_KEY, "7"))]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertTrue(out["lens"]["has_token"] and out["lens2"]["has_token"])
        self.assertEqual(out["lens"]["pid"], out["lens2"]["pid"])
        self.assertFalse(out["cron"]["has_token"] or out["bg"]["has_token"])
        self.assertEqual(out["cron"]["pid"], out["bg"]["pid"], "both are the gateway's one bridge")

    def test_a_bg_task_inside_an_open_lens_turn_is_the_gateways_even_with_the_lenss_key_and_message(self):
        # The case the first two checks alone get wrong (Hermes 38d3dbc, run_turn.py, run.py): /bg inherits
        # the turn's ContextVars through copy_context, so key and message id are the Lens's while the turn
        # is open. Its agent is built with session_id=task_id.
        self.lens.set_token("tok-lens-1")
        bg = self.status(self.run_as(LENS_KEY, "7", "taint_status", session_id=BG_SESSION))
        lens = self.status(self.run_as(LENS_KEY, "7", "taint_status"))
        self.assertFalse(bg["has_token"])
        self.assertTrue(lens["has_token"])
        self.assertNotEqual(bg["pid"], lens["pid"])

    def test_a_bg_task_that_read_private_state_cannot_taint_or_use_the_lens_conversation(self):
        self.lens.set_token("tok-lens-1")
        self.run_as(LENS_KEY, "7", "os_describe", session_id=BG_SESSION)
        self.assertFalse(self.status(self.run_as(LENS_KEY, "7", "taint_status"))["tainted"],
                         "its read went to the gateway's bridge, not the Lens conversation's")

    def test_before_any_token_everything_is_the_gateways(self):
        self.assertFalse(self.status(self.run_as(LENS_KEY, "7", "taint_status"))["has_token"])


class TaintScopeTests(Case):
    def test_a_plus_clears_the_lens_bridge_only_and_the_gateways_taint_stays(self):
        self.lens.set_token("tok-conversation-1")
        self.run_as(LENS_KEY, "7", "os_describe")
        self.run_as("cron:job", "7", "os_describe")
        self.assertTrue(self.status(self.run_as(LENS_KEY, "7", "taint_status"))["tainted"])
        self.assertTrue(self.status(self.run_as("cron:job", "7", "taint_status"))["tainted"])
        # The person presses + in the Lens: the desktop mints a new conversation token, which the
        # next poll carries.
        old_pid = self.lens.pid
        self.lens.set_token("tok-conversation-2")
        after = self.status(self.run_as(LENS_KEY, "7", "taint_status"))
        self.assertFalse(after["tainted"], "a new conversation starts clean")
        self.assertNotEqual(after["pid"], old_pid)
        self.assertTrue(self.status(self.run_as("cron:job", "7", "taint_status"))["tainted"],
                        "the gateway-wide bridge keeps its taint: only a restart clears it")

    def test_no_tool_the_bridge_offers_is_a_way_to_change_the_token(self):
        # The token is set by the adapter from the desktop's poll; the proxies only call tools.
        self.assertFalse(any("token" in n.lower() and n != "token_echo"
                             for n in ("os_apps", "os_describe", "os_act", "web_go")))
        self.assertFalse(hasattr(mods["lens_tools"], "set_token"))


class TokenTests(Case):
    def test_the_child_gets_the_token_in_its_environment_and_the_shared_one_never(self):
        self.lens.set_token("tok-secret-123")
        self.assertTrue(self.status(self.run_as(LENS_KEY, "7", "taint_status"))["has_token"])
        self.assertFalse(self.status(self.run_as("cron:job", "7", "taint_status"))["has_token"])

    def test_a_token_in_the_gateways_own_environment_reaches_neither_bridge(self):
        os.environ["YANTRIK_AGENT_TOKEN"] = "ambient-token-from-the-gateway"
        self.addCleanup(os.environ.pop, "YANTRIK_AGENT_TOKEN", None)
        self.assertFalse(self.status(self.run_as("cron:job", "7", "taint_status"))["has_token"])
        self.lens.set_token("tok-lens-own")
        raw = self.run_as(LENS_KEY, "7", "token_echo")
        self.assertIn("[token]", raw)
        self.assertNotIn("ambient-token-from-the-gateway", raw)

    def test_the_token_is_not_on_the_childs_command_line(self):
        self.lens.set_token("tok-secret-123")
        self.run_as(LENS_KEY, "7", "taint_status")
        cmdline = Path("/proc/%d/cmdline" % self.lens.pid)
        if cmdline.exists():
            self.assertNotIn(b"tok-secret-123", cmdline.read_bytes())

    def test_a_tool_result_that_echoes_the_token_is_scrubbed(self):
        self.lens.set_token("tok-secret-123")
        raw = self.run_as(LENS_KEY, "7", "token_echo")
        self.assertNotIn("tok-secret-123", raw)
        self.assertIn("[token]", raw)

    def test_the_token_is_never_logged_or_in_an_error(self):
        self.lens.set_token("tok-secret-123")
        with self.assertLogs(level=logging.DEBUG) as logs:
            logging.getLogger("probe").debug("probe")
            self.run_as(LENS_KEY, "7", "taint_status")
            self.lens.set_token("tok-secret-456")
            self.lens.stop()
        self.assertNotIn("tok-secret", "\n".join(logs.output))
        self.lens.set_token("tok-secret-789")
        self.lens._command = ["/no/such/yos-mcp-tok-secret-789"]
        raw = self.run_as(LENS_KEY, "7", "taint_status")
        self.assertNotIn("tok-secret-789", raw)

    def test_the_shared_bridge_is_handed_the_arguments_and_nothing_of_the_token(self):
        seen = []
        self.lens.set_token("tok-secret-123")
        handler = mods["lens_tools"].make_handler(
            "os_apps", self.lens, self.standing, lambda tool, args: seen.append((tool, args)) or "{}")
        ctx = contextvars.copy_context()
        ctx.run(mods["sc"]._SESSION_KEY.set, "cron:job")
        ctx.run(mods["sc"]._SESSION_MESSAGE_ID.set, "7")
        ctx.run(mods["sc"]._SESSION_ID.set, LENS_SESSION)
        ctx.run(handler, {"q": 1})
        self.assertEqual(seen, [("os_apps", {"q": 1})])
        self.assertNotIn("tok-secret", json.dumps(seen))


class FailClosedTests(Case):
    def test_a_token_rotated_mid_call_fails_that_call_and_runs_nothing_elsewhere(self):
        self.lens.set_token("tok-1")
        seen = []
        handler = mods["lens_tools"].make_handler(
            "slow", self.lens, self.standing, lambda tool, args: seen.append(tool) or "{}")
        out = []

        def call():
            ctx = contextvars.copy_context()
            ctx.run(mods["sc"]._SESSION_KEY.set, LENS_KEY)
            ctx.run(mods["sc"]._SESSION_MESSAGE_ID.set, "7")
            ctx.run(mods["sc"]._SESSION_ID.set, LENS_SESSION)
            out.append(ctx.run(handler, {}))

        t = threading.Thread(target=call)
        t.start()
        time.sleep(0.6)
        self.lens.set_token("tok-2")  # the person's +
        t.join(8)
        self.assertIn("error", json.loads(out[0]))
        self.assertIn("Nothing was repeated elsewhere", out[0])
        self.assertEqual(seen, [], "never re-run on the shared bridge")

    def test_a_plugin_restarted_mid_turn_has_no_token_until_the_desktop_gives_one(self):
        restarted = mods["lens_bridge"].LensBridge(self.command)
        self.addCleanup(restarted.stop)
        standing = mods["lens_route"].Standing(LENS_KEY, LENS_SESSION, frozenset({"7"}), restarted.token_current)
        self.assertEqual(mods["lens_route"].route(LENS_KEY, "7", LENS_SESSION, standing), mods["lens_route"].SHARED)

    def test_a_dead_bridge_is_an_error_to_the_mind_not_a_call_made_elsewhere(self):
        self.lens.set_token("tok-1")
        self.run_as(LENS_KEY, "7", "taint_status")
        os.kill(self.lens.pid, signal.SIGKILL)
        time.sleep(0.3)
        seen = []
        handler = mods["lens_tools"].make_handler(
            "taint_status", self.lens, self.standing, lambda tool, args: seen.append(tool) or "{}")
        ctx = contextvars.copy_context()
        ctx.run(mods["sc"]._SESSION_KEY.set, LENS_KEY)
        ctx.run(mods["sc"]._SESSION_MESSAGE_ID.set, "7")
        ctx.run(mods["sc"]._SESSION_ID.set, LENS_SESSION)
        raw = ctx.run(handler, {})
        self.assertEqual(seen, [])
        self.assertTrue("error" in json.loads(raw) or "result" in json.loads(raw))

    def test_losing_the_token_stops_using_the_lens_bridge(self):
        self.lens.set_token("tok-1")
        self.lens.set_token(None)
        self.assertFalse(self.status(self.run_as(LENS_KEY, "7", "taint_status"))["has_token"])


def alive(pid):
    """Running: not gone, and not a zombie waiting to be reaped."""
    try:
        os.kill(pid, 0)
        state = Path("/proc/%d/stat" % pid).read_text().rsplit(") ", 1)[-1][:1]
    except (ProcessLookupError, FileNotFoundError):
        return False
    return state != "Z"


class CleanupTests(Case):
    def test_stop_leaves_no_process(self):
        self.lens.set_token("tok-1")
        self.run_as(LENS_KEY, "7", "taint_status")
        pid = self.lens.pid
        self.lens.stop()
        time.sleep(0.2)
        self.assertFalse(self._reaped(pid))

    def _reaped(self, pid):
        """True while the process is still running (the name is the old one: it asks 'left over?')."""
        try:
            os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            pass
        return alive(pid)

    def test_a_rotated_token_leaves_no_process_under_the_old_one(self):
        self.lens.set_token("tok-1")
        self.run_as(LENS_KEY, "7", "taint_status")
        old = self.lens.pid
        self.lens.set_token("tok-2")
        time.sleep(0.2)
        self.assertFalse(self._reaped(old))
        self.assertIsNone(self.lens.pid)

    @unittest.skipUnless(sys.platform.startswith("linux"), "PR_SET_PDEATHSIG is Linux's")
    def test_a_gateway_that_is_killed_takes_its_bridge_with_it_even_one_that_ignores_stdin_closing(self):
        stubborn = Path(self.dir.name) / "stubborn.py"
        stubborn.write_text(FAKE_YOS_MCP + chr(10) + "while True:" + chr(10) + "    time.sleep(1)" + chr(10), encoding="utf-8")
        self.command = [sys.executable, str(stubborn)]
        probe = textwrap.dedent(
            f"""
            import importlib.util, sys, types, time
            spec = importlib.util.spec_from_file_location("lb", {str(PLUGIN / 'lens_bridge.py')!r})
            lb = importlib.util.module_from_spec(spec); sys.modules["lb"] = lb; spec.loader.exec_module(lb)
            bridge = lb.LensBridge({self.command!r})
            bridge.set_token("tok-1")
            bridge.call("taint_status", {{}})
            print(bridge.pid, flush=True)
            time.sleep(60)
            """
        )
        parent = subprocess.Popen([sys.executable, "-c", probe], stdout=subprocess.PIPE, text=True)
        try:
            child = int(parent.stdout.readline())
            self.assertTrue(alive(child))
            parent.kill()
            parent.wait()
            for _ in range(50):
                if not alive(child):
                    break
                time.sleep(0.1)
            self.assertFalse(self._reaped(child), "the bridge outlived the gateway that started it")
        finally:
            parent.kill()


class SharedPathTests(Case):
    """A call that is not the Lens's reaches the gateway's bridge through Hermes's registry, under the
    name Hermes gave it (at 38d3dbc: mcp__<server>__<tool>, tools/mcp_tool_schema.py)."""

    def fake_registry(self, with_namer):
        seen = []
        tools = types.ModuleType("tools")
        reg = types.ModuleType("tools.registry")
        reg.registry = types.SimpleNamespace(dispatch=lambda name, args: seen.append((name, args)) or '{"result": "x"}')
        saved = {k: sys.modules.get(k) for k in ("tools", "tools.registry", "tools.mcp_tool_schema")}
        sys.modules.update({"tools": tools, "tools.registry": reg})
        if with_namer:
            namer = types.ModuleType("tools.mcp_tool_schema")
            namer.mcp_prefixed_tool_name = lambda server, tool: "NAMED_%s_%s" % (server, tool)
            sys.modules["tools.mcp_tool_schema"] = namer
        else:
            sys.modules.pop("tools.mcp_tool_schema", None)

        def restore():
            for k, v in saved.items():
                if v is None:
                    sys.modules.pop(k, None)
                else:
                    sys.modules[k] = v

        self.addCleanup(restore)
        return seen

    def test_the_shared_name_is_the_one_hermes_gives_when_it_can_say(self):
        self.fake_registry(True)
        self.assertEqual(mods["lens_tools"].shared_name("os_apps"), "NAMED_yantrik_os_os_apps")

    def test_without_hermess_namer_it_is_the_double_underscore_form_at_the_pin(self):
        self.fake_registry(False)
        self.assertEqual(mods["lens_tools"].shared_name("os_apps"), "mcp__yantrik_os__os_apps")

    def test_a_cron_call_is_dispatched_to_the_registered_mcp_tool(self):
        seen = self.fake_registry(False)
        handler = mods["lens_tools"].make_handler("os_apps", self.lens, self.standing)  # the default shared path
        ctx = contextvars.copy_context()
        ctx.run(mods["sc"]._SESSION_KEY.set, "cron:job")
        ctx.run(mods["sc"]._SESSION_MESSAGE_ID.set, "7")
        ctx.run(mods["sc"]._SESSION_ID.set, "cron_session")
        ctx.run(handler, {"q": 1})
        self.assertEqual(seen, [("mcp__yantrik_os__os_apps", {"q": 1})])

    def test_with_no_registry_the_answer_is_an_error_not_a_crash(self):
        saved = sys.modules.get("tools.registry")
        sys.modules["tools.registry"] = None  # an import of it fails
        self.addCleanup(lambda: sys.modules.__setitem__("tools.registry", saved) if saved else sys.modules.pop("tools.registry", None))
        self.assertIn("error", json.loads(mods["lens_tools"]._dispatch_shared("os_apps", {})))


class InstallTests(Case):
    class Ctx:
        def __init__(self):
            self.tools = []

        def register_tool(self, **kw):
            self.tools.append(kw)

    def test_a_proxy_is_registered_for_each_tool_in_its_own_toolset(self):
        ctx = self.Ctx()
        registered = mods["lens_tools"].register_proxies(ctx, self.shared.list_tools(), self.lens, self.standing)
        self.assertEqual(registered, ["os_apps", "os_describe", "taint_status", "token_echo", "slow"])
        self.assertTrue(all(t["toolset"] == "yantrik_desk" for t in ctx.tools))
        self.assertEqual(ctx.tools[0]["schema"]["name"], "os_apps")

    def test_no_bridge_to_start_means_no_proxies_and_no_error(self):
        ctx = self.Ctx()
        saved = mods["lens_bridge"].COMMAND
        mods["lens_bridge"].COMMAND = "/no/such/yos-mcp"
        try:
            self.assertEqual(mods["lens_tools"].install(ctx, self.standing, self.lens), [])
        finally:
            mods["lens_bridge"].COMMAND = saved
        self.assertEqual(ctx.tools, [])


if __name__ == "__main__":
    unittest.main()
