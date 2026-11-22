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


if __name__ == "__main__":
    unittest.main()
