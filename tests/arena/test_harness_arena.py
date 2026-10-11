"""harness_arena's own-events file: what the arena added itself, kept across the separate processes
of control, preflight and the main run, so the reset asks the calendar to delete only those -- and
never raises a refusal card for an event a mind made.

Stdlib only, nothing driven: `yos` and `act` are replaced for the tests that would call them.

    python3 -m unittest discover -s tests/arena -v      # pytest collects it too
"""
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness_arena as arena  # noqa: E402


class OwnEvents(unittest.TestCase):
    def setUp(self):
        # A private $XDG_RUNTIME_DIR per test: the real one holds a live run's file.
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = tmp.name
        old = os.environ.get("XDG_RUNTIME_DIR")
        os.environ["XDG_RUNTIME_DIR"] = self.dir
        self.addCleanup(lambda: os.environ.pop("XDG_RUNTIME_DIR", None) if old is None
                        else os.environ.__setitem__("XDG_RUNTIME_DIR", old))

    def replace(self, name, fake):
        real = getattr(arena, name)
        setattr(arena, name, fake)
        self.addCleanup(setattr, arena, name, real)

    def test_the_file_lives_in_the_runtime_dir(self):
        self.assertEqual(arena.own_events_path(), os.path.join(self.dir, "yantrik-arena-own-events.json"))

    def test_no_file_is_no_events(self):
        self.assertEqual(arena.read_own_events(), [])

    def test_ids_round_trip_once_each_in_order(self):
        for event_id in ("evt-1", "evt-2", "evt-1"):
            arena.remember_own_event(event_id)
        self.assertEqual(arena.read_own_events(), ["evt-1", "evt-2"])
        with open(arena.own_events_path(), encoding="utf-8") as f:
            self.assertEqual(json.load(f), ["evt-1", "evt-2"], "plain JSON another process can read")

    def test_a_write_leaves_no_temp_file_behind(self):
        arena.remember_own_event("evt-1")
        self.assertEqual(os.listdir(self.dir), ["yantrik-arena-own-events.json"])

    def test_clear_forgets_and_clearing_twice_is_fine(self):
        arena.remember_own_event("evt-1")
        arena.clear_own_events()
        self.assertEqual(arena.read_own_events(), [])
        arena.clear_own_events()

    def test_a_damaged_file_is_no_events_not_a_crash(self):
        for junk, want in (("{not json", []), (json.dumps({"id": "evt-1"}), []),
                           (json.dumps([1, "", None, "evt-2"]), ["evt-2"])):
            with open(arena.own_events_path(), "w", encoding="utf-8") as f:
                f.write(junk)
            self.assertEqual(arena.read_own_events(), want, junk)

    def test_an_added_event_is_remembered_by_the_id_the_calendar_answered(self):
        asked = []
        self.replace("yos", lambda *a, **k: asked.append(a) or json.dumps(
            {"accepted": True, "settled": True, "result": {"added": "Arena x", "id": "evt-9"}}, indent=2))
        arena.add_own_event(date="2026-09-30", time="15:00", title="Arena x", duration_min=30)
        self.assertEqual(arena.read_own_events(), ["evt-9"])
        self.assertEqual(asked[0][:3], ("act", "calendar", "add_event"))
        self.assertIn("--full", asked[0], "the reply as JSON, so the id is read and not scraped")

    def test_an_add_that_answered_no_id_remembers_nothing(self):
        self.replace("yos", lambda *a, **k:
                     "(yos timed out after 60s: act calendar add_event -- probably waiting on a card)")
        arena.add_own_event(date="2026-09-30", time="15:00", title="Arena x", duration_min=30)
        self.assertEqual(arena.read_own_events(), [])

    def test_reset_asks_the_calendar_only_about_the_arenas_own_events(self):
        # The point of the file: a mind-made "Arena ..." event never reaches `delete_own_event`,
        # so the calendar never refuses it in front of the person; the store sweep takes it.
        arena.remember_own_event("evt-arena")
        calls = []
        self.replace("act", lambda app, action, **kw: calls.append((app, action, kw)) or "")
        arena.delete_own_events()
        self.assertEqual(calls, [("calendar", "delete_own_event", {"id": "evt-arena"})])
        self.assertEqual(arena.read_own_events(), [], "cleared once asked")


if __name__ == "__main__":
    unittest.main()
