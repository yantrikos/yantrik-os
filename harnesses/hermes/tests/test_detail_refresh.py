"""Hermes says its model again when the person picks another (#1114): once per change, per turn.

    python3 -m unittest discover -s harnesses/hermes/tests -p test_detail_refresh.py
"""

import unittest
from unittest import mock

import test_adapter_behaviour as base
from test_adapter_behaviour import setUpModule, tearDownModule  # noqa: F401  (the faked Hermes, set up here too)


class DetailRefreshTests(base.AdapterCase):
    async def test_a_changed_pick_is_sent_before_the_turn_and_an_unchanged_one_is_not(self):
        self.adapter._detail_said = "Hermes 1 · deepseek"
        with mock.patch.object(base.adapter_mod, "_detail", return_value="Hermes 1 · qwen3.8:27b"):
            await self.adapter._refresh_detail()
            await self.adapter._refresh_detail()
        sent = self.fake.of(base.desktop_mod.DETAIL)
        self.assertEqual(sent, [{"session": "s1", "detail": "Hermes 1 · qwen3.8:27b"}])
        self.assertEqual(self.fake.of(base.desktop_mod.ATTACH), [], "the line changes without attaching again")

    async def test_a_desktop_that_refuses_it_does_not_stop_the_turn(self):
        self.adapter._detail_said = "old"
        def refuse(*args, **kwargs):
            raise base.desktop_mod.HarnessError("no")

        base.desktop_mod.call = refuse  # AdapterCase.tearDown puts the fake back
        with mock.patch.object(base.adapter_mod, "_detail", return_value="new"):
            await self.adapter._refresh_detail()
        self.assertEqual(self.adapter._detail_said, "old", "tried again on the next turn")


if __name__ == "__main__":
    unittest.main()
