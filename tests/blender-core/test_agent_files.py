"""An agent names only files in the person's home, outside its protected places, and writes
nowhere programs read their startup (#443; #942's audit): `save` overwrote any file the person can
write, and `render` and `screenshot` wrote anywhere and made the folders on the way. The person is
not asked. The shell's `files_stat` is stood in for by `stat` below, which answers as
`home_paths::stat` does for these cases; the rule itself is tested in yantrik-ipc-contracts."""

import os
import shutil
import tempfile
import unittest

from test_dispatch import mode_file, refusal  # also puts the addon and the SDK on the path
import fake_bpy
from yantrik_blender.bridge import DirectBridge
from yantrik_blender.scene import Scene
from yantrik_blender.surface import Surface


def no_reach(token, what):
    """The shell's answer to `reach_of` for a live token with no role."""
    return {"jsonrpc": "2.0", "id": 1, "result": {"accepted": True, "settled": True,
                                                  "result": {"reach": None, "known": True}}}


class TestAgentFiles(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="yantrik-blender-files-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        for folder in ("Documents", ".ssh", os.path.join(".local", "share", "yantrik")):
            os.makedirs(os.path.join(self.home, folder))
        saved = os.environ.get("HOME")
        os.environ["HOME"] = self.home
        self.addCleanup(lambda: os.environ.pop("HOME") if saved is None else
                        os.environ.__setitem__("HOME", saved))
        self.fake = fake_bpy.make_bpy()
        settings = os.path.join(self.tmp, "settings.yaml")
        with open(settings, "w", encoding="utf-8") as f:
            f.write("tool_permission: dangerous\n")
        self.surface = Surface(Scene(self.fake), DirectBridge(), app_id="blender",
                               settings_path=settings, mode_path=mode_file("bypass_all"),
                               ask_shell=no_reach, stat=self.stat)

    def stat(self, asked):
        """`home_paths::stat` for these cases: outside, protected, or what is there."""
        if not asked.startswith(self.home + os.sep):
            return {"exists": "unknown", "reason": "outside"}
        rest = os.path.relpath(asked, self.home)
        if rest.startswith(".ssh") or rest.startswith(os.path.join(".local", "share", "yantrik")):
            return {"exists": "unknown", "reason": "protected"}
        if os.path.isfile(asked):
            return {"exists": True, "kind": "file"}
        if os.path.isdir(asked):
            return {"exists": True, "kind": "directory"}
        return {"exists": False, "reason": "not_found"}

    def agent(self, action, **args):
        return self.surface.act({"action": action, "args": args, "agent_token": "tok-blender"})

    def test_an_agent_reads_and_writes_only_files_in_the_home(self):
        outside = os.path.join(self.tmp, "elsewhere")
        os.makedirs(outside)
        cases = [
            ("import_model", "path", os.path.join(self.home, ".ssh", "id.obj"), " is protected"),
            ("import_model", "path", os.path.join(outside, "m.obj"), " is outside"),
            ("open", "path", os.path.join(self.home, ".local", "share", "yantrik", "s.blend"), " is protected"),
            ("save", "path", os.path.join(outside, "s.blend"), " is outside"),
            ("save", "path", os.path.join(self.home, ".config", "autostart", "s.blend"), "hidden_place"),
            ("render", "output", os.path.join(self.home, ".local", "share", "yantrik", "r.png"), " is protected"),
            ("screenshot", "output", os.path.join(outside, "v.png"), " is outside"),
            ("render", "output", os.path.join(self.home, "new-folder", "r.png"), "makes no folders on the way"),
        ]
        for action, key, path, why in cases:
            said = refusal(self, lambda: self.agent(action, **{key: path}))
            self.assertTrue(said.endswith(why), "%s %s: %s" % (action, path, said))
        self.assertFalse(os.path.exists(os.path.join(self.home, "new-folder")), "no folder was made")
        self.assertFalse(os.path.exists(os.path.join(outside, "s.blend")))
        # In the home, the agent's save goes on to the scene as before.
        mine = os.path.join(self.home, "Documents", "scene.blend")
        self.assertEqual(self.agent("save", path=mine)["result"]["saved"], mine)

    def test_the_person_is_not_asked(self):
        outside = os.path.join(self.tmp, "elsewhere")
        os.makedirs(outside)
        theirs = os.path.join(outside, "s.blend")
        answer = self.surface.act({"action": "save", "args": {"path": theirs}})
        self.assertEqual(answer["result"]["saved"], theirs)


if __name__ == "__main__":
    unittest.main()
