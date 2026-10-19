"""The Hermes adapter against a fake gateway: a /new from the desktop is not left waiting on a
confirmation nobody can see.

Hermes 0.21 asks before /new, /reset and /undo, through the platform's `send_slash_confirm`. The
desktop's new-conversation button sends /new and throws the reply away, so the question went where
nobody saw it, the confirm stayed pending, and every turn after it hung (VM 541, 13 Oct 2026).

Hermes itself is not needed: the gateway, its platform base and `tools.slash_confirm` are faked
here with the shape Hermes 0.21 has, and the desktop is the adapter's own `_call`, recorded.
"""

import asyncio
import importlib.util
import sys
import types
import unittest
from enum import Enum
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

PLUGIN = Path(__file__).resolve().parents[1] / "hermes"


# ── Hermes, faked ────────────────────────────────────────────────────────────────────────────────


class SendResult:
    def __init__(self, success, message_id=None, error=None):
        self.success, self.message_id, self.error = success, message_id, error


class ProcessingOutcome(Enum):
    SUCCESS = "success"
    FAILURE = "failure"
    CANCELLED = "cancelled"


class MessageEvent:
    def __init__(self, text, message_type=None, source=None, message_id=None, timestamp=None):
        self.text, self.source, self.message_id = text, source, message_id

    def get_command(self):
        word = self.text.strip().split()[0] if self.text.strip() else ""
        return word[1:].lower() if word.startswith("/") else None


class BasePlatformAdapter:
    """As much of Hermes's platform base as the adapter leans on. `handle_message` is the
    gateway: it runs the message to the end, processing hooks included."""

    gateway = None

    def __init__(self, config, platform):
        self.config, self.platform = config, platform
        self._active_sessions = {}

    def _mark_connected(self):
        pass

    def _mark_disconnected(self):
        pass

    def build_source(self, **fields):
        return SimpleNamespace(**fields)

    async def handle_message(self, event):
        await self.gateway.handle(self, event)

    async def send_slash_confirm(self, chat_id, title, message, session_key, confirm_id, metadata=None):
        return SendResult(success=False, error="Not supported")


def slash_confirm_module():
    """`tools.slash_confirm` as Hermes 0.21 has it: one pending confirm per session."""
    module = types.ModuleType("tools.slash_confirm")
    pending = {}

    def register(session_key, confirm_id, command, handler):
        pending[session_key] = {"confirm_id": confirm_id, "command": command, "handler": handler}

    def get_pending(session_key):
        entry = pending.get(session_key)
        return dict(entry) if entry else None

    async def resolve(session_key, confirm_id, choice):
        entry = pending.get(session_key)
        if not entry or entry["confirm_id"] != confirm_id:
            return None
        pending.pop(session_key)
        module.resolved.append((entry["command"], choice))
        result = await entry["handler"](choice)
        return result if isinstance(result, str) else None

    module.register, module.get_pending, module.resolve = register, get_pending, resolve
    module.resolved = []
    return module


class Gateway:
    """Hermes's gateway, as far as these turns go.

    /new and /undo are gated the way `_maybe_confirm_destructive_slash` gates them: registered
    first, then offered to the adapter, and asked in text when the adapter cannot. While a confirm
    is pending, a plain message is held, which is how every turn after the unseen /new hung. A
    Hermes without `tools.slash_confirm` runs the command straight away.
    """

    COMMANDS = {"new": "✨ New session started.", "undo": "↩️ Undid the last exchange."}

    def __init__(self, confirms):
        self.confirms = confirms
        self.asked_model = []
        self.held = []
        self.ran = []
        self._ids = 0

    async def handle(self, adapter, event):
        reply = await self._reply(adapter, event)
        if reply:
            await adapter.send(event.source.chat_id, reply, reply_to=event.message_id)
        await adapter.on_processing_complete(event, ProcessingOutcome.SUCCESS)

    async def _reply(self, adapter, event):
        command = event.get_command()
        if command in ("new", "reset"):
            command = "new"
        if command in self.COMMANDS:
            return await self._confirm(adapter, event, command)
        if self.confirms is not None and self.confirms.get_pending(SESSION):
            self.held.append(event.text)
            return None
        self.asked_model.append(event.text)
        return f"An answer to {event.text}"

    async def _confirm(self, adapter, event, command):
        async def run(choice):
            if choice == "cancel":
                return f"/{command} cancelled."
            self.ran.append(command)
            return self.COMMANDS[command]

        if self.confirms is None:
            return await run("once")
        self._ids += 1
        confirm_id = str(self._ids)
        self.confirms.register(SESSION, confirm_id, command, run)
        result = await adapter.send_slash_confirm(
            chat_id=event.source.chat_id, title=f"/{command}", message="…", session_key=SESSION,
            confirm_id=confirm_id, metadata=None,
        )
        if result and result.success:
            return None
        return f"⚠️ Confirm /{command} — reply /approve, /always or /cancel."


SESSION = "agent:main:yantrik:dm:desktop"


def _hermes_modules():
    gateway = types.ModuleType("gateway")
    config = types.ModuleType("gateway.config")
    config.Platform = lambda name: name
    config.PlatformConfig = SimpleNamespace
    platforms = types.ModuleType("gateway.platforms")
    base = types.ModuleType("gateway.platforms.base")
    base.BasePlatformAdapter = BasePlatformAdapter
    base.MessageEvent = MessageEvent
    base.MessageType = SimpleNamespace(TEXT="text")
    base.ProcessingOutcome = ProcessingOutcome
    base.SendResult = SendResult
    session = types.ModuleType("gateway.session")
    session.build_session_key = lambda source, **_: SESSION
    hermes_cli = types.ModuleType("hermes_cli")
    commands = types.ModuleType("hermes_cli.commands")
    commands.should_bypass_active_session = lambda command: command in {"stop", "new", "reset", "approve", "deny"}
    return {
        "gateway": gateway,
        "gateway.config": config,
        "gateway.platforms": platforms,
        "gateway.platforms.base": base,
        "gateway.session": session,
        "hermes_cli": hermes_cli,
        "hermes_cli.commands": commands,
    }


def _load_adapter():
    """The plugin as Hermes loads it: a package, so `from . import desktop` resolves."""
    with mock.patch.dict(sys.modules, _hermes_modules()):
        spec = importlib.util.spec_from_file_location(
            "yantrik_hermes_plugin", PLUGIN / "__init__.py", submodule_search_locations=[str(PLUGIN)]
        )
        package = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = package
        spec.loader.exec_module(package)
        from yantrik_hermes_plugin import adapter

    return adapter


ADAPTER = _load_adapter()


class Desktop:
    """What the adapter told the desktop, per turn: what it streamed, and how it was closed."""

    def __init__(self):
        self.said = {}
        self.closed = {}

    async def call(self, method, params, timeout=10.0):
        turn = str(params.get("turn_id"))
        if method == "harness.chunk":
            self.said[turn] = self.said.get(turn, "") + params["delta"]
        elif method in ("harness.complete", "harness.fail"):
            self.closed[turn] = self.closed.get(turn, []) + [method]
        return {}


def _tools(confirms):
    """`tools` with `slash_confirm` in it, or (None) a Hermes from before it existed; and the
    `hermes_cli` the adapter imports while a turn is under way."""
    package = types.ModuleType("tools")
    modules = {name: m for name, m in _hermes_modules().items() if name.startswith("hermes_cli")}
    modules["tools"] = package
    if confirms is not None:
        package.slash_confirm = confirms
        modules["tools.slash_confirm"] = confirms
    return mock.patch.dict(sys.modules, modules)


class Harness:
    def __init__(self, confirms):
        self.desktop = Desktop()
        self.gateway = Gateway(confirms)
        self.adapter = ADAPTER.YantrikAdapter(SimpleNamespace(extra={}))
        self.adapter.gateway = self.gateway
        self.adapter._session = "session"
        self.adapter._address = "socket"
        self.adapter._call = self.desktop.call

    async def turn(self, turn_id, text):
        await self.adapter._on_turn({"turn_id": turn_id, "text": text})


def run(confirms, script):
    async def go():
        harness = Harness(confirms)
        await script(harness)
        return harness

    with _tools(confirms):
        return asyncio.run(go())


class NewConversationTests(unittest.TestCase):
    def test_a_new_from_the_desktop_is_confirmed_once_and_ends_with_hermes_reply(self):
        confirms = slash_confirm_module()
        harness = run(confirms, lambda h: h.turn(41, "/new"))
        self.assertEqual(confirms.resolved, [("new", "once")])
        self.assertEqual(harness.gateway.ran, ["new"])
        self.assertEqual(harness.desktop.said["41"], "✨ New session started.")
        self.assertNotIn("Confirm", harness.desktop.said["41"], "no question goes where nobody reads it")
        self.assertEqual(harness.desktop.closed["41"], ["harness.complete"], "closed exactly once")
        self.assertIsNone(confirms.get_pending(SESSION))

    def test_a_typed_reset_is_the_same_ask_and_is_confirmed_too(self):
        confirms = slash_confirm_module()
        harness = run(confirms, lambda h: h.turn(7, "/reset"))
        self.assertEqual(confirms.resolved, [("new", "once")])
        self.assertEqual(harness.desktop.closed["7"], ["harness.complete"])

    def test_the_next_turn_after_new_is_handed_to_hermes_not_left_waiting(self):
        async def script(h):
            await h.turn(41, "/new")
            await h.turn(42, "what is on my calendar?")

        harness = run(slash_confirm_module(), script)
        self.assertEqual(harness.gateway.held, [])
        self.assertEqual(harness.gateway.asked_model, ["what is on my calendar?"])
        self.assertEqual(harness.desktop.said["42"], "An answer to what is on my calendar?")
        self.assertEqual(harness.desktop.closed["42"], ["harness.complete"])

    def test_undo_is_not_confirmed_for_the_person_and_hermes_asks_them(self):
        confirms = slash_confirm_module()
        harness = run(confirms, lambda h: h.turn(9, "/undo"))
        self.assertEqual(confirms.resolved, [])
        self.assertEqual(harness.gateway.ran, [])
        self.assertIn("Confirm /undo", harness.desktop.said["9"])
        self.assertEqual(confirms.get_pending(SESSION)["command"], "undo", "still the person's to answer")

    def test_a_hermes_without_slash_confirm_runs_new_as_before(self):
        harness = run(None, lambda h: h.turn(5, "/new"))
        self.assertEqual(harness.gateway.ran, ["new"])
        self.assertEqual(harness.desktop.said["5"], "✨ New session started.")
        self.assertEqual(harness.desktop.closed["5"], ["harness.complete"])

    def test_without_slash_confirm_the_adapter_declines_and_hermes_falls_back_to_text(self):
        async def script(h):
            h.result = await h.adapter.send_slash_confirm("desktop", "/new", "…", SESSION, "1")

        harness = run(None, script)
        self.assertFalse(harness.result.success)

    def test_a_new_while_hermes_works_is_closed_once_and_ends_the_work_it_stopped(self):
        async def script(h):
            # Hermes is mid-task on turn 40: it started, and its session is busy.
            h.adapter._ledger.open("40", "desktop", "compare flights")
            h.adapter._ledger.mark_started("40")
            h.adapter._ledger.get("40").session_key = SESSION
            h.adapter._active_sessions[SESSION] = object()
            await h.turn(41, "/new")

        harness = run(slash_confirm_module(), script)
        self.assertEqual(harness.desktop.said["41"], "✨ New session started.")
        self.assertEqual(harness.desktop.closed["41"], ["harness.complete"], "not closed a second time")
        self.assertEqual(harness.desktop.closed["40"], ["harness.fail"])


if __name__ == "__main__":
    unittest.main()
