"""An agent works on copies of the person's files, and only checked bytes reach the person's
folders (workspace.py, copies.py; #953).

Run against the fake LibreOffice, with the real `AgentOffice`, `AgentWorkspace` and copies: what is
faked is only that the fake sees the work folder at the adapter's own paths, where the real one sees
it at /yantrik/work in its sandbox."""

import json
import os
import unittest
from unittest import mock

import support
from test_agent_files import SENTINEL, AgentCase
from yantrik_libreoffice import copies
from yantrik_libreoffice.workspace import HELD, lock_file


def read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


class TestCopies(AgentCase):
    def plan(self, text="plan"):
        return self.doc(self.home, "Documents", "plan.odt", text=text)

    def test_an_agent_works_on_a_copy_and_save_puts_it_back(self):
        path = self.plan()
        opened = self.agent("open", path=path)["result"]
        self.assertEqual(opened["path"], path, "answers name the person's file")
        work = self.agent_soffice.documents[-1].url
        self.assertNotIn(self.home, work, "LibreOffice has the copy, not the person's file")
        self.agent("write_text", text=" and more")
        self.assertEqual(read_json(path)["text"], "plan", "nothing reaches the file before save")
        saved = self.agent("save")["result"]
        self.assertEqual(saved["path"], path)
        self.assertEqual(read_json(path)["text"], "plan and more")

    def test_no_answer_or_describe_names_the_sandboxs_own_paths(self):
        path = self.plan()
        private = []
        for action, args in (("open", {"path": path}), ("read_text", {}),
                             ("write_text", {"text": "x"}), ("save", {}),
                             ("save_as", {"path": os.path.join(self.home, "Documents", "p2.odt")}),
                             ("export_pdf",
                              {"path": os.path.join(self.home, "Documents", "p.pdf")})):
            private.append(json.dumps(self.agent(action, **args)["result"]))
        _, state, _ = self.view()
        private.append(json.dumps(state))
        for said in private:
            self.assertNotIn("yantrik-lo-agent-", said)
        self.assertEqual([d["path"] for d in state["agent_documents"]],
                         [os.path.join(self.home, "Documents", "p2.odt")])

    def test_a_document_named_by_the_persons_path_is_found(self):
        path = self.plan(text="by path")
        self.agent("open", path=path)
        self.assertEqual(self.agent("read_text", document=path)["result"]["text"], "by path")

    def test_save_is_refused_when_the_file_changed_on_disk_and_the_change_is_kept(self):
        path = self.plan()
        self.agent("open", path=path)
        self.agent("write_text", text="agent's")
        support.fake_uno.write_doc(path, "writer", text="the person's later edit")
        said = self.agent_refused("save")
        self.assertEqual(said, copies.CHANGED % path)
        self.assertEqual(read_json(path)["text"], "the person's later edit")

    def test_save_is_refused_when_the_person_opened_the_file_meanwhile(self):
        path = self.plan()
        self.agent("open", path=path)
        self.act("open", path=path)
        self.assertEqual(self.agent_refused("save"), HELD % path)
        self.assertEqual(read_json(path)["text"], "plan")

    def test_save_as_creates_and_never_replaces_even_when_a_file_appears_meanwhile(self):
        self.agent("open", path=self.plan())
        target = os.path.join(self.home, "Documents", "copy.odt")
        real = copies.create

        def racing(data, folder, name, shown):
            support.fake_uno.write_doc(os.path.join(folder, name), "writer", text=SENTINEL)
            return real(data, folder, name, shown)
        with mock.patch.object(copies, "create", racing):
            said = self.agent_refused("save_as", path=target)
        self.assertIn("already at", said)
        self.assertEqual(read_json(target)["text"], SENTINEL, "what appeared there is kept")

    def test_export_pdf_writes_the_persons_new_file_and_leaves_no_copy_behind(self):
        self.agent("open", path=self.plan())
        target = os.path.join(self.home, "Documents", "plan.pdf")
        done = self.agent("export_pdf", path=target)["result"]
        self.assertEqual(done["path"], target)
        self.assertEqual(done["bytes"], os.path.getsize(target))
        work = self.agents._private.work  # noqa: SLF001 - the sandbox's folder
        self.assertFalse(any(name.endswith(".pdf") for _, _, names in os.walk(work)
                             for name in names))

    def test_close_drops_the_copy(self):
        self.agent("open", path=self.plan())
        work = self.agents._private.work  # noqa: SLF001
        self.assertEqual(len(os.listdir(work)), 1)
        self.agent("close")
        self.assertEqual(os.listdir(work), [])

    def test_a_file_with_a_second_name_is_not_copied(self):
        path = self.plan()
        os.link(path, os.path.join(self.home, "Documents", "other-name.odt"))
        self.assertIn("second name", self.agent_refused("open", path=path))
        self.assertEqual(self.agent_soffice.documents, [])

    def test_a_file_that_moved_while_it_was_opened_is_not_copied(self):
        path = self.plan()
        with mock.patch.object(copies, "_where", return_value="/elsewhere"):
            self.assertIn("moved", self.agent_refused("open", path=path))
        self.assertEqual(self.agent_soffice.documents, [])


class TestThePersonsFiles(AgentCase):
    """The person's open documents and lock files (#942 review L4)."""

    def test_a_file_the_person_has_open_is_refused_to_an_agent(self):
        theirs = os.path.join(self.home, "Documents", "shared.odt")
        support.fake_uno.write_doc(theirs, "writer", text=SENTINEL)
        self.act("open", path=theirs)
        self.assertEqual(self.agent_refused("open", path=theirs), HELD % theirs)
        self.assertEqual(self.agent_soffice.documents, [])

    def test_the_same_file_through_a_link_is_refused_too(self):
        theirs = os.path.join(self.home, "Documents", "shared.odt")
        support.fake_uno.write_doc(theirs, "writer", text=SENTINEL)
        self.act("open", path=theirs)
        os.makedirs(os.path.join(self.home, "Work"))
        link = os.path.join(self.home, "Work", "alias.odt")
        os.symlink(theirs, link)
        self.assertEqual(self.agent_refused("open", path=link), HELD % link)

    def test_a_lock_beside_the_real_file_is_seen_through_a_link(self):
        real = os.path.join(self.home, "Documents", "locked.odt")
        support.fake_uno.write_doc(real, "writer", text="x")
        with open(lock_file(real), "w"):
            pass
        os.makedirs(os.path.join(self.home, "Work"))
        link = os.path.join(self.home, "Work", "alias.odt")
        os.symlink(real, link)
        self.assertEqual(self.agent_refused("open", path=link), HELD % link)

    def test_an_agent_opening_its_own_document_again_is_answered_already_open(self):
        path = os.path.join(self.home, "Documents", "mine.odt")
        support.fake_uno.write_doc(path, "writer", text="mine")
        self.agent("open", path=path)
        self.assertTrue(self.agent("open", path=path)["result"]["already_open"])

    def test_a_file_the_rule_refuses_is_refused_before_anything_starts(self):
        path = self.doc(self.home, ".ssh", "id.odt")
        self.assertTrue(self.agent_refused("open", path=path).endswith("protected"))
        self.assertEqual(self.agents.starts, 0)

    def test_one_agent_office_serves_files_from_any_folder(self):
        for folder in ("Documents", "Later"):
            path = os.path.join(self.home, folder, "%s.odt" % folder)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            support.fake_uno.write_doc(path, "writer", text=folder)
            self.assertEqual(self.agent("open", path=path)["result"]["path"], path)
        self.assertEqual(self.agents.starts, 1)

    def test_describe_lists_the_agents_documents_apart_from_the_persons(self):
        path = os.path.join(self.home, "Documents", "book.ods")
        support.fake_uno.write_doc(path, "calc", sheets=[{"name": "S", "cells": {"A1": 1}}])
        self.agent("open", path=path)
        _, state, _ = self.view()
        self.assertEqual([(d["name"], d["path"]) for d in state["agent_documents"]],
                         [("book.ods", path)])
        self.assertEqual(state["documents"], [])


if __name__ == "__main__":
    unittest.main()
