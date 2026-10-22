"""An agent names only files in the person's home, outside its protected places, and acts on no
document the person opened from anywhere else (#443; #942's audit). `read_text` is graded `safe`,
so before this an agent's `open ~/.ssh/id_ed25519` and `read_text` was any file the person can
read, asked about nothing. The person is not asked.

The shell's `files_stat` is stood in for by `stat` below, which answers as `home_paths::stat`
does for the cases here; the rule itself is tested where it lives, in yantrik-ipc-contracts."""

import os
import unittest

import support
from yantrik_libreoffice import build
from yantrik_libreoffice.surface import HIDDEN

SENTINEL = "SENTINEL-libreoffice-agent-91d2"


def no_reach(token, what):
    """The shell's answer to `reach_of` for a live token with no role."""
    return {"jsonrpc": "2.0", "id": 1, "result": {"accepted": True, "settled": True,
                                                  "result": {"reach": None, "known": True}}}


class TestAgentFiles(support.Case):
    def setUp(self):
        super().setUp()
        home = self.machine.home
        saved = os.environ.get("HOME")
        os.environ["HOME"] = home
        self.addCleanup(lambda: os.environ.pop("HOME") if saved is None else
                        os.environ.__setitem__("HOME", saved))
        self.home = home
        os.makedirs(os.path.join(home, "Documents"))
        os.makedirs(os.path.join(home, ".local", "share", "yantrik", "snippets"))
        os.makedirs(os.path.join(home, ".ssh"))
        self.asked = []
        self.surface = build(self.office, settings_path=self.machine.settings,
                             mode_path=self.machine.mode_file, ask_shell=no_reach, stat=self.stat)

    def stat(self, asked):
        """`home_paths::stat` for these cases: outside, protected, or what is there."""
        self.asked.append(asked)
        path = os.path.join(self.home, asked[2:]) if asked.startswith("~/") else asked
        if not path.startswith(self.home + os.sep):
            return {"exists": "unknown", "reason": "outside"}
        rest = os.path.relpath(path, self.home)
        if rest.startswith(".ssh") or rest.startswith(os.path.join(".local", "share", "yantrik")):
            return {"exists": "unknown", "reason": "protected"}
        if os.path.isfile(path):
            return {"exists": True, "kind": "file"}
        if os.path.isdir(path):
            return {"exists": True, "kind": "directory"}
        return {"exists": False, "reason": "not_found"}

    def agent(self, action, **args):
        return self.surface.act({"action": action, "args": args, "agent_token": "tok-lo"})

    def agent_refused(self, action, **args):
        with self.assertRaises(support.RpcError) as caught:
            self.agent(action, **args)
        return caught.exception.message

    def doc(self, *parts, text=SENTINEL):
        path = os.path.join(*parts)
        support.fake_uno.write_doc(path, "writer", text=text)
        return path

    def test_an_agent_opens_only_files_in_the_home(self):
        for path in (self.doc(self.home, ".ssh", "id_ed25519"),
                     self.doc(self.home, ".local", "share", "yantrik", "snippets", "snippets.odt"),
                     self.doc(self.files, "outside.odt")):
            said = self.agent_refused("open", path=path)
            self.assertTrue(said.endswith(" is protected") or said.endswith(" is outside"), said)
        self.assertEqual(self.soffice.documents, [], "nothing was opened")
        mine = self.doc(self.home, "Documents", "plan.odt", text="the agent's own")
        self.assertEqual(self.agent("open", path=mine)["result"]["path"], mine)
        self.assertEqual(self.agent("read_text")["result"]["text"], "the agent's own")
        # The person opens anything they can read, as before.
        self.assertEqual(self.result("open", path=self.doc(self.files, "theirs.odt"))["document"],
                         "theirs.odt")

    def test_a_document_the_person_opened_from_elsewhere_is_left_alone(self):
        theirs = self.doc(self.files, "theirs.odt")
        self.act("open", path=theirs)
        for action, args in (("read_text", {}), ("write_text", {"text": "x"}), ("save", {}),
                             ("save_as", {"path": os.path.join(self.home, "Documents", "copy.odt")}),
                             ("export_pdf", {"path": os.path.join(self.home, "Documents", "c.pdf")}),
                             ("close", {})):
            self.assertEqual(self.agent_refused(action, document="theirs.odt", **args), HIDDEN, action)
        self.assertFalse(os.path.exists(os.path.join(self.home, "Documents", "copy.odt")))
        self.assertIn(SENTINEL, self.result("read_text", document="theirs.odt")["text"],
                      "the person reads their own document")

    def test_an_agent_writes_only_where_it_may_write(self):
        self.agent("open", path=self.doc(self.home, "Documents", "plan.odt", text="plan"))
        for path, why in ((os.path.join(self.home, ".config", "autostart", "x.odt"), "hidden_place"),
                          (os.path.join(self.home, ".local", "share", "yantrik", "x.odt"), "protected"),
                          (os.path.join(self.files, "out.odt"), "outside")):
            self.assertTrue(self.agent_refused("save_as", path=path).endswith(why), path)
        self.assertTrue(self.agent_refused(
            "export_pdf", path=os.path.join(self.files, "out.pdf")).endswith(" is outside"))
        made = os.path.join(self.home, "Documents", "plan-copy.odt")
        self.assertEqual(self.agent("save_as", path=made)["result"]["path"], made)
        self.assertTrue(os.path.exists(made))


if __name__ == "__main__":
    unittest.main()
