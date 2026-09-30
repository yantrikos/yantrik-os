"""What the adapter's tests share: the paths, a private machine, and a surface over a fake
LibreOffice.

Every test gets a machine of its own — `HOME` and `XDG_RUNTIME_DIR` in a temporary directory,
with a ceiling and a mode the test chooses — and a `FakeLibreOffice` (tests/fake_uno.py) already
running on the adapter's pipe, reached through the adapter's real `Office` and the real surface.
Nothing but `uno` is faked.
"""

import json
import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ADAPTER = os.path.abspath(os.path.join(HERE, ".."))
REPO = os.path.abspath(os.path.join(ADAPTER, "..", ".."))
SDK = os.path.join(REPO, "sdk", "python")
BIN = os.path.join(ADAPTER, "bin")
YOS = os.path.join(REPO, "deploy", "yantrik-os", "yos")
DESKTOP = os.path.join(ADAPTER, "yantrik-libreoffice.desktop")
for path in (HERE, ADAPTER, SDK):
    if path not in sys.path:
        sys.path.insert(0, path)

import fake_uno  # noqa: E402
from yantrik_libreoffice import Office, build  # noqa: E402
from yantrik_surface import RpcError  # noqa: E402


class Machine:
    """A private HOME (the ceiling, the mode) and runtime dir (the sockets)."""

    def __init__(self, ceiling="sensitive", mode="ask"):
        self.tmp = tempfile.mkdtemp(prefix="yantrik-lo-")
        self.home = os.path.join(self.tmp, "home")
        self.runtime = os.path.join(self.tmp, "run")
        self.config = os.path.join(self.home, ".config", "yantrik")
        os.makedirs(self.config)
        os.makedirs(self.runtime)
        self.settings = os.path.join(self.config, "settings.yaml")
        self.mode_file = os.path.join(self.config, "mind-mode.json")
        self.set_ceiling(ceiling)
        self.set_mode(mode)

    def set_ceiling(self, ceiling):
        with open(self.settings, "w", encoding="utf-8") as f:
            f.write("tool_permission: %s\n" % ceiling)

    def set_mode(self, mode):
        with open(self.mode_file, "w", encoding="utf-8") as f:
            json.dump({"mode": mode, "session_rules": []}, f)

    def env(self, **extra):
        env = dict(os.environ, HOME=self.home, XDG_RUNTIME_DIR=self.runtime)
        env["PYTHONPATH"] = os.pathsep.join(p for p in (ADAPTER, SDK, env.get("PYTHONPATH")) if p)
        env.update(extra)
        return env

    def socket(self, app_id="libreoffice"):
        return os.path.join(self.runtime, "yantrik", "app-%s.sock" % app_id)

    def cleanup(self):
        shutil.rmtree(self.tmp, ignore_errors=True)


class Case(unittest.TestCase):
    """A surface over a running fake LibreOffice, on a machine of the test's own."""

    ceiling = "sensitive"
    mode = "bypass_all"

    def setUp(self):
        self.machine = Machine(self.ceiling, self.mode)
        self.addCleanup(self.machine.cleanup)
        self.files = os.path.join(self.machine.tmp, "files")
        os.makedirs(self.files)
        self.soffice = fake_uno.FakeLibreOffice().start()
        self.office = Office(uno_module=fake_uno.FakeUno(self.soffice))
        self.surface = build(self.office, settings_path=self.machine.settings,
                             mode_path=self.machine.mode_file)

    # ── documents on disk ────────────────────────────────────────────────

    def path(self, name):
        return os.path.join(self.files, name)

    def writer(self, name="report.odt", text="First paragraph.\nSecond paragraph."):
        path = self.path(name)
        fake_uno.write_doc(path, "writer", text=text)
        return path

    def calc(self, name="budget.ods", sheets=None):
        path = self.path(name)
        fake_uno.write_doc(path, "calc", sheets=sheets or [
            {"name": "Costs", "cells": {"A1": "Item", "B1": "Amount", "A2": "Rent", "B2": 1200,
                                        "A3": "Food", "B3": 350.5, "B4": "=SUM(B2:B3)"}},
            {"name": "Notes", "cells": {"A1": "checked in March"}},
        ])
        return path

    def on_disk(self, path):
        with open(path, encoding="utf-8") as f:
            return json.load(f)

    # ── calls, as a caller makes them ────────────────────────────────────

    def act(self, action, **args):
        return self.surface.act({"action": action, "args": args})

    def result(self, action, **args):
        return self.act(action, **args)["result"]

    def refused(self, action, **args):
        with self.assertRaises(RpcError) as caught:
            self.act(action, **args)
        self.assertEqual(caught.exception.code, -32602, caught.exception.message)
        return caught.exception.message

    def view(self):
        described = self.surface.describe_json()
        return described["summary"], described["state"], described["revision"]
