#!/usr/bin/env python3
"""A drag, a held key and a right click (#1152), against the live bench of test_live.py: the moves a
Mind needs to test the window manager, the game and the menu it wrote.

    python3 tests/browser-core/test_input.py

One browser and one site for the run, from test_live; skips without a browser, as it does.
"""

import unittest

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)


class Input(unittest.TestCase):
    """What the surface lacked (FABLE.md 9 Nov, "got in the way" 2): a drag, a held key, a right
    click — the moves a Mind needs to test the window manager, the game and the menu it wrote."""

    @classmethod
    def setUpClass(cls):
        cls.d, cls.site = Bench.d, Bench.site

    def setUp(self):
        out = self.d.go(self.site + "/pages/input.html")
        self.assertTrue(out.get("navigated"), out)

    def ref(self, name):
        for e in self.d.read(all=True)["elements"]:
            if e.get("name") == name:
                return e["ref"]
        self.fail("no %r in %s" % (name, [e.get("name") for e in self.d.read(all=True)["elements"]]))

    def test_a_drag_moves_a_box_by_what_was_asked(self):
        box = self.ref("Drag me")
        out = self.d.drag(box, 150, 60)
        self.assertEqual(out["moved"], [150, 60], out)
        self.assertIn("dragged button", out["did"])
        self.assertIn("at 250,160 (dropped)", self.d.text()["text"])

    def test_a_drag_with_no_distance_is_a_click_and_is_refused(self):
        with self.assertRaises(drv.Refused) as caught:
            self.d.drag(self.ref("Drag me"), 0, 0)
        self.assertIn("click", str(caught.exception))

    def test_a_drop_onto_a_commitment_is_refused_before_anything_moves(self):
        box = self.ref("Drag me")
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.drag(box, 420, 10)        # onto "Delete"
        self.assertIn("Delete", str(caught.exception))
        self.assertIn("Nothing was moved", str(caught.exception))
        self.assertIn("at 100,100", self.d.text()["text"])

    def test_a_held_key_is_seen_down_for_frames_and_then_up(self):
        out = self.d.key_down("ArrowUp", seconds=0.6)
        self.assertIn("held ArrowUp for 0.6 s", out["did"])
        self.assertEqual(out["held"], [])
        text = self.d.text()["text"]
        self.assertIn("ArrowUp up", text)
        frames = int(text.split("ArrowUp held for ")[1].split(" frames")[0])
        self.assertGreater(frames, 10, text)

    def test_key_down_stays_down_until_key_up(self):
        out = self.d.key_down("ArrowRight")
        self.assertEqual(out["held"], ["arrowright"])
        self.assertIn("ArrowRight down", self.d.text()["text"])
        out = self.d.key_up()
        self.assertEqual(out["held"], [])
        self.assertIn("ArrowRight up", self.d.text()["text"])
        with self.assertRaises(drv.Refused):
            self.d.key_up()

    def test_holding_space_on_a_focused_commitment_is_refused(self):
        send = self.ref("Send")
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.key_down("space", ref=send, seconds=0.2)
        self.assertIn("Send", str(caught.exception))
        self.assertNotIn("SENT", self.d.text()["text"])

    def test_a_right_click_shows_the_pages_menu_and_not_the_browsers(self):
        out = self.d.context_click(self.ref("Right-click zone"))
        self.assertIn("menu of its own", out["menu"], out)
        names = [e["name"] for e in self.d.read()["elements"] if e["role"] == "menuitem"]
        self.assertEqual(names, ["Rename", "Duplicate"])
        out = self.d.context_click(self.ref("Plain"))
        self.assertIn("no menu of its own", out["menu"], out)
        self.assertIn("plain saw contextmenu", self.d.text()["text"])


if __name__ == "__main__":
    unittest.main()
