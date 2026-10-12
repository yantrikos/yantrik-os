"""Every refusal the dispatch makes, in the Rust runtime's words and codes, in its order.

Each expected sentence is built from the Rust format string itself (`support.quoted` checks the
string is in the Rust source; `support.render` fills it the way `format!` would), so what is
asserted is the runtime's sentence, not a copy of it. The refusals the Rust dispatch on `main`
does not make yet — an argument of the wrong type, `args` that are not an object — are the
`yantrik-surface` crate's (piece B), quoted from it once it is in this tree.
"""

import threading
import unittest
from typing import Literal, Optional

import support
from yantrik_surface import Param, Refusal, Surface, wire
from yantrik_surface.surface import NotAnswered

C = support.RUST_CONTROL
G = support.RUST_GATE
B = support.RUST_SURFACE_ARGS


def notes():
    s = Surface("notes", summary="Notes — 1 note")

    @s.view
    def state():
        return {"open": "Kernel asks"}

    @s.action("open_note")
    def open_note(title: str, pinned: bool = False) -> dict:
        """Open a note by title."""
        if title == "Ghost":
            raise Refusal("there is no note called `Ghost`")
        return {"opened": title}

    @s.action("tidy")
    def tidy() -> dict:
        """Tidy the list."""
        return {}

    @s.action("delete_note", grade="sensitive")
    def delete_note(title: str) -> dict:
        """Delete a note. It is not recoverable."""
        return {"deleted": title}

    @s.action("rename_note", grade="sensitive")
    def rename_note(title: str) -> dict:
        """Rename a note; its old title stays in its history."""
        return {"renamed": title}

    @s.action("wipe_disk", grade="dangerous")
    def wipe_disk(disk: str) -> dict:
        """Erase a disk."""
        return {}

    return s


class TestTheOrder(support.MachineCase):
    def setUp(self):
        super().setUp()
        self.s = notes()

    def act(self, action, **args):
        return self.s.act({"action": action, "args": args})

    def test_an_empty_action_is_refused_first(self):
        fragment = '"act needs a non-empty `action`"'
        support.quoted(self, C, fragment)
        for params in ({}, {"action": ""}, {"action": "  "}, {"action": 5}, None, [1]):
            self.assertEqual(self.refusal(lambda p=params: self.s.act(p)), fragment.strip('"'))

    def test_args_left_out_or_null_are_none_given(self):
        self.assertTrue(self.s.act({"action": "tidy", "args": None})["accepted"])
        self.assertTrue(self.s.act({"action": "tidy"})["accepted"])

    def test_an_unknown_action_names_the_whole_vocabulary(self):
        fragment = 'format!("unknown action `{name}`; this app offers: {}", known.join(", "))'
        support.quoted(self, C, fragment)
        expected = support.render("unknown action `{name}`; this app offers: {}",
                                  "open_note, tidy, delete_note, rename_note, wipe_disk",
                                  name="make_coffee")
        self.assertEqual(self.refusal(lambda: self.act("make_coffee")), expected)

    def test_a_missing_argument_is_named(self):
        fragment = 'format!("`{name}` needs argument `{}`", p.name)'
        support.quoted(self, C, fragment)
        self.assertEqual(self.refusal(lambda: self.act("open_note")),
                         support.render("`{name}` needs argument `{}`", "title", name="open_note"))

    def test_an_unexpected_argument_lists_the_ones_it_takes(self):
        fragment = '"`{name}` has no argument `{key}`; it takes: {}"'
        support.quoted(self, C, fragment)
        self.assertEqual(
            self.refusal(lambda: self.act("open_note", title="x", colour="red")),
            support.render(fragment.strip('"'), "title, pinned", name="open_note", key="colour"))

    def test_an_unexpected_argument_to_an_action_with_none(self):
        fragment = '"`{name}` takes no arguments, but `{key}` was given"'
        support.quoted(self, C, fragment)
        self.assertEqual(self.refusal(lambda: self.act("tidy", please=True)),
                         support.render(fragment.strip('"'), name="tidy", key="please"))

    def test_unexpected_arguments_are_reported_in_sorted_order(self):
        # serde_json's map is a BTreeMap: the first unexpected key in sorted order is named.
        message = self.refusal(lambda: self.act("tidy", zebra=1, apple=2))
        self.assertIn("`apple` was given", message)

    def test_missing_comes_before_unexpected_and_both_before_the_type(self):
        self.assertIn("needs argument", self.refusal(lambda: self.act("open_note", colour=1)))
        self.assertIn("has no argument",
                      self.refusal(lambda: self.act("open_note", title=5, colour=1)))

    def test_stale_says_where_the_app_is_now(self):
        fragment = ('"STALE: this app is at revision {} and you acted on {expected}. It now '
                    'reports: {}. Read it again before deciding."')
        support.quoted(self, C, fragment)
        current = self.s.describe_json()["revision"]
        message = self.refusal(lambda: self.s.act({
            "action": "tidy", "expect_revision": "deadbeefdeadbeef"}))
        self.assertEqual(message, support.render(fragment.strip('"'), current, "Notes — 1 note",
                                                 expected="deadbeefdeadbeef"))

    def test_a_current_revision_is_accepted_and_none_means_no_guard(self):
        current = self.s.describe_json()["revision"]
        self.assertTrue(self.s.act({"action": "tidy", "expect_revision": current})["accepted"])
        self.assertTrue(self.s.act({"action": "tidy", "expect_revision": None})["accepted"])

    def test_stale_comes_after_the_arguments(self):
        message = self.refusal(lambda: self.s.act({
            "action": "open_note", "args": {}, "expect_revision": "deadbeefdeadbeef"}))
        self.assertIn("needs argument", message)

    def test_a_handler_refusal_is_the_apps_own_sentence(self):
        self.assertEqual(self.refusal(lambda: self.act("open_note", title="Ghost")),
                         "there is no note called `Ghost`")
        support.quoted(self, C, "ServiceError { code: REFUSED, message }")
        support.quoted(self, C, "pub const REFUSED: i32 = -32602;")

    def test_an_unknown_method_names_the_two_it_serves(self):
        fragment = 'format!("unknown method `{other}`; this app serves app.describe, app.act")'
        support.quoted(self, C, fragment)
        message = self.refusal(lambda: self.s.handle("app.vibe_check", {}),
                               code=wire.RPC_METHOD_NOT_FOUND)
        self.assertEqual(message, support.render(
            "unknown method `{other}`; this app serves app.describe, app.act",
            other="app.vibe_check"))

    def test_a_handler_that_crashes_is_not_a_refusal(self):
        s = Surface("crashy")

        @s.action("boom")
        def boom() -> dict:
            """Fail."""
            raise KeyError("x")

        with self.assertRaises(KeyError):
            s.act({"action": "boom"})


class TestTheGateInTheDispatch(support.MachineCase):
    """The ceiling and the mode, as the dispatch meets them: after the arguments, which are
    answered first so that a malformed call never uses up a person's grant."""

    ceiling = None  # the default: sensitive
    mode = None     # the default: ask

    def setUp(self):
        super().setUp()
        self.s = notes()

    def test_the_arguments_come_before_the_ceiling_and_the_mode(self):
        self.assertEqual(self.refusal(lambda: self.s.act({"action": "wipe_disk"})),
                         "`wipe_disk` needs argument `disk`")
        self.assertEqual(self.refusal(lambda: self.s.act({"action": "rename_note",
                                                          "args": {"title": True}})),
                         "`rename_note` argument `title` must be a string, and a boolean arrived")

    def test_the_ceiling_refuses_a_well_formed_call(self):
        fragment = ('"CEILING: {app_id}.{action} is graded `{graded}`, above this machine\'s '
                    '`{ceiling}` ceiling (`tool_permission` in ~/.config/yantrik/settings.yaml), '
                    'so it was not run. An action at that grade needs a person to authorise it '
                    'directly — raise the ceiling in Settings if that is the intent."')
        support.quoted(self, G, fragment)
        message = self.refusal(lambda: self.s.act({"action": "wipe_disk",
                                                   "args": {"disk": "sda"}}))
        self.assertEqual(message, support.render(
            fragment.strip('"'), app_id="notes", action="wipe_disk", graded="dangerous",
            ceiling="sensitive"))

    def test_the_mode_refuses_a_well_formed_call(self):
        fragment = ('"GRANT: {app}.{action} is graded `{graded}` and this machine is in {mode} '
                    'mode, which runs nothing above `{allowed}` without asking — so it was not '
                    'run. {HOW}"')
        support.quoted(self, G, fragment)
        support.quoted(self, G, 'const HOW: &str = "%s";' % support.GATE_HOW)
        message = self.refusal(lambda: self.s.act({"action": "rename_note",
                                                   "args": {"title": "x"}}))
        self.assertEqual(message, support.render(
            fragment.strip('"'), app="notes", action="rename_note", graded="sensitive",
            mode="ask", allowed="standard", HOW=support.GATE_HOW))

    def test_what_its_own_description_says_cannot_be_undone_is_asked_about(self):
        fragment = ('"GRANT: {app}.{action} is graded `{graded}` and {final_word}, and this machine '
                    'is in {mode} mode, which asks before anything that cannot be undone — so it '
                    'was not run. {HOW}"')
        support.quoted(self, G, fragment)
        support.quoted(self, G, 'let final_word = "%s";' % support.GATE_FINAL_WORD)
        message = self.refusal(lambda: self.s.act({"action": "delete_note",
                                                   "args": {"title": "x"}}))
        self.assertEqual(message, support.render(
            fragment.strip('"'), app="notes", action="delete_note", graded="sensitive",
            final_word=support.GATE_FINAL_WORD, mode="ask", HOW=support.GATE_HOW))

    def test_standard_runs_unasked_in_ask(self):
        self.assertTrue(self.s.act({"action": "tidy"})["accepted"])

    def test_an_off_ladder_grade_is_a_ceiling_bug_said_as_one(self):
        fragment = ('"CEILING: {}.{} is graded `{}`, which is not a level this OS defines ({}), '
                    'so it was not run."')
        support.quoted(self, G, fragment)
        self.s.actions[1].permission = "lethal"
        self.assertEqual(self.refusal(lambda: self.s.act({"action": "tidy"})), support.render(
            fragment.strip('"'), "notes", "tidy", "lethal", "safe < standard < sensitive < dangerous"))

    def test_describe_needs_nothing(self):
        grades = {a["name"]: a["permission"] for a in self.s.describe_json()["actions"]}
        self.assertEqual(grades["wipe_disk"], "dangerous")


class TestWrongType(support.MachineCase):
    """An argument of the wrong type, refused before the handler sees it.

    The Rust dispatch on `main` checks presence, not type; the `yantrik-surface` crate (piece B of
    the SDK design) adds the check, and these are its rules and its sentences (`args.rs`): the
    refusal names the kind of value that arrived, never the value; `integer` is a number written
    without a fraction; an enum is a string from a list; `null` for an optional argument is the
    same as leaving it out. Each sentence is quoted from the crate when it is in this tree.
    """

    def setUp(self):
        super().setUp()
        s = Surface("types")
        self.got = {}

        @s.action("set")
        def set_(label: str = "", whole: int = 0, real: float = 0.0, on: bool = False,
                 colour: Literal["red", "green"] = "red", tags: list[str] = (),
                 blob: dict = None, need: Optional[str] = None) -> dict:
            """Set things."""
            self.got.update(label=label, whole=whole, colour=colour, blob=blob, need=need)
            return {"whole": whole, "real": real}

        @s.action("must")
        def must(n: int) -> dict:
            """Need a number."""
            return {"n": n}

        self.s = s

    def says(self, action="set", **args):
        return self.refusal(lambda: self.s.act({"action": action, "args": args}))

    def test_each_type_is_named_by_the_kind_that_arrived(self):
        support.quoted(self, B, '"`{action}` argument `{}` must be {}, and {} arrived"',
                       skip=False)
        self.assertEqual(self.says(label=True),
                         "`set` argument `label` must be a string, and a boolean arrived")
        self.assertEqual(self.says(label=2.5),
                         "`set` argument `label` must be a string, and a number arrived")
        self.assertEqual(self.says(whole="3x"),
                         "`set` argument `whole` must be an integer, and a string arrived")
        self.assertEqual(self.says(whole="3.0"),
                         "`set` argument `whole` must be an integer, and a string arrived")
        self.assertEqual(self.says(whole=2.5),
                         "`set` argument `whole` must be an integer, and a number with a fraction "
                         "arrived")
        self.assertEqual(self.says(whole=True),
                         "`set` argument `whole` must be an integer, and a boolean arrived")
        self.assertEqual(self.says(real="0.5.1"),
                         "`set` argument `real` must be a number, and a string arrived")
        self.assertEqual(self.says(on="yes"),
                         "`set` argument `on` must be a boolean, and a string arrived")
        self.assertEqual(self.says(tags="a,b"),
                         "`set` argument `tags` must be an array of strings, and a string arrived")
        self.assertEqual(self.says(blob=[1]),
                         "`set` argument `blob` must be an object, and an array arrived")
        self.assertEqual(self.says("must", n=None),
                         "`must` argument `n` must be an integer, and null arrived")

    def test_the_value_itself_is_never_repeated(self):
        message = self.says(whole="271828x")
        self.assertNotIn("271828", message)

    def test_what_converts_without_loss_reaches_the_handler_as_its_declared_type(self):
        support.quoted(self, B, "pub fn coerced(p: &Param, value: &Value) -> Option<Value>",
                       skip=False)
        out = self.s.act({"action": "set", "args": {"label": 5, "whole": "3", "real": "0.5",
                                                    "on": "true", "colour": "green"}})
        self.assertEqual(out["result"], {"whole": 3, "real": 0.5})
        self.assertEqual(self.got["label"], "5")
        self.assertEqual(self.says(colour="Green"),
                         "`set` argument `colour` must be one of `red`, `green`, and another "
                         "string arrived")
        self.assertIn("an array of strings", self.says(tags=[1]), "nothing inside an array")

    def test_an_enum_names_its_values(self):
        support.quoted(self, B, '"`{action}` argument `{}` must be one of {}, and another string '
                                'arrived"', skip=False)
        self.assertEqual(self.says(colour="blue"),
                         "`set` argument `colour` must be one of `red`, `green`, and another "
                         "string arrived")

    def test_an_array_names_the_item_that_is_wrong(self):
        support.quoted(self, B, '"`{action}` argument `{}` must be {}, and `{}[{i}]` is {}"',
                       skip=False)
        self.assertEqual(self.says(tags=["a", "b", 2]),
                         "`set` argument `tags` must be an array of strings, and `tags[2]` is a "
                         "number")

    def test_args_that_are_not_an_object(self):
        support.quoted(self, B, '"`{name}` takes its arguments as an object of named values, and '
                                '{} arrived"', skip=False)
        self.assertEqual(self.refusal(lambda: self.s.act({"action": "set", "args": [1]})),
                         "`set` takes its arguments as an object of named values, and an array "
                         "arrived")
        # After the unknown action, before the gate, as the crate checks it.
        self.assertTrue(self.refusal(lambda: self.s.act({"action": "nope", "args": "x"}))
                        .startswith("unknown action `nope`"))

    def test_what_json_allows_is_allowed(self):
        # An integer is a number; 3.0 is a number and not an integer, as serde_json reads it.
        out = self.s.act({"action": "set", "args": {"real": 2, "whole": 3}})
        self.assertEqual(out["result"], {"whole": 3, "real": 2})
        self.assertTrue(self.s.act({"action": "set", "args": {
            "label": "x", "on": True, "colour": "green", "tags": [], "blob": {}}})["accepted"])
        self.assertIn("a number with a fraction", self.says(whole=3.0))
        self.assertIn("a number with a fraction", self.says(whole=2 ** 64),
                      "past u64, serde_json holds it as a float")

    def test_null_for_an_optional_argument_is_leaving_it_out(self):
        self.s.act({"action": "set", "args": {"whole": None, "colour": None, "need": None}})
        self.assertEqual(self.got["whole"], 0, "the declared default, as with_defaults fills it")
        self.assertEqual(self.got["colour"], "red")
        self.assertIsNone(self.got["need"])

    def test_the_type_is_checked_after_the_names(self):
        self.assertIn("has no argument", self.says(label=5, nope=1))

    def test_a_default_of_the_wrong_type_is_refused_at_declaration(self):
        with self.assertRaises(ValueError) as caught:
            Param("n", "integer", default="three")
        self.assertIn("the default is wrong: argument `n` must be an integer, and a string arrived",
                      str(caught.exception))


class TestNotAnswering(support.MachineCase):
    def test_an_app_thread_that_does_not_turn_up_is_a_transport_error(self):
        fragment = 'format!("app did not answer within {}s", UI_ROUNDTRIP.as_secs())'
        support.quoted(self, C, fragment)

        class Stalled(Surface):
            def run_on_app_thread(self, fn, timeout):
                raise NotAnswered()

        s = Stalled("stalled")

        @s.action("slow", timeout=1800)
        def slow() -> dict:
            """Take a long time."""
            return {}

        @s.action("quick")
        def quick() -> dict:
            """Be quick."""
            return {}

        for call, seconds in ((lambda: s.describe_json(), 10),
                              (lambda: s.act({"action": "quick"}), 30),
                              (lambda: s.act({"action": "slow"}), 1800)):
            message = self.refusal(call, code=wire.RPC_TRANSPORT_ERROR)
            self.assertEqual(message, support.render("app did not answer within {}s", seconds))

    def test_an_app_tied_up_doing_something_it_can_report_on_is_still_described(self):
        # The default `busy_answer` is the timeout above. An app whose thread is tied up with
        # work it can describe — a render, an export — overrides it, and a caller sees a busy
        # app instead of a dead one.
        class Busy(Surface):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, **kwargs)
                self.holding = None

            def run_on_app_thread(self, fn, timeout):
                if self.holding is not None:
                    raise NotAnswered()
                return fn()

            def busy_answer(self):
                if self.holding is None:
                    return None
                return "%s, exporting" % self.app_id, {"exporting": self.holding}

        s = Busy("busy", summary="Notes — 1 note")

        @s.view
        def state():
            return {"open": "Kernel asks"}

        @s.action("export")
        def export() -> dict:
            """Export the notes."""
            return {}

        s.holding = "notes.zip"
        described = s.describe_json()
        self.assertEqual(described["summary"], "busy, exporting")
        self.assertEqual(described["state"], {"exporting": "notes.zip"},
                         "the state as of the last moment it was safe to read, not the one it "
                         "cannot reach now")
        self.assertEqual(described["revision"],
                         wire.revision(described["summary"], described["state"]),
                         "the revision of the state it is actually giving back")
        self.assertEqual([a["name"] for a in described["actions"]], ["export"],
                         "still says what it offers")

        # Only the caller that asked for nothing may be answered this way. An act has to keep
        # failing: a state beside it would read as "done", and nothing ran.
        self.assertEqual(self.refusal(lambda: s.act({"action": "export"}),
                                      code=wire.RPC_TRANSPORT_ERROR),
                         support.render("app did not answer within {}s", 30))

    def test_the_default_lock_times_out_behind_a_long_act(self):
        s = Surface("busy")
        s.describe_timeout = 0.2
        inside, release = threading.Event(), threading.Event()

        @s.action("hold")
        def hold() -> dict:
            """Hold the app."""
            inside.set()
            release.wait(5)
            return {}

        worker = threading.Thread(target=lambda: s.act({"action": "hold"}))
        worker.start()
        self.assertTrue(inside.wait(5))
        try:
            message = self.refusal(s.describe_json, code=wire.RPC_TRANSPORT_ERROR)
            self.assertEqual(message, "app did not answer within 0s")
        finally:
            release.set()
            worker.join(5)
        self.assertEqual(s.describe_json()["app"], "busy")


if __name__ == "__main__":
    unittest.main()
