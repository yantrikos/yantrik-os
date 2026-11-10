#!/usr/bin/env python3
"""A drag, a held key and a right click (#1152), against the live bench of test_live.py: the moves a
Mind needs to test the window manager, the game and the menu it wrote.

    python3 tests/browser-core/test_input.py

One browser and one site for the run, from test_live; skips without a browser, as it does.
"""

import time
import unittest
from unittest import mock

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)
from yantrik_browser import hands, holds, inputs  # noqa: E402


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

    # ── the security review of #1170: attacks from its scenarios ──

    def test_a_drag_cannot_press_into_another_sites_frame_that_click_refuses(self):
        self.d.go(self.site + "/pages/framepay.html")
        time.sleep(0.5)
        frame = self.d.read(all=True)["blind_frames"][0]["ref"]
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.click(frame)
        self.assertIn("another site", str(caught.exception))
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.drag(frame, 2, 0)        # mousedown and mouseup on the same element make a click
        self.assertIn("another site", str(caught.exception))
        self.assertNotIn("PAID", self.d.text()["text"])

    def test_a_drag_cannot_be_released_inside_another_sites_frame(self):
        self.d.go(self.site + "/pages/framepay.html")
        time.sleep(0.5)
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.drag(self.ref("Drag me"), 320, 30)        # lets go over the payment frame
        self.assertIn("frame from another site", str(caught.exception))
        self.assertIn("Nothing was moved", str(caught.exception))
        out = self.d.drag(self.ref("Drag me"), 0, 150)        # the same box, to open ground, still moves
        self.assertEqual(out["moved"], [0, 150], out)

    def test_a_drag_cannot_press_a_commitment_that_acts_on_mousedown(self):
        send = self.ref("Send report")
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.drag(send, 0, 100)
        self.assertIn("commit ref=", str(caught.exception))
        self.assertNotIn("SENT-ON-MOUSEDOWN", self.d.text()["text"])

    def test_a_right_click_cannot_press_a_commitment_that_acts_on_mousedown(self):
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.context_click(self.ref("Send report"))
        self.assertIn("commit ref=", str(caught.exception))
        self.assertNotIn("SENT-ON-MOUSEDOWN", self.d.text()["text"])

    def test_a_key_held_with_nobody_to_release_it_is_released_after_the_limit(self):
        with mock.patch.object(holds, "HOLD_MAX", 2.0):
            out = self.d.key_down("ArrowRight")
            self.assertEqual(out["held"], ["arrowright"])
            time.sleep(3.0)
        self.assertIn("ArrowRight up", self.d.text()["text"])
        self.assertEqual(self.d.browser.tab().held_keys, set())

    def test_a_timed_hold_that_fails_midway_still_releases_the_key(self):
        with mock.patch.object(inputs.time, "sleep", side_effect=RuntimeError("the page went away")):
            with self.assertRaises(RuntimeError):
                self.d.key_down("ArrowRight", seconds=1)
        self.assertIn("ArrowRight up", self.d.text()["text"])
        self.assertEqual(self.d.browser.tab().held_keys, set())

    def test_a_held_key_gets_its_keyup_before_a_reload_and_before_the_tab_is_left(self):
        self.d.key_down("ArrowRight")
        sent = []
        with mock.patch.object(hands, "key_up", wraps=hands.key_up) as up:
            self.d.reload()
            sent.extend(c.args[2] for c in up.call_args_list)
        self.assertIn("arrowright", sent, "the keyUp was sent, not only forgotten")
        self.d.key_down("ArrowLeft")
        here = next(t["tab"] for t in self.d.tabs()["tabs"] if t["active"])
        self.d.go(self.site + "/pages/blue.html", new_tab=True)
        blue = next(t["tab"] for t in self.d.tabs()["tabs"] if t["url"].endswith("blue.html"))
        self.d.switch(here)
        self.assertIn("ArrowLeft up", self.d.text()["text"], "leaving the tab let the key go")
        self.d.close_tab(blue)


if __name__ == "__main__":
    unittest.main()
