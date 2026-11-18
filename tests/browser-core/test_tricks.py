#!/usr/bin/env python3
"""What the security review of #477 tried, against the live bench of test_live.py: every way to press
a commitment without the person's card, or to confuse the reader. None of them may work.

    python3 tests/browser-core/test_tricks.py

One browser and one site for the run, from test_live; skips without a browser, as it does.
"""

import time
import unittest

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)


class Tricks(unittest.TestCase):
    """What the security review of #477 tried, each one: a way to press a commitment without the
    person's card, or to confuse the reader. None of them may work."""

    @classmethod
    def setUpClass(cls):
        cls.d, cls.site = Bench.d, Bench.site

    def setUp(self):
        self.d.go(self.site + "/pages/tricky.html")

    def ref(self, name, role=None):
        for e in self.d.read(all=True)["elements"]:
            if e.get("name") == name and (role is None or e["role"] == role):
                return e["ref"]
        self.fail("no %r in %s" % (name, [e.get("name") for e in self.d.read(all=True)["elements"]]))

    def title(self):
        """What the page's log says a press did: "untouched" when nothing was pressed. (The page's
        own title cannot be set: its <img name="title"> shadows document.title, on purpose.)"""
        text = self.d.text()["text"]
        return "Tricky Page" if "untouched" in text else text

    def refused(self, fn, *args, **kwargs):
        with self.assertRaises(drv.PageRefused) as caught:
            fn(*args, **kwargs)
        return str(caught.exception)

    def test_a_bot_check_sees_a_person(self):
        # The page checks itself once on load and again when its button is pressed — by a
        # driver that has attached, read and clicked.
        self.d.go(self.site + "/pages/detect.html")
        self.d.click(self.ref("Check"))
        time.sleep(0.5)
        self.assertEqual(self.d.read()["title"], "HUMAN", self.d.text()["text"])

    def test_the_page_cannot_fake_the_reader_or_its_title(self):
        page = self.d.read()
        self.assertEqual(page["title"], "Tricky Page")
        self.assertIn("Tricky Page", self.d.status()["tabs"][0]["title"])

    def test_a_link_that_deletes_is_a_commitment(self):
        self.assertIn("commit", self.refused(self.d.click, self.ref("Delete")))
        self.assertEqual(self.title(), "Tricky Page")

    def test_a_link_to_a_form_is_not_but_a_link_that_orders_is(self):
        by = {e["name"]: e for e in self.d.read(all=True)["elements"]}
        self.assertNotIn("commitment", by["submit"])
        self.assertIn("commitment", by["Place order"])

    def test_a_press_that_lands_on_a_commitment_names_it(self):
        card = next(e["ref"] for e in self.d.read(all=True)["elements"] if e["role"] == "clickable")
        message = self.refused(self.d.click, card)
        self.assertIn("Buy now", message)
        self.assertEqual(self.title(), "Tricky Page")

    def test_enter_in_a_composer_with_a_send_button_is_refused_after_typing(self):
        message = self.refused(self.d.type, self.ref("Message"), "hi", enter=True)
        self.assertIn("Send", message)
        self.assertEqual(self.title(), "Tricky Page")

    def test_enter_reaches_a_button_tied_to_the_form_from_outside(self):
        message = self.refused(self.d.type, self.ref("Amount"), "5", enter=True)
        self.assertIn("Pay now", message)
        self.assertEqual(self.title(), "Tricky Page")

    def test_space_on_a_focused_commitment_is_refused(self):
        buy = self.ref("Buy now")
        self.assertIn("Space", self.refused(self.d.press, "space", buy))
        self.assertEqual(self.title(), "Tricky Page")

    def test_a_list_that_acts_on_change_does_not_get_a_commitment_chosen(self):
        self.refused(self.d.select, self.ref("Bulk action"), "Delete selected")
        self.assertEqual(self.title(), "Tricky Page")
        self.d.select(self.ref("Bulk action"), "Archive selected")

    def test_a_prompt_that_reads_as_a_commitment_goes_to_commit(self):
        self.d.click(self.ref("Wipe data"))
        self.refused(self.d.dialog, True, "DELETE")
        self.d.dialog(False)
        self.assertEqual(self.title(), "Tricky Page")

    def test_commit_on_a_dialog_needs_its_whole_message(self):
        self.d.click(self.ref("Wipe data"))
        for label in ("", "Type"):
            self.refused(self.d.commit, "dialog", label, "127.0.0.1")
        self.d.dialog(False)

    def test_words_are_words(self):
        by = {e["name"]: e for e in self.d.read(all=True)["elements"]}
        self.assertEqual(by["Post review"].get("commitment"), "post", "view inside review is not a word")
        zw = next(e for e in self.d.read(all=True)["elements"] if e.get("ref") and "uy" in e.get("name", "")
                  and e["name"] != "Buy now")
        self.assertIn("commitment", zw, "a zero-width space does not hide Buy")
        self.assertIn("Submit", by, "an input type=submit with no value is named as the browser names it")


if __name__ == "__main__":
    unittest.main()
