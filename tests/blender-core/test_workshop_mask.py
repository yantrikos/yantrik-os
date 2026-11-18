"""Blender does not bind its socket unless it sits in the sealed view of the workshops folder."""

import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.abspath(os.path.join(HERE, "..", "..", "apps", "blender", "addon")))
sys.path.insert(0, os.path.abspath(os.path.join(HERE, "..", "..", "sdk", "python")))

import fake_bpy  # noqa: E402
import yantrik_blender  # noqa: E402
from yantrik_blender import workshop_mask  # noqa: E402

WS = "/var/lib/yantrik/workshops"
ROOT = "26 1 8:1 / / rw - ext4 /dev/sda1 rw\n"
MASKED = ROOT + "40 26 0:35 / %s rw - tmpfs tmpfs rw,size=4k\n" % WS
UNMASKED = ROOT + "41 26 0:36 / /run/user/1000 rw - tmpfs tmpfs rw\n"


class TestMask(unittest.TestCase):
    def _mountinfo(self, text):
        f = tempfile.NamedTemporaryFile("w", suffix=".mountinfo", delete=False)
        f.write(text)
        f.close()
        self.addCleanup(os.unlink, f.name)
        return f.name

    def test_a_tmpfs_over_the_folder_is_the_sealed_view(self):
        self.assertTrue(workshop_mask.tmpfs_over(MASKED, WS))

    def test_no_tmpfs_over_it_is_not(self):
        self.assertFalse(workshop_mask.tmpfs_over(UNMASKED, WS))
        self.assertFalse(workshop_mask.tmpfs_over(
            ROOT + "40 26 0:35 / %s rw - ext4 /dev/sdb rw\n" % WS, WS))

    def test_the_last_mount_at_the_path_wins(self):
        late = MASKED + "42 26 8:2 / %s rw - ext4 /dev/sdb rw\n" % WS
        self.assertFalse(workshop_mask.tmpfs_over(late, WS))

    def test_refuses_when_the_folder_exists_and_nothing_covers_it(self):
        why = workshop_mask.refusal(WS, self._mountinfo(UNMASKED), exists=lambda p: True)
        self.assertIn(WS, why)
        self.assertIn("tmpfs", why)

    def test_allows_when_covered(self):
        self.assertIsNone(workshop_mask.refusal(WS, self._mountinfo(MASKED), exists=lambda p: True))

    def test_allows_when_there_is_no_folder(self):
        self.assertIsNone(workshop_mask.refusal(WS, self._mountinfo(UNMASKED), exists=lambda p: False))

    def test_an_unreadable_mountinfo_counts_as_not_covered(self):
        self.assertIsNotNone(workshop_mask.refusal(WS, "/nonexistent/mountinfo", exists=lambda p: True))


class TestStartDoesNotBind(unittest.TestCase):
    def test_start_returns_without_a_session_and_says_why(self):
        fake = fake_bpy.make_bpy(background=True)
        saved = sys.modules.get("bpy")
        sys.modules["bpy"] = fake
        orig = workshop_mask.refusal
        workshop_mask.refusal = lambda *a, **k: "control surface not bound: test"
        stderr, sys.stderr = sys.stderr, tempfile.TemporaryFile("w+")
        try:
            self.assertIsNone(yantrik_blender.start())
            sys.stderr.seek(0)
            self.assertIn("not bound", sys.stderr.read())
        finally:
            sys.stderr = stderr
            workshop_mask.refusal = orig
            if saved is None:
                sys.modules.pop("bpy", None)
            else:
                sys.modules["bpy"] = saved


if __name__ == "__main__":
    unittest.main()
