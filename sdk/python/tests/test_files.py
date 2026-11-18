"""`yantrik_surface.files`: an agent's file is held to `home_paths`, asked of the shell, and the
verdicts are `verdict.rs`'s read off its answer. The person is not asked; a shell that does not
answer refuses an agent's file."""

import os
import shutil
import tempfile
import unittest

import support
from yantrik_surface import Refusal, files
from yantrik_surface.surface import _Scope


class TestAgentFiles(unittest.TestCase):
    def setUp(self):
        self.home = os.path.realpath(tempfile.mkdtemp(prefix="yantrik-sdk-files-"))
        self.addCleanup(shutil.rmtree, self.home, True)
        saved = os.environ.get("HOME")
        os.environ["HOME"] = self.home
        self.addCleanup(lambda: os.environ.pop("HOME") if saved is None else
                        os.environ.__setitem__("HOME", saved))
        os.makedirs(os.path.join(self.home, "Documents"))
        with open(os.path.join(self.home, "Documents", "plan.md"), "w") as f:
            f.write("plan")
        self.answers = {}

    def stat(self, asked):
        return self.answers.get(asked, {"exists": False, "reason": "not_found"})

    def as_agent(self):
        return _Scope(None, "tok-files")

    def test_the_person_is_not_asked(self):
        def no_shell(asked):
            raise AssertionError("the shell was asked about the person's own file")
        files.may_read_file("/etc/shadow", no_shell)
        files.may_write_file("~/.bashrc", no_shell)

    def test_reads_are_decided_from_the_shells_stat(self):
        self.answers = {
            "~/Documents/plan.md": {"exists": True, "kind": "file"},
            "~/Documents": {"exists": True, "kind": "directory"},
            "~/.ssh/id_ed25519": {"exists": "unknown", "reason": "protected"},
            "/etc/hostname": {"exists": "unknown", "reason": "outside"},
        }
        with self.as_agent():
            files.may_read_file("~/Documents/plan.md", self.stat)
            for asked, said in (("~/Documents", "~/Documents is not a file"),
                                ("~/gone.md", "there is no file at ~/gone.md")):
                with self.assertRaises(Refusal) as caught:
                    files.may_read_file(asked, self.stat)
                self.assertEqual(str(caught.exception), said)
            for asked, why in (("~/.ssh/id_ed25519", " is protected"), ("/etc/hostname", " is outside")):
                with self.assertRaises(Refusal) as caught:
                    files.may_read_file(asked, self.stat)
                self.assertTrue(str(caught.exception).endswith(asked + why), str(caught.exception))

    def test_writes_go_nowhere_hidden_through_no_link_and_make_no_folders(self):
        docs = os.path.join(self.home, "Documents")
        os.symlink(os.path.join(docs, "plan.md"), os.path.join(docs, "link.md"))
        self.answers = {
            "~/Documents/plan.md": {"exists": True, "kind": "file"},
            "~/Documents/link.md": {"exists": True, "kind": "file"},
            docs: {"exists": True, "kind": "directory"},
            "~/.gitconfig": {"exists": False},
            "~/bin/x": {"exists": False},
            "~/.config/yantrik/x": {"exists": "unknown", "reason": "protected"},
        }
        with self.as_agent():
            files.may_write_file("~/Documents/plan.md", self.stat)
            files.may_write_file("~/Documents/new.md", self.stat)
            for asked, ends in (("~/.gitconfig", "~/.gitconfig is hidden_place"),
                                ("~/bin/x", "~/bin/x is hidden_place"),
                                ("~/.config/yantrik/x", " is protected"),
                                ("~/Documents/link.md", "not through a link to one"),
                                ("~/new-folder/x.md", "makes no folders on the way")):
                with self.assertRaises(Refusal) as caught:
                    files.may_write_file(asked, self.stat)
                self.assertTrue(str(caught.exception).endswith(ends), str(caught.exception))
        # A link in the home to a hidden folder is hidden where it leads.
        os.makedirs(os.path.join(self.home, ".config"))
        os.symlink(os.path.join(self.home, ".config"), os.path.join(self.home, "cfg"))
        self.assertTrue(files.where_programs_look(os.path.join(self.home, "cfg", "git", "config"), self.home))
        self.assertFalse(files.where_programs_look(os.path.join(self.home, "Documents", "a.md"), self.home))

    def test_a_shell_that_does_not_answer_refuses_an_agents_file(self):
        os.environ["XDG_RUNTIME_DIR"] = self.home
        self.addCleanup(os.environ.pop, "XDG_RUNTIME_DIR", None)
        with self.as_agent():
            with self.assertRaises(Refusal) as caught:
                files.may_read_file("~/Documents/plan.md")
        self.assertIn("the shell did not say whether an agent may use", str(caught.exception))

    def test_the_hidden_rule_is_the_rusts_word_for_word(self):
        source = support.rust("crates/yantrik-ipc-contracts/src/home_paths/writes.rs")
        if source is None:
            self.skipTest("not in the repository")
        self.assertIn('"%s"' % files.HIDDEN_RULE, source)


if __name__ == "__main__":
    unittest.main()
