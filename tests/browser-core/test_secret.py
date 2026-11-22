"""#1357 round 7: `secret=true` is typed only into a field the page itself marks a password field.
A call that says it is secret on any other field is refused before anything is typed."""

import unittest

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)


class Secret(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.d, cls.site = Bench.d, Bench.site

    def open_shop(self):
        out = self.d.go(self.site + "/pages/shop.html")
        self.assertTrue(out.get("navigated"), out)

    def ref(self, name):
        for e in self.d.read(all=True)["elements"]:
            if e.get("name") == name:
                return e["ref"]
        self.fail("no %r on the page" % name)

    def test_secret_is_refused_on_a_field_that_is_not_a_password_and_nothing_is_typed(self):
        self.open_shop()
        ref = self.ref("Nickname")
        with self.assertRaises(drv.PageRefused) as cm:
            self.d.type(ref, "visible-text", secret=True)
        self.assertIn("not a password field", str(cm.exception))
        field = self.d.read(all=True)
        self.assertNotIn("visible-text", str(field))

    def test_secret_on_a_password_field_types_and_reports_only_a_count(self):
        self.open_shop()
        out = self.d.type(self.ref("Password"), "hunter2-correct", secret=True)
        self.assertIn("(15 characters)", out["did"], out)
        self.assertNotIn("hunter2", str(out))

    def tricks(self, name):
        out = self.d.go(self.site + "/pages/secret_tricks.html")
        self.assertTrue(out.get("navigated"), out)
        return self.ref(name)

    def sink(self):
        return self.d.read(all=True)["elements"]

    def assertNothingInSink(self, text):
        self.assertNotIn(text, str(self.sink()))

    def test_focus_that_moves_after_the_check_is_refused_and_nothing_is_typed_elsewhere(self):
        for name in ("Moves on focus", "Moves on key"):
            for secret in (True, False):
                ref = self.tricks(name)
                with self.assertRaises(drv.PageRefused) as cm:
                    self.d.type(ref, "plain-visible-text", secret=secret)
                self.assertIn("focus", str(cm.exception), (name, secret))
                self.assertNothingInSink("plain-visible-text")

    def test_focus_that_moves_while_typing_is_said_in_the_result(self):
        out = self.d.type(self.tricks("Moves on input"), "abc", secret=True, enter=True)
        self.assertIn("focus_moved", out)

    def test_a_container_marked_password_is_not_a_password_field(self):
        ref = self.tricks("Looks like a password")
        with self.assertRaises(drv.PageRefused) as cm:
            self.d.type(ref, "plain-visible-text", secret=True)
        self.assertIn("not a password field", str(cm.exception))
        self.assertNothingInSink("plain-visible-text")

    def test_a_field_whose_type_changes_is_refused_and_nothing_is_typed(self):
        ref = self.tricks("Changes type")
        with self.assertRaises(drv.PageRefused):
            self.d.type(ref, "plain-visible-text", secret=True)
        self.assertNothingInSink("plain-visible-text")


if __name__ == "__main__":
    unittest.main()
