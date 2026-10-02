"""Security review round 4 on #574 (F1-F6): what the adapter does that the review found missing.

No Hermes here: the gateway modules the adapter imports are stood in for just long enough to import
it, and removed again, so the contract test beside this one still meets the real Hermes where there
is one.
"""

import asyncio
import importlib.util
import logging
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]
INSTALL = PLUGIN.parent / "lib" / "install"


def _stub_gateway():
    class Platform(str):
        pass

    class PlatformConfig:
        def __init__(self, extra=None):
            self.extra = extra or {}

    class Base:
        def __init__(self, config, platform):
            self.config = config
            self._active_sessions = set()
            self._message_handler = None

        def build_source(self, **kw):
            return types.SimpleNamespace(**kw)

        def _mark_connected(self):
            pass

        def _mark_disconnected(self):
            pass

    class MessageEvent:
        def __init__(self, **kw):
            self.__dict__.update(kw)

    class Enum:
        TEXT = "text"
        SUCCESS = "s"
        CANCELLED = "c"

    def build_session_key(source, group_sessions_per_user=True, thread_sessions_per_user=False):
        return "agent:main:yantrik:dm:%s" % source.chat_id

    mods = {
        "gateway": types.ModuleType("gateway"),
        "gateway.config": types.SimpleNamespace(Platform=Platform, PlatformConfig=PlatformConfig),
        "gateway.platforms": types.ModuleType("gateway.platforms"),
        "gateway.platforms.base": types.SimpleNamespace(
            BasePlatformAdapter=Base, MessageEvent=MessageEvent, MessageType=Enum, ProcessingOutcome=Enum, SendResult=object
        ),
        "gateway.session": types.SimpleNamespace(build_session_key=build_session_key),
    }
    return mods


def _load_package():
    mods = _stub_gateway()
    saved = {k: sys.modules.get(k) for k in mods}
    sys.modules.update(mods)
    try:
        spec = importlib.util.spec_from_file_location(
            "yantrik_r4_plugin", PLUGIN / "__init__.py", submodule_search_locations=[str(PLUGIN)]
        )
        package = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = package
        spec.loader.exec_module(package)
        from yantrik_r4_plugin import adapter, desktop, guard  # noqa: F401

        return mods["gateway.config"], adapter, desktop, guard
    finally:
        for k, v in saved.items():
            if v is None:
                sys.modules.pop(k, None)
            else:
                sys.modules[k] = v


CONFIG, adapter, desktop, guard = _load_package()
CREDENTIAL = "mem-" + "cd" * 32
URL = "unix:/run/yantrik-mind/1000/memory.sock"


class Provider:
    """The memory provider's registry, with the failures a test asks it to have."""

    def __init__(self):
        self.held = {}
        self.fail_with = None
        self.cleared = 0

    def register(self, key, credential, url=None):
        if self.fail_with is not None:
            raise self.fail_with
        if credential is None:
            self.held.pop(key, None)
        else:
            self.held[key] = (credential, url)

    def clear(self):
        self.cleared += 1
        self.held.clear()


class Env:
    """Run with the provider's registry found, and the refusal file in a scratch directory."""

    def setUp(self):
        self.provider = Provider()
        self.scratch = tempfile.TemporaryDirectory()
        self._env = dict(os.environ)
        os.environ["XDG_CONFIG_HOME"] = self.scratch.name
        found = {
            desktop.REGISTRY_FUNCTION: self.provider.register,
            desktop.CLEAR_FUNCTION: self.provider.clear,
        }
        self._real = desktop.find_registry
        desktop.find_registry = lambda import_module=None, name=desktop.REGISTRY_FUNCTION: found.get(name)
        self.adapter = adapter.YantrikAdapter(CONFIG.PlatformConfig())

    def tearDown(self):
        desktop.find_registry = self._real
        os.environ.clear()
        os.environ.update(self._env)
        self.scratch.cleanup()


class RevocationTests(Env, unittest.TestCase):
    """F2: the desktop's revoke reaches every session the adapter ever registered."""

    def test_a_revoke_calls_set_desktop_credential_none_for_every_session(self):
        for key in ("agent:main:yantrik:dm:a", "agent:main:yantrik:dm:b"):
            self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, key)
        self.assertEqual(len(self.provider.held), 2)
        calls = []
        real = self.provider.register
        desktop_register = lambda k, c, u=None: (calls.append((k, c, u)), real(k, c, u))  # noqa: E731
        found = {desktop.REGISTRY_FUNCTION: desktop_register, desktop.CLEAR_FUNCTION: self.provider.clear}
        desktop.find_registry = lambda import_module=None, name=desktop.REGISTRY_FUNCTION: found.get(name)
        self.adapter._revoke_memory()
        self.assertEqual(
            sorted(calls), [("agent:main:yantrik:dm:a", None, None), ("agent:main:yantrik:dm:b", None, None)]
        )
        self.assertEqual(self.provider.held, {})
        self.assertIsNone(self.adapter._carried_memory)

    def test_one_failing_session_does_not_leave_the_others_holding_theirs(self):
        failed = desktop.revoke_all({"a", "b"}, lambda k, c, u=None: (_ for _ in ()).throw(RuntimeError("secret " + CREDENTIAL)), None)
        self.assertEqual(failed, ["RuntimeError", "RuntimeError"], "kinds only, never a message")

    def test_the_poll_loop_acts_on_memory_revoked(self):
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "k")
        replies = iter([{desktop.MEMORY_REVOKED: True}])
        self.adapter._session = "s"

        async def call(method, params, timeout=10.0):
            if method == desktop.POLL:
                try:
                    return next(replies)
                except StopIteration:
                    raise asyncio.CancelledError
            return {}

        self.adapter._call = call
        asyncio.run(self.adapter._run())
        self.assertEqual(self.provider.held, {}, "the credential was void before any next turn")

    def test_the_protocol_field_is_the_one_the_shell_sends(self):
        self.assertEqual(desktop.MEMORY_REVOKED, "memory_revoked")


class FailClosedTests(Env, unittest.TestCase):
    """F4: no failure leaves a session holding an earlier credential."""

    def test_an_unexpected_provider_error_clears_what_the_session_held(self):
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "k")
        self.assertIn("k", self.provider.held)
        # The provider breaks on the next set, but takes the clear: the held one is gone.
        real = self.provider.register

        def flaky(key, credential, url=None):
            if credential is not None:
                raise RuntimeError("provider bug quoting " + CREDENTIAL)
            real(key, credential, url)

        found = {desktop.REGISTRY_FUNCTION: flaky, desktop.CLEAR_FUNCTION: self.provider.clear}
        desktop.find_registry = lambda import_module=None, name=desktop.REGISTRY_FUNCTION: found.get(name)
        with self.assertLogs("yantrik_r4_plugin.adapter", level="WARNING") as logs:
            self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "k")
        self.assertNotIn("k", self.provider.held)
        self.assertIsNone(self.adapter._carried_memory)
        self.assertNotIn(CREDENTIAL, "\n".join(logs.output))

    def test_when_even_the_clear_of_that_session_fails_everything_is_cleared(self):
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "k")

        def broken(key, credential, url=None):
            raise RuntimeError("down")

        found = {desktop.REGISTRY_FUNCTION: broken, desktop.CLEAR_FUNCTION: self.provider.clear}
        desktop.find_registry = lambda import_module=None, name=desktop.REGISTRY_FUNCTION: found.get(name)
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "k")
        self.assertEqual(self.provider.cleared, 1)
        self.assertEqual(self.provider.held, {})

    def test_a_refused_address_leaves_nothing_registered(self):
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "k")
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": "http://example.com/mcp"}, "k")
        self.assertEqual(self.provider.held, {})


class MemoryUrlTests(unittest.TestCase):
    """F5: the adapter's own loopback-or-unix rule, whatever the provider does."""

    def test_only_loopback_http_and_absolute_unix_paths_pass(self):
        for good in (
            "http://127.0.0.1:8765/mcp", "http://localhost/mcp", "http://[::1]:9/mcp",
            "unix:/run/yantrik-mind/1000/memory.sock",
        ):
            with self.subTest(good=good):
                self.assertTrue(desktop.valid_memory_url(good))
        for bad in (
            "", None, 5, "https://127.0.0.1/mcp", "http://example.com/mcp", "http://127.0.0.1.evil.example/",
            "http://127.0.0.1@evil.example/", "http://user:pw@127.0.0.1/", "http://127.0.0.1:notaport/",
            "unix:relative/sock", "unix:", "unix:/run/../etc/sock", "ftp://127.0.0.1/", " http://127.0.0.1/",
            "http://0.0.0.0/", "http://192.168.1.5/",
        ):
            with self.subTest(bad=bad):
                self.assertFalse(desktop.valid_memory_url(bad))

    def test_an_off_host_address_never_reaches_the_provider_and_clears_the_session(self):
        calls = []
        with self.assertRaises(ValueError) as refused:
            desktop.carry_memory(
                {"memory_credential": CREDENTIAL, "memory_url": "http://evil.example/mcp"}, "k",
                lambda *a: calls.append(a),
            )
        self.assertEqual(calls, [("k", None, None)], "cleared, and never handed the credential")
        self.assertNotIn(CREDENTIAL, str(refused.exception))
        self.assertNotIn("evil", str(refused.exception))


class SessionKeyTests(Env, unittest.TestCase):
    """F6: one deterministic key per session, and no turn shares another's."""

    def source(self, chat):
        return types.SimpleNamespace(chat_id=chat)

    def test_the_key_is_the_same_every_time_and_two_sessions_never_share_one(self):
        keys = {}
        for chat in ("desktop", "agent-2", "agent-3", "desktop2"):
            first, agreed = self.adapter._gateway_session_key(self.source(chat))
            again, _ = self.adapter._gateway_session_key(self.source(chat))
            self.assertTrue(agreed)
            self.assertEqual(first, again, "deterministic")
            keys[chat] = first
        self.assertEqual(len(set(keys.values())), len(keys), "two sessions never share a key: %s" % keys)

    def test_a_gateway_that_keys_it_differently_gets_no_credential_and_it_is_said_once(self):
        self.adapter._session_key_for_source = lambda source: "agent:main:yantrik:dm:other"
        with self.assertLogs("yantrik_r4_plugin.adapter", level="ERROR") as logs:
            key, agreed = self.adapter._gateway_session_key(self.source("desktop"))
            self.adapter._gateway_session_key(self.source("desktop"))
        self.assertFalse(agreed)
        self.assertEqual(len(logs.output), 1)
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "" if not agreed else key)
        self.assertEqual(self.provider.held, {})

    def test_a_turn_that_only_queues_behind_a_running_one_registers_nothing(self):
        self.adapter._carry_memory({"memory_credential": CREDENTIAL, "memory_url": URL}, "agent:main:yantrik:dm:desktop")
        running = self.provider.held["agent:main:yantrik:dm:desktop"]
        self.adapter._active_sessions.add("agent:main:yantrik:dm:desktop")
        ledger_turns = []

        async def while_busy(event, turn, key):
            ledger_turns.append(key)

        self.adapter._while_busy = while_busy
        asyncio.run(self.adapter._on_turn({"turn_id": 7, "text": "hi", "memory_credential": "mem-" + "ef" * 32, "memory_url": URL}))
        self.assertEqual(ledger_turns, ["agent:main:yantrik:dm:desktop"])
        self.assertEqual(self.provider.held["agent:main:yantrik:dm:desktop"], running, "the running turn's own is intact")


class FakeConfig:
    """hermes_config as the guard meets it."""

    class ConfigError(Exception):
        pass

    def __init__(self, found=(), left=(), raises=None):
        self._found, self._left, self.raises = list(found), list(left), raises
        self.applied = 0

    def reassert(self, path):
        self.applied += 1
        return self._found, self._left

    def hermes_resolver(self):
        return lambda *a: set()

    def problems(self, path, resolve):
        if self.raises:
            raise self.raises
        return self._found


class GuardTests(Env, unittest.TestCase):
    """F1: the allowlist is held at every gateway start, and a widened one is refused and said."""

    def test_a_clean_start_attaches_and_clears_an_old_refusal(self):
        desktop.say_refused("old")
        self.assertIsNone(guard.start("x", FakeConfig()))
        self.assertTrue(guard.decide(None))
        self.assertFalse(desktop.refusal_path().exists())

    def test_a_widened_allowlist_is_restored_but_this_start_does_not_attach(self):
        cfg = FakeConfig(found=["platform_toolsets.yantrik lists terminal"])
        reason = guard.start("x", cfg)
        self.assertEqual(cfg.applied, 1, "re-asserted")
        self.assertIn("terminal", reason)
        self.assertIn("restart", reason)
        self.assertFalse(guard.decide(reason))
        self.assertIn("terminal", desktop.refusal_path().read_text())

    def test_a_file_that_could_not_be_put_right_refuses(self):
        self.assertIn("could not be put right", guard.start("x", FakeConfig(found=["a"], left=["a"])))

    def test_a_missing_check_refuses_rather_than_passes(self):
        real = guard._hermes_config
        guard._hermes_config = lambda: None
        try:
            self.assertIn("missing", guard.start("x"))
            self.assertIn("missing", guard.check("x"))
        finally:
            guard._hermes_config = real

    def test_an_attach_checks_again_and_a_file_widened_since_is_refused(self):
        self.assertIsNone(guard.check("x", FakeConfig()))
        self.assertIn("widened", guard.check("x", FakeConfig(found=["mcp_servers.yantrik_os is not the desktop's own"])))
        err = FakeConfig(raises=FakeConfig.ConfigError("cannot read"))
        err.ConfigError = FakeConfig.ConfigError
        self.assertIn("cannot read", guard.check("x", err))

    def test_the_adapter_does_not_attach_while_refused_and_attaches_when_put_right(self):
        cfg = FakeConfig(found=["terminal"])
        real = guard._hermes_config
        guard._hermes_config = lambda: cfg
        try:
            self.assertFalse(asyncio.run(self.adapter._attach()))
            self.assertTrue(desktop.refusal_path().exists(), "said on the row")
            cfg._found = []
            self.adapter._refusal = None
            desktop.socket_path = lambda: None  # nothing to attach to; past the guard is all this asks
            asyncio.run(self.adapter._attach())
            self.assertFalse(desktop.refusal_path().exists())
        finally:
            guard._hermes_config = real

    def test_a_gateway_start_refuses_on_a_widened_file(self):
        real = guard.start
        guard.start = lambda: "widened"
        try:
            asyncio.run(self.adapter.connect(is_reconnect=False))
            self.assertEqual(self.adapter._refusal, "widened")
            for task in self.adapter._tasks:
                task.cancel()
        finally:
            guard.start = real


class SourceShapeTests(unittest.TestCase):
    def test_hermes_sh_copies_the_check_beside_the_plugin(self):
        script = (INSTALL / "hermes.sh").read_text(encoding="utf-8")
        self.assertIn('hermes_config.py" "$HOME/.hermes/plugins/yantrik/hermes_config.py"', script)

    def test_the_credential_is_never_in_a_log_line_of_the_new_code(self):
        for name in ("adapter.py", "guard.py"):
            for line in (PLUGIN / name).read_text(encoding="utf-8").splitlines():
                if "logger." in line:
                    self.assertNotIn("credential)", line)
                    self.assertNotIn("memory_credential", line)


if __name__ == "__main__":
    logging.disable(logging.NOTSET)
    unittest.main()
