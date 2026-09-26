"""The Hermes plugin's desktop half: the parts that carry a turn across a shell restart (#246).

The adapter itself imports the Hermes gateway and is exercised live; `desktop.py` is plain
Python, so what it decides is pinned here.
"""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "hermes"))

import desktop  # noqa: E402


class DesktopTests(unittest.TestCase):
    def test_no_desktop_at_the_socket_is_gone_not_a_refusal(self):
        missing = os.path.join(tempfile.mkdtemp(), "harness.sock")
        with self.assertRaises(desktop.HarnessError) as caught:
            desktop.call(missing, desktop.POLL, {"session": "s1"}, timeout=1)
        self.assertTrue(caught.exception.gone, "nothing there: hold the work for the next desktop")

    def test_a_turn_keeps_its_gateway_name_and_learns_its_new_desktop_id(self):
        turn = desktop.Turn(turn_id="18", chat_id="yantrik", text="build the town")
        self.assertEqual(turn.on_desktop(), 18, "until a restart, the two are the same")
        turn.desktop_turn = 3
        self.assertEqual((turn.turn_id, turn.on_desktop()), ("18", 3),
                         "the gateway still names it 18; the desktop that came back knows it as 3")


if __name__ == "__main__":
    unittest.main()
