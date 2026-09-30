"""The dispatch layer: grades, ceiling, revision guard, and the refusal vocabulary.

This is the half that has to be indistinguishable from `yantrik-app-runtime::control`:
the same order of checks, the same sentences to the punctuation, the same error codes, the
same envelopes. The sentences are asserted in full — not `assertIn` on a fragment — because
a caller on the other end (yos, yos-mcp, a harness, a conformance probe) reads them as the
app's own words, and a paraphrase is a different promise.
"""

import atexit
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)),
                 "..", "..", "apps", "blender", "addon")))
sys.path.insert(0, os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)),
                 "..", "..", "sdk", "python")))

import fake_bpy  # noqa: E402
from yantrik_blender.bridge import BridgeTimeout, DirectBridge  # noqa: E402
from yantrik_blender.scene import Scene  # noqa: E402
from yantrik_blender.surface import ACTIONS, Surface  # noqa: E402
from yantrik_surface import wire  # noqa: E402
from yantrik_surface.gate import LADDER, GrantRefused, grant_refusal, mode_from  # noqa: E402

ALL_ACTION_NAMES = [a.name for a in ACTIONS]


def mode_file(mode):
    """A `mind-mode.json` saying `mode`, or no file at all for None — the shell has published
    nothing, which the dispatch reads as `ask`. A dict is written as given."""
    tmp = tempfile.NamedTemporaryFile("w", suffix="-mind-mode.json", delete=False)
    tmp.close()
    atexit.register(lambda p=tmp.name: os.path.exists(p) and os.unlink(p))
    if mode is None:
        os.unlink(tmp.name)
    else:
        with open(tmp.name, "w", encoding="utf-8") as f:
            json.dump(mode if isinstance(mode, dict) else {"mode": mode}, f)
    return tmp.name


def make_surface(ceiling=None, bridge=None, mode="bypass_all", spend_grant=None):
    """A Surface over a fake bpy. `ceiling` writes a settings file; None means no file at
    all, which is a machine that has never opened Settings — the default, `sensitive`.

    `mode` defaults to full bypass, which asks about nothing under the ceiling, for the tests that
    are about everything EXCEPT the mode — the runtime's tests do the same with `open()`. The
    mode has its own tests below, with the mode pinned per case."""
    fake = fake_bpy.make_bpy()
    tmp = tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False)
    if ceiling is not None:
        tmp.write("other_key: 1\ntool_permission: %s\n" % ceiling)
    tmp.close()
    surface = Surface(Scene(fake), bridge or DirectBridge(), app_id="blender",
                      settings_path=tmp.name, mode_path=mode_file(mode),
                      spend_grant=spend_grant)
    return surface, fake, tmp.name


def refusal(asserts, fn):
    """Run fn, expect an RpcError of -32602, return the message."""
    with asserts.assertRaises(wire.RpcError) as caught:
        fn()
    asserts.assertEqual(caught.exception.code, wire.RPC_INVALID_PARAMS)
    return caught.exception.message


class TestTheActionTable(unittest.TestCase):
    def test_every_action_the_brief_names_is_here_and_graded(self):
        expected = {
            "new_scene": "standard",
            "add_primitive": "standard",
            "delete_object": "standard",
            "transform": "standard",
            "set_material": "standard",
            "set_camera": "standard",
            "set_light": "standard",
            "import_model": "standard",
            "set_render": "standard",
            "render": "sensitive",
            "save": "sensitive",
            "open": "sensitive",
            "run_python": "dangerous",
            "screenshot": "standard",
        }
        self.assertEqual({a.name: a.permission for a in ACTIONS}, expected)

    def test_every_grade_is_on_the_ladder(self):
        for action in ACTIONS:
            self.assertIn(action.permission, LADDER, action.name)

    def test_every_action_says_what_it_is_for(self):
        for action in ACTIONS:
            self.assertGreater(len(action.description), 20, action.name)

    def test_delete_object_admits_what_cannot_be_undone(self):
        delete = next(a for a in ACTIONS if a.name == "delete_object")
        self.assertIn("not recoverable", delete.description)

    def test_run_python_is_dangerous_and_says_what_it_reaches(self):
        run = next(a for a in ACTIONS if a.name == "run_python")
        self.assertEqual(run.permission, "dangerous")
        self.assertIn("anything", run.description.lower())


class TestDescribe(unittest.TestCase):
    def test_the_envelope_has_the_keys_of_every_other_app(self):
        surface, _, path = make_surface()
        self.addCleanup(os.unlink, path)
        out = surface.describe_json()
        self.assertEqual(set(out), {"protocol", "app", "summary", "state", "revision", "actions"})
        self.assertEqual(out["protocol"], 1)
        self.assertEqual(out["app"], "blender")
        self.assertEqual(len(out["revision"]), 16)
        self.assertEqual([a["name"] for a in out["actions"]], ALL_ACTION_NAMES)

    def test_the_action_schema_is_the_published_shape(self):
        surface, _, path = make_surface()
        self.addCleanup(os.unlink, path)
        schema = {a["name"]: a for a in surface.describe_json()["actions"]}
        render = schema["render"]
        self.assertEqual(set(render), {"name", "description", "permission", "settles",
                                       "parameters"})
        self.assertEqual(render["permission"], "sensitive")
        self.assertEqual(render["settles"], "on return")
        self.assertEqual(render["parameters"]["required"], ["output"])
        self.assertEqual(set(render["parameters"]["properties"]["output"]),
                         {"type", "description"})
        # Every parameter of every action carries a description — an empty string if there
        # is nothing to add, but the key is there, because a caller that reads it must not
        # have to ask whether an absent key means "no description" or "old app".
        for action in surface.describe_json()["actions"]:
            for prop in action["parameters"]["properties"].values():
                self.assertIn("description", prop)
                self.assertIsInstance(prop["description"], str)

    def test_new_scene_takes_no_arguments_and_says_so(self):
        surface, _, path = make_surface()
        self.addCleanup(os.unlink, path)
        schema = {a["name"]: a for a in surface.describe_json()["actions"]}
        self.assertEqual(schema["new_scene"]["parameters"]["properties"], {})
        self.assertEqual(schema["new_scene"]["parameters"]["required"], [])


class TestDispatchOrder(unittest.TestCase):
    def setUp(self):
        self.surface, self.fake, self.settings = make_surface()
        self.addCleanup(os.unlink, self.settings)

    def act(self, action, **args):
        return self.surface.act({"action": action, "args": args})

    def test_an_empty_action_is_refused_before_anything_else(self):
        for params in ({}, {"action": ""}, {"action": "   "}, {"action": 5}):
            message = refusal(self, lambda p=params: self.surface.act(p))
            self.assertEqual(message, "act needs a non-empty `action`")

    def test_unknown_action_lists_the_whole_vocabulary(self):
        message = refusal(self, lambda: self.act("make_coffee"))
        self.assertEqual(
            message,
            "unknown action `make_coffee`; this app offers: " + ", ".join(ALL_ACTION_NAMES))

    def test_unknown_method_names_the_two_methods_it_serves(self):
        with self.assertRaises(wire.RpcError) as caught:
            self.surface.handle("app.vibe_check", {})
        self.assertEqual(caught.exception.code, wire.RPC_METHOD_NOT_FOUND)
        self.assertEqual(
            caught.exception.message,
            "unknown method `app.vibe_check`; this app serves app.describe, app.act")

    def test_missing_required_argument(self):
        self.assertEqual(refusal(self, lambda: self.act("render")),
                         "`render` needs argument `output`")
        self.assertEqual(refusal(self, lambda: self.act("add_primitive")),
                         "`add_primitive` needs argument `kind`")

    def test_unexpected_argument_lists_the_ones_it_takes(self):
        message = refusal(self, lambda: self.act("render", output="/tmp/x.png", quality=5))
        self.assertEqual(message,
                         "`render` has no argument `quality`; it takes: output")

    def test_unexpected_argument_to_an_action_with_none(self):
        message = refusal(self, lambda: self.act("new_scene", please=True))
        self.assertEqual(message,
                         "`new_scene` takes no arguments, but `please` was given")

    def test_an_argument_of_the_wrong_type_is_refused_before_the_scene_sees_it(self):
        # The SDK checks each argument against its published type, in the Rust
        # `yantrik-surface` crate's words: `samples` is published `integer`, so text that is not
        # exactly an integer is refused naming the argument (and not echoing the value), and the
        # integer 4 — which is what `yos act blender set_render samples=4` sends — is taken.
        self.assertEqual(
            refusal(self, lambda: self.act("set_render", samples="four")),
            "`set_render` argument `samples` must be an integer, and a string arrived")
        self.assertEqual(
            refusal(self, lambda: self.act("set_material", name="Cube", metallic="0.2.0")),
            "`set_material` argument `metallic` must be a number, and a string arrived")
        self.act("set_render", engine="cycles")
        self.assertEqual(self.act("set_render", samples=4)["state"]["render"]["samples"], 4)
        # Text that is exactly the number converts without loss: the scene reads the integer.
        self.assertEqual(self.act("set_render", samples="8")["state"]["render"]["samples"], 8)

    def test_the_arguments_are_checked_before_the_ceiling(self):
        # Order pinned by the runtime (docs/surface-protocol.md §5): a malformed call is refused
        # for its arguments first, so a person's grant is never spent on one; a well-formed
        # dangerous call is then refused on the grade.
        self.assertEqual(refusal(self, lambda: self.act("run_python")),
                         "`run_python` needs argument `code`")
        message = refusal(self, lambda: self.act("run_python", code="print(1)"))
        self.assertTrue(message.startswith("CEILING: blender.run_python is graded "
                                           "`dangerous`, above this machine's `sensitive` "
                                           "ceiling"), message)

    def test_a_stale_revision_is_refused_with_the_current_state_of_the_world(self):
        message = refusal(self, lambda: self.surface.act({
            "action": "add_primitive",
            "args": {"kind": "cube"},
            "expect_revision": "deadbeefdeadbeef",
        }))
        summary, state = self.surface.scene.snapshot()
        current = wire.revision(summary, state)
        self.assertEqual(
            message,
            "STALE: this app is at revision %s and you acted on deadbeefdeadbeef. "
            "It now reports: %s. Read it again before deciding." % (current, summary))

    def test_a_current_revision_is_accepted(self):
        described = self.surface.describe_json()
        out = self.surface.act({
            "action": "add_primitive",
            "args": {"kind": "cube"},
            "expect_revision": described["revision"],
        })
        self.assertTrue(out["accepted"])
        self.assertEqual(out["result"]["type"], "MESH")

    def test_a_handler_refusal_comes_out_as_the_apps_own_sentence(self):
        message = refusal(self, lambda: self.act("delete_object", name="Ghost"))
        self.assertEqual(message, "there is no object `Ghost` in this scene")

    def test_the_failure_is_said_twice_once_in_the_error_once_in_the_state(self):
        refusal(self, lambda: self.act("delete_object", name="Ghost"))
        state = self.surface.describe_json()["state"]
        self.assertEqual(state["notice"], "there is no object `Ghost` in this scene")
        # And a success clears it: a notice that outlives the fault is a lie by staleness.
        self.act("add_primitive", kind="cube")
        state = self.surface.describe_json()["state"]
        self.assertEqual(state["notice"], "")


class TestEnvelopeOnSuccess(unittest.TestCase):
    def test_act_answers_with_the_post_action_world(self):
        surface, _, path = make_surface()
        self.addCleanup(os.unlink, path)
        out = surface.act({"action": "add_primitive", "args": {"kind": "monkey"}})
        self.assertEqual(set(out), {"app", "action_id", "accepted", "settled", "result",
                                    "revision", "summary", "state"})
        self.assertEqual(out["app"], "blender")
        self.assertEqual(out["action_id"], "app-blender#1")
        self.assertTrue(out["accepted"])
        self.assertTrue(out["settled"])
        self.assertEqual(out["result"]["object"], "Suzanne.001")
        self.assertEqual(out["state"]["objects_total"], 1)
        self.assertIn("1 object", out["summary"])
        # The revision in the answer is the revision of the state in the answer.
        self.assertEqual(out["revision"], wire.revision(out["summary"], out["state"]))
        # And the next action is numbered after this one.
        second = surface.act({"action": "new_scene", "args": {}})
        self.assertEqual(second["action_id"], "app-blender#2")


class TestCeiling(unittest.TestCase):
    def surfaces_with(self, ceiling):
        surface, _, path = make_surface(ceiling=ceiling)
        self.addCleanup(os.unlink, path)
        return surface

    def test_no_settings_file_is_the_default_ceiling(self):
        surface, _, path = make_surface(ceiling=None)
        os.unlink(path)  # the file was made empty; remove it to be a machine with none
        self.assertEqual(surface.configured_ceiling(), "sensitive")

    def test_an_explicit_ceiling_is_read(self):
        self.assertEqual(self.surfaces_with("dangerous").configured_ceiling(), "dangerous")
        self.assertEqual(self.surfaces_with("standard").configured_ceiling(), "standard")

    def test_a_quoted_or_padded_value_is_still_read(self):
        self.assertEqual(self.surfaces_with('"dangerous"').configured_ceiling(), "dangerous")

    def test_a_value_off_the_ladder_falls_back_to_the_default(self):
        self.assertEqual(self.surfaces_with("godmode").configured_ceiling(), "sensitive")

    def test_dangerous_runs_when_the_ceiling_says_it_may(self):
        surface = self.surfaces_with("dangerous")
        out = surface.act({"action": "run_python",
                           "args": {"code": "print(2 + 2)"}})
        self.assertTrue(out["accepted"])
        self.assertIn("4", out["result"]["printed"])

    def test_sensitive_actions_are_refused_under_a_standard_ceiling(self):
        surface = self.surfaces_with("standard")
        message = refusal(self, lambda: surface.act(
            {"action": "render", "args": {"output": "/tmp/x.png"}}))
        self.assertEqual(
            message,
            "CEILING: blender.render is graded `sensitive`, above this machine's "
            "`standard` ceiling (`tool_permission` in ~/.config/yantrik/settings.yaml), "
            "so it was not run. An action at that grade needs a person to authorise it "
            "directly — raise the ceiling in Settings if that is the intent.")

    def test_run_python_under_the_default_ceiling_gets_the_full_sentence(self):
        surface, _, path = make_surface()
        message = refusal(self, lambda: surface.act(
            {"action": "run_python", "args": {"code": "import os"}}))
        self.assertEqual(
            message,
            "CEILING: blender.run_python is graded `dangerous`, above this machine's "
            "`sensitive` ceiling (`tool_permission` in ~/.config/yantrik/settings.yaml), "
            "so it was not run. An action at that grade needs a person to authorise it "
            "directly — raise the ceiling in Settings if that is the intent.")

    def test_a_grade_off_the_ladder_is_a_ceiling_bug_said_as_one(self):
        surface, _, path = make_surface(ceiling="dangerous")
        spec = next(a for a in surface.actions if a.name == "new_scene")
        original = spec.permission
        spec.permission = "lethal"
        try:
            message = refusal(self, lambda: surface.act(
                {"action": "new_scene", "args": {}}))
            self.assertEqual(
                message,
                "CEILING: blender.new_scene is graded `lethal`, which is not a level this "
                "OS defines (safe < standard < sensitive < dangerous), so it was not run.")
        finally:
            spec.permission = original


class TestModeAndGrant(unittest.TestCase):
    """Issue #116, found on this app: `blender.render` through the MCP bridge raised a card, and
    through `yos act` it ran in 1.72 s with nobody asked — the mode lived in the bridge. These
    pin the runtime's rule in this port of its dispatch, sentences in full."""

    ASK_SENTENCE = (
        "GRANT: blender.save is graded `sensitive` and this machine is in ask mode, which runs "
        "nothing above `standard` without asking — so it was not run. Ask the shell for approval "
        "first (`request_approval` with this app, action and these exact arguments, poll "
        "`approval_status`, then send the granted request_id as `grant` on app.act — `yos act` "
        "does all of that for you), or have the person at the machine press Allow when the card "
        "appears.")

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.blend = os.path.join(self.tmp.name, "scene.blend")
        self.spent = []

    def shell(self, grant, app, action, args):
        """A stand-in for the shell's store: `fresh-*` holds once, for exactly this save."""
        if not grant.startswith("fresh-"):
            raise GrantRefused("no approval request `%s`." % grant)
        if (app, action, args) != ("blender", "save", {"path": self.blend}):
            raise GrantRefused("`%s` was approved for other arguments. Nothing was authorised."
                               % grant)
        if grant in self.spent:
            raise GrantRefused("`%s` was already used." % grant)
        self.spent.append(grant)

    def surface(self, mode, ceiling=None):
        surface, fake, path = make_surface(ceiling=ceiling, mode=mode, spend_grant=self.shell)
        self.addCleanup(os.unlink, path)
        return surface, fake

    def save(self, surface, grant=None, **args):
        params = {"action": "save", "args": args or {"path": self.blend}}
        if grant:
            params["grant"] = grant
        return surface.act(params)

    def test_a_sensitive_act_without_a_grant_is_refused_in_ask_mode(self):
        surface, fake = self.surface("ask")
        self.assertEqual(refusal(self, lambda: self.save(surface)), self.ASK_SENTENCE)
        self.assertNotEqual(fake.data.filepath, self.blend, "the handler must not have run")

    def test_no_mode_file_is_ask_not_something_looser(self):
        surface, _ = self.surface(None)
        self.assertEqual(refusal(self, lambda: self.save(surface)), self.ASK_SENTENCE)

    def test_the_arguments_are_looked_at_before_the_mode(self):
        surface, _ = self.surface("ask")
        message = refusal(self, lambda: surface.act({"action": "save", "args": {}}))
        self.assertEqual(message, "`save` needs argument `path`")
        self.assertTrue(refusal(self, lambda: self.save(surface)).startswith("GRANT:"))

    def test_a_sensitive_act_runs_in_auto_and_bypass(self):
        for mode in ("auto", "bypass", "bypass_all"):
            surface, fake = self.surface(mode)
            self.assertTrue(self.save(surface)["accepted"], mode)
            self.assertEqual(fake.data.filepath, self.blend, mode)

    def test_a_grant_lets_a_sensitive_act_run_in_any_mode(self):
        for n, mode in enumerate(("plan", "ask", "auto", "bypass", "bypass_all")):
            surface, fake = self.surface(mode)
            self.assertTrue(self.save(surface, grant="fresh-%d" % n)["accepted"], mode)
            self.assertEqual(fake.data.filepath, self.blend, mode)

    def test_a_spent_or_wrong_grant_is_refused_before_anything_is_dispatched(self):
        surface, fake = self.surface("ask")
        self.save(surface, grant="fresh-1")
        fake.data.filepath = ""
        self.assertEqual(
            refusal(self, lambda: self.save(surface, grant="fresh-1")),
            "GRANT: `fresh-1` does not authorise blender.save — `fresh-1` was already used. "
            "Nothing was run; a grant covers one action, once, with the arguments the person "
            "was shown.")
        message = refusal(self, lambda: self.save(
            surface, grant="fresh-2", path=os.path.join(self.tmp.name, "other.blend")))
        self.assertIn("approved for other arguments", message)
        self.assertIn("no approval request", refusal(self, lambda: self.save(surface, grant="made-up")))
        self.assertEqual(fake.data.filepath, "", "nothing ran on a grant that did not hold")

    def test_the_ceiling_refuses_whatever_the_grant_or_mode(self):
        for mode, grant in (("bypass", None), ("ask", "fresh-9"), ("bypass", "fresh-10"),
                            ("bypass_all", None)):
            surface, fake = self.surface(mode, ceiling="standard")
            message = refusal(self, lambda s=surface, g=grant: self.save(s, grant=g))
            self.assertTrue(message.startswith("CEILING:"), (mode, grant, message))

    def test_a_grant_is_not_spent_on_an_act_the_ceiling_refuses(self):
        # #154: the grant was spent first, so an act above the ceiling used up the person's
        # Allow and never ran. The ceiling comes first now, as in the runtime.
        surface, _ = self.surface("ask", ceiling="standard")
        message = refusal(self, lambda: self.save(surface, grant="fresh-154"))
        self.assertTrue(message.startswith("CEILING:"), message)
        self.assertEqual(self.spent, [], "the grant was spent on an act the ceiling refused")
        # And it still holds once the ceiling allows the act.
        surface, fake = self.surface("ask")
        self.assertTrue(self.save(surface, grant="fresh-154")["accepted"])
        self.assertEqual(self.spent, ["fresh-154"])

    def test_an_unknown_action_is_answered_before_a_grant_is_looked_at(self):
        surface, _ = self.surface("ask")
        message = refusal(self, lambda: surface.act({"action": "nope", "args": {}, "grant": "made-up"}))
        self.assertTrue(message.startswith("unknown action `nope`"), message)

    def test_a_standard_act_runs_unasked_in_every_mode_plan_included(self):
        # The desktop's own processes call `standard` actions on these sockets; see the
        # runtime's SOCKET_FLOOR. Plan's refusal of `standard` is the bridge's.
        for mode in ("plan", "ask", "auto", "bypass", "bypass_all"):
            surface, _ = self.surface(mode)
            out = surface.act({"action": "add_primitive", "args": {"kind": "cube"}})
            self.assertTrue(out["accepted"], mode)
        self.assertEqual(self.spent, [], "no grant was asked for, so none was spent")

    def test_plan_refuses_a_sensitive_act_and_says_no_card_is_coming(self):
        surface, _ = self.surface("plan")
        self.assertEqual(
            refusal(self, lambda: self.save(surface)),
            "GRANT: blender.save is graded `sensitive` and this machine is in plan mode, which "
            "raises no card for anything above `standard` — so it was not run. Say what you "
            "would do and let the person decide; they switch the mode from the chip in the "
            "status bar.")

    def test_a_session_rule_covers_its_own_action_and_no_other(self):
        rule = {"mode": "ask", "session_rules": [{"app": "blender", "action": "save"}]}
        surface, fake = self.surface(rule)
        self.assertTrue(self.save(surface)["accepted"])
        other = {"mode": "ask", "session_rules": [{"app": "blender", "action": "open"}]}
        surface, _ = self.surface(other)
        self.assertTrue(refusal(self, lambda: self.save(surface)).startswith("GRANT:"))

    def test_describe_needs_nothing(self):
        surface, _ = self.surface("plan")
        actions = {a["name"]: a for a in surface.describe_json()["actions"]}
        self.assertEqual(actions["save"]["permission"], "sensitive")

    def test_the_mode_file_is_read_the_way_the_runtime_reads_it(self):
        now = 1_800_000_000
        self.assertEqual(mode_from('{"mode":"auto"}', now), ("auto", set()))
        for text in ("", "{}", "mode: auto", '{"mode":"yolo"}', '{"mode":"BYPASS"}', "[]"):
            self.assertEqual(mode_from(text, now)[0], "ask", text)
        bypass = json.dumps({"mode": "bypass", "previous": "auto",
                             "bypass_expires_unix": now + 60})
        self.assertEqual(mode_from(bypass, now)[0], "bypass")
        self.assertEqual(mode_from(bypass, now + 60)[0], "auto")
        odd = json.dumps({"mode": "bypass", "previous": "bypass", "bypass_expires_unix": now})
        self.assertEqual(mode_from(odd, now)[0], "ask")
        until_restart = json.dumps({"mode": "bypass", "bypass_expires_unix": None})
        self.assertEqual(mode_from(until_restart, now + 86_400)[0], "bypass")
        full = json.dumps({"mode": "bypass_all", "previous": "auto", "bypass_expires_unix": now + 60})
        self.assertEqual(mode_from(full, now)[0], "bypass_all")
        self.assertEqual(mode_from(full, now + 60)[0], "auto")
        self.assertEqual(grant_refusal("blender", "save", "sensitive", "ask"), self.ASK_SENTENCE)


class TestBridgeTimeout(unittest.TestCase):
    def test_a_main_thread_that_never_turns_up_is_a_transport_error(self):
        class StalledBridge(DirectBridge):
            def submit(self, fn, timeout=None):
                raise BridgeTimeout()

        surface, _, path = make_surface(bridge=StalledBridge())
        with self.assertRaises(wire.RpcError) as caught:
            surface.describe_json()
        self.assertEqual(caught.exception.code, wire.RPC_TRANSPORT_ERROR)
        self.assertEqual(caught.exception.message, "app did not answer within 10s")
        with self.assertRaises(wire.RpcError) as caught:
            surface.act({"action": "add_primitive", "args": {"kind": "cube"}})
        self.assertEqual(caught.exception.message, "app did not answer within 30s")
        with self.assertRaises(wire.RpcError) as caught:
            surface.act({"action": "render", "args": {"output": "/tmp/x.png"}})
        self.assertEqual(caught.exception.message, "app did not answer within 1800s")


class TestQueuedBridge(unittest.TestCase):
    """The real bridge, with a pump thread standing in for Blender's main thread."""

    def test_submit_crosses_threads_and_comes_back(self):
        import threading
        import time

        from yantrik_blender.bridge import QueuedBridge

        bridge = QueuedBridge()
        results = []

        def pump():
            while True:
                bridge.pump()
                time.sleep(0.005)

        worker = threading.Thread(target=pump, daemon=True)
        worker.start()
        try:
            results.append(bridge.submit(lambda: "from the main thread", timeout=5))
            results.append(bridge.submit(lambda: 2 + 2, timeout=5))
        finally:
            bridge.wake()
        self.assertEqual(results, ["from the main thread", 4])

    def test_a_job_that_raises_gives_the_submitter_the_exception(self):
        import threading
        import time

        from yantrik_blender.bridge import QueuedBridge
        from yantrik_blender.scene import Refusal

        bridge = QueuedBridge()
        outcome = {}

        def submit():
            try:
                bridge.submit(lambda: (_ for _ in ()).throw(Refusal("no")), timeout=5)
            except Refusal as e:
                outcome["raised"] = str(e)

        submitter = threading.Thread(target=submit)
        submitter.start()
        # Pump only once the job is actually queued, or the pump and the submit race and the
        # test flaps: a pump that ran too early finds nothing and the submit times out.
        for _ in range(500):
            with bridge._lock:
                queued = bool(bridge._queue)
            if queued:
                break
            time.sleep(0.01)
        self.assertTrue(queued, "the job never reached the queue")
        bridge.pump()
        submitter.join(timeout=5)
        self.assertEqual(outcome.get("raised"), "no")

    def test_wake_releases_a_waiter_with_the_truth(self):
        import threading

        from yantrik_blender.bridge import QueuedBridge

        bridge = QueuedBridge()
        outcome = {}

        def submit():
            try:
                bridge.submit(lambda: "never runs", timeout=30)
            except RuntimeError as e:
                outcome["error"] = str(e)

        submitter = threading.Thread(target=submit)
        submitter.start()
        # Wait for the job to be queued and the submitter to be inside its wait, so what is
        # tested is a waiter being released, not a submit finding the door already closed.
        import time as _time
        for _ in range(500):
            with bridge._lock:
                queued = bool(bridge._queue)
            if queued:
                break
            _time.sleep(0.01)
        self.assertTrue(queued, "the job never reached the queue")
        bridge.wake()
        submitter.join(timeout=5)
        self.assertEqual(outcome.get("error"), "the app is closing; it took no action")


if __name__ == "__main__":
    unittest.main()
