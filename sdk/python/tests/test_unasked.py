"""An app's own gate records what it runs unasked, with the shell (`unasked.py`, #1357 round 4)."""

import unittest

import support  # noqa: F401
from yantrik_surface import unasked
from yantrik_surface.gate import Authority, Mode

RUN = "Run a command. What it runs can do anything you can."
GONE = "Take an event off the calendar. It is not recoverable"


def at(mode, ceiling="dangerous", **kw):
    return Authority(ceiling, Mode(mode, frozenset()), **kw)


def run(authority, grade, purpose, fail=False):
    heard = []

    def write(app, action, graded, mode, args):
        heard.append((mode, args))
        return not fail

    return unasked.decide_recorded(authority, "terminal", "run", grade, purpose, {"command": "ls"}, write), heard


class TestUnasked(unittest.TestCase):
    def test_either_bypass_hands_the_shell_the_whole_call(self):
        for mode in ("bypass", "bypass_all"):
            got, heard = run(at(mode), "sensitive", RUN)
            self.assertIsNone(got, mode)
            self.assertEqual(heard, [(mode, {"command": "ls"})])
        got, heard = run(at("bypass_all"), "sensitive", GONE)
        self.assertIsNone(got)
        self.assertEqual(len(heard), 1)

    def test_no_shell_no_run(self):
        for mode in ("bypass", "bypass_all"):
            got, _ = run(at(mode), "sensitive", RUN, fail=True)
            self.assertTrue(got.startswith("NOT RUN"), mode)

    def test_nothing_else_is_recorded(self):
        got, heard = run(at("bypass_all"), "safe", RUN, fail=True)
        self.assertIsNone(got)
        self.assertEqual(heard, [])
        granted = at("bypass")
        granted.granted = True
        got, heard = run(granted, "sensitive", RUN, fail=True)
        self.assertIsNone(got)
        self.assertEqual(heard, [])
        got, heard = run(at("bypass_all", "safe"), "sensitive", RUN)
        self.assertTrue(got.startswith("CEILING:") and heard == [])
        for mode in ("ask", "auto", "plan"):
            _, heard = run(at(mode), "sensitive", RUN)
            self.assertEqual(heard, [], mode)

    def test_a_reply_has_to_say_it_was_written(self):
        ok = {"result": {"result": {"written": True}}}
        self.assertTrue(unasked._written(ok))
        for bad in ({"result": {"result": {"written": False}}}, {"result": {}}, {"error": {}}, None):
            self.assertFalse(unasked._written(bad))


if __name__ == "__main__":
    unittest.main()
