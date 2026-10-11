"""My Surface's tests: what it publishes, what each action does, what the dispatch refuses before
a handler runs, and the program held to the protocol by `yos check`.

Every test runs on a machine of its own — `HOME` and `XDG_RUNTIME_DIR` in a temporary directory,
with the settings this OS ships (a `sensitive` ceiling) and the mode the test names — so nothing
on the developer's desktop is read or touched. Keep these when you rename the template, and add
one for each action you add.

    python3 -m unittest discover -s tests -v
"""

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PROGRAM = os.path.join(HERE, "..", "my_surface.py")
DESKTOP = os.path.join(HERE, "..", "my-surface.desktop")
# Inside the yantrik-os repository the SDK and `yos` are a few directories up; anywhere else the
# SDK is installed (or copied beside my_surface.py) and `yos` is found as below.
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
SDK = os.path.join(REPO, "sdk", "python")
if os.path.isdir(os.path.join(SDK, "yantrik_surface")) and SDK not in sys.path:
    sys.path.insert(0, SDK)

from yantrik_surface import RpcError  # noqa: E402


def find_yos():
    """`yos`: `$YOS`, the repository's, a Yantrik machine's, or PATH's — or None."""
    for path in (os.environ.get("YOS"), os.path.join(REPO, "deploy", "yantrik-os", "yos"),
                 "/opt/yantrik/bin/yos"):
        if path and os.path.isfile(path):
            return [sys.executable, path]
    found = shutil.which("yos")
    return [found] if found else None


class Machine:
    """A private HOME (the ceiling, the mode) and runtime dir (the sockets)."""

    def __init__(self, mode):
        self.tmp = tempfile.TemporaryDirectory(prefix="my-surface-")
        self.home = os.path.join(self.tmp.name, "home")
        self.runtime = os.path.join(self.tmp.name, "run")
        config = os.path.join(self.home, ".config", "yantrik")
        os.makedirs(config)
        os.makedirs(self.runtime)
        with open(os.path.join(config, "settings.yaml"), "w", encoding="utf-8") as f:
            f.write("tool_permission: sensitive\n")
        with open(os.path.join(config, "mind-mode.json"), "w", encoding="utf-8") as f:
            json.dump({"mode": mode, "session_rules": []}, f)

    def env(self):
        env = dict(os.environ, HOME=self.home, XDG_RUNTIME_DIR=self.runtime)
        env["PYTHONPATH"] = os.pathsep.join(p for p in (SDK, env.get("PYTHONPATH")) if p)
        return env


class SurfaceCase(unittest.TestCase):
    mode = "ask"

    def setUp(self):
        self.machine = Machine(self.mode)
        self.addCleanup(self.machine.tmp.cleanup)
        saved = {k: os.environ.get(k) for k in ("HOME", "XDG_RUNTIME_DIR")}
        os.environ.update(HOME=self.machine.home, XDG_RUNTIME_DIR=self.machine.runtime)
        self.addCleanup(lambda: [os.environ.pop(k, None) if v is None else os.environ.update({k: v})
                                 for k, v in saved.items()])
        # A fresh copy of the program for every test: its list starts empty.
        spec = importlib.util.spec_from_file_location("my_surface_under_test", PROGRAM)
        self.program = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.program)
        self.surface = self.program.surface

    def act(self, action, **args):
        return self.surface.act({"action": action, "args": args})

    def refused(self, action, **args):
        with self.assertRaises(RpcError) as caught:
            self.act(action, **args)
        self.assertEqual(caught.exception.code, -32602, "every refusal of an act is -32602")
        return caught.exception.message

    def mode_is(self, mode):
        path = os.path.join(self.machine.home, ".config", "yantrik", "mind-mode.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump({"mode": mode, "session_rules": []}, f)


class TestWhatItPublishes(SurfaceCase):
    def test_describe_publishes_the_list_and_every_action_graded(self):
        described = self.surface.describe_json()
        self.assertEqual(described["app"], self.program.APP_ID)
        self.assertEqual(described["protocol"], 1)
        self.assertEqual(described["summary"], "My Surface — 0 tasks, 0 done")
        self.assertEqual([(a["name"], a["permission"]) for a in described["actions"]],
                         [("add", "standard"), ("complete", "standard"), ("find", "safe"),
                          ("remove", "sensitive")])
        add = described["actions"][0]["parameters"]
        self.assertEqual(add["required"], ["title"])
        self.assertEqual(add["properties"]["priority"],
                         {"type": "string", "description": "How soon it matters",
                          "enum": ["low", "normal", "high"], "default": "normal"})

    def test_the_desktop_file_declares_this_surface(self):
        # Rename the template and forget the .desktop file, and this is the test that says so.
        with open(DESKTOP, encoding="utf-8") as f:
            keys = dict(line.strip().split("=", 1) for line in f
                        if "=" in line and not line.lstrip().startswith("#"))
        self.assertEqual(keys["X-Yantrik-Surface"], self.program.APP_ID)
        self.assertTrue(keys["X-Yantrik-Purpose"])
        self.assertEqual(keys["Exec"], "my-surface")


class TestWhatEachActionDoes(SurfaceCase):
    def test_add_complete_and_find_do_what_they_say(self):
        reply = self.act("add", title="Water the plants", priority="high")
        self.assertEqual((reply["accepted"], reply["settled"]), (True, True))
        self.assertEqual(reply["summary"], "My Surface — 1 task, 0 done",
                         "the answer carries the view after the act")
        self.act("add", title="Call the plumber")
        self.assertEqual(self.program.tasks[1]["priority"], "normal", "the declared default")
        self.act("complete", index=0)
        self.mode_is("plan")
        found = self.act("find", query="PLANTS")
        self.assertEqual(found["result"]["matches"],
                         [{"index": 0, "title": "Water the plants", "done": True}])
        self.mode_is("ask")
        # `done=false` takes it back: that is why `complete` is `standard`, not more.
        self.act("complete", index=0, done=False)
        self.assertFalse(self.program.tasks[0]["done"])

    def test_a_call_its_own_arguments_refuse_never_reaches_the_handler(self):
        self.assertEqual(self.refused("add"), "`add` needs argument `title`")
        self.assertEqual(self.refused("add", title="x", priority="urgent"),
                         "`add` argument `priority` must be one of `low`, `normal`, `high`, "
                         "and another string arrived")
        self.assertEqual(self.refused("complete", index="first"),
                         "`complete` argument `index` must be an integer, and a string arrived")
        self.assertEqual(self.refused("add", title="x", due="friday"),
                         "`add` has no argument `due`; it takes: title, priority")
        self.assertEqual(self.program.tasks, [])
        # What converts without loss is converted: "0" for an integer is 0.
        self.act("add", title="x")
        self.act("complete", index="0")
        self.assertTrue(self.program.tasks[0]["done"])
        # The handler's own refusal, in its own words.
        self.assertEqual(self.refused("complete", index=5),
                         "there is no task 5; the list has 1, indexed 0 to 0")

    def test_remove_waits_for_the_person_in_ask_and_in_auto(self):
        self.act("add", title="Water the plants")
        self.assertTrue(self.refused("remove", index=0).startswith(
            "GRANT: my-surface.remove is graded `sensitive`"))
        # Auto runs `sensitive` unasked — but not what its own description says cannot be undone.
        self.mode_is("auto")
        self.assertIn("its own description says it cannot be undone", self.refused("remove", index=0))
        self.assertEqual(len(self.program.tasks), 1, "nothing was removed")
        # Bypass still asks before what cannot be undone; full bypass runs everything under
        # the ceiling.
        self.mode_is("bypass")
        self.assertIn("its own description says it cannot be undone", self.refused("remove", index=0))
        self.mode_is("bypass_all")
        self.assertEqual(self.act("remove", index=0)["result"],
                         {"removed": "Water the plants", "left": 0})

    def test_an_act_decided_on_a_view_that_moved_is_refused_as_stale(self):
        read = self.surface.describe_json()["revision"]
        self.act("add", title="Something else happened first")
        with self.assertRaises(RpcError) as caught:
            self.surface.act({"action": "add", "args": {"title": "x"}, "expect_revision": read})
        self.assertTrue(caught.exception.message.startswith("STALE: this app is at revision "))


class TestTheProgram(SurfaceCase):
    def test_yos_check_finds_nothing_wrong(self):
        yos = find_yos()
        if yos is None:
            self.skipTest("no yos on this machine (set YOS=/path/to/yos)")
        program = subprocess.Popen([sys.executable, PROGRAM], env=self.machine.env(),
                                   stderr=subprocess.PIPE, text=True)
        socket = os.path.join(self.machine.runtime, "yantrik", "app-my-surface.sock")
        try:
            deadline = time.monotonic() + 15
            while not os.path.exists(socket):
                self.assertIsNone(program.poll(), "my_surface.py exited before it served")
                self.assertLess(time.monotonic(), deadline, "my_surface.py never bound its socket")
                time.sleep(0.05)
            done = subprocess.run(yos + ["check", "my-surface", "--json"], env=self.machine.env(),
                                  capture_output=True, text=True, timeout=60)
            report = json.loads(done.stdout)
            failed = [r for r in report["surfaces"][0]["checks"] if r["status"] == "fail"]
            self.assertEqual((done.returncode, failed), (0, []))
        finally:
            program.terminate()
            _, err = program.communicate(timeout=10)
        self.assertEqual(program.returncode, 0, "SIGTERM is a clean stop: %s" % err)
        self.assertFalse(os.path.exists(socket), "the socket outlived the program")


if __name__ == "__main__":
    unittest.main()
