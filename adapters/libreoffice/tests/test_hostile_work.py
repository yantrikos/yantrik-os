"""The sandbox's work folder is hostile ground: nothing the adapter reads or writes there follows
what the sandbox left in it (workdir.py, copies.py; #953 review H1).

Code in the sandbox (a filter exploit, a macro) can change anything in the work folder. Each test
below plays that code, at the moment the review found, through the fake LibreOffice: the real
`AgentWorkspace`, copies and work folder do the rest. Each attack has a control that shows it
reaches the protected file when the work folder is read the way it was before the fix, so a
test that passes is a guard that holds, not an attack that missed."""

import json
import os
import shutil
import tempfile
import unittest
import urllib.parse
from unittest import mock

import support
from test_agent_files import AgentCase
from test_fence import put
from yantrik_libreoffice import copies, workdir

SECRET = "SECRET-KEY-MATERIAL-h1-7c03"


def read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def local(url):
    return urllib.parse.unquote(url[len("file://"):])


def text_of(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


class Hostile(AgentCase):
    def setUp(self):
        super().setUp()
        self.secret = put(os.path.join(self.home, ".ssh", "id_ed25519"), SECRET)
        self.plan = self.doc(self.home, "Documents", "plan.odt", text="plan")
        self.documents = os.path.join(self.home, "Documents")

    def plant(self, path):
        """What the sandbox's code does: its file swapped for a link to the person's key."""
        os.remove(path)
        os.symlink(self.secret, path)

    def then_plant(self, method):
        """The fake document's `method`, which plants the link once LibreOffice has written."""
        doc = self.agent_soffice.documents[-1]
        real = getattr(type(doc), method)
        test = self

        def planting(me, *args):
            real(me, *args)
            test.plant(local(args[0] if args else me.url))
        return mock.patch.object(type(doc), method, planting)

    def test_control_a_link_planted_in_the_work_folder_reaches_the_key(self):
        self.agent("open", path=self.plan)
        work = local(self.agent_soffice.documents[-1].url)
        self.plant(work)
        with open(work, encoding="utf-8") as f:   # how put_back read it before the fix
            self.assertEqual(f.read(), SECRET)

    def test_save_does_not_put_a_planted_link_back_over_the_persons_file(self):
        self.agent("open", path=self.plan)
        with self.then_plant("store"):
            said = self.agent_refused("save")
        self.assertIn("not a plain file of its own", said)
        self.assertEqual(read_json(self.plan)["text"], "plan")
        self.assertNotIn(SECRET, text_of(self.plan))

    def test_save_as_and_export_pdf_do_not_copy_a_planted_link_out(self):
        for action, target, method in (("save_as", "copy.odt", "storeAsURL"),
                                       ("export_pdf", "plan.pdf", "storeToURL")):
            self.agent("open", path=self.plan)
            target = os.path.join(self.documents, target)
            with self.then_plant(method):
                said = self.agent_refused(action, path=target)
            self.assertIn("not a plain file of its own", said, action)
            self.assertFalse(os.path.lexists(target), action)
            self.agent("close")

    def test_a_refused_store_puts_the_old_bytes_back_over_the_link_not_through_it(self):
        book = os.path.join(self.documents, "book.ods")
        support.fake_uno.write_doc(book, "calc", sheets=[{"name": "S", "cells": {"A1": 1}}])
        before = text_of(book)
        self.agent("open", path=book)
        doc = self.agent_soffice.documents[-1]
        real = type(doc).store
        test = self

        def planting_and_linking(me):
            real(me)
            test.plant(local(me.url))
            me.external_links.append("file:///elsewhere/data.ods")
        with mock.patch.object(type(doc), "store", planting_and_linking):
            self.assertIn("taken back", self.agent_refused("save"))
        self.assertEqual(text_of(self.secret), SECRET, "the key was not written over")
        self.assertEqual(text_of(book), before)

    def test_a_slot_swapped_for_a_link_to_a_folder_gets_no_copy_written_into_that_folder(self):
        autostart = os.path.join(self.home, ".config", "autostart")
        os.makedirs(autostart)
        real = copies._read  # noqa: SLF001

        def swapping(path):
            got = real(path)
            work = self.agents._private.work  # noqa: SLF001
            for name in os.listdir(work):
                slot = os.path.join(work, name)
                if os.path.isdir(slot) and not os.path.islink(slot):
                    os.rename(slot, slot + "-moved")
                    os.symlink(autostart, slot)
            return got
        with mock.patch.object(copies, "_read", swapping):
            try:
                self.agent("open", path=self.plan)
            except support.RpcError:
                pass
        self.assertEqual(os.listdir(autostart), [], "nothing was written into the person's folder")


class TestWorkFolder(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="yantrik-workdir-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        os.mkdir(os.path.join(self.tmp, "work"))
        self.secret = put(os.path.join(self.tmp, "home", ".ssh", "key"), SECRET)
        self.folder = workdir.WorkFolder(os.path.join(self.tmp, "work"))
        self.addCleanup(self.folder.close)
        self.slot = self.folder.slot()
        self.path = os.path.join(self.slot, "book.ods")

    def refused(self, call, *args):
        with self.assertRaises(support.OfficeError) as caught:
            call(*args)
        self.assertNotIn(SECRET, str(caught.exception))
        return str(caught.exception)

    def test_a_link_a_fifo_and_a_second_name_are_not_read(self):
        os.symlink(self.secret, self.path)
        self.assertIn("not a plain file", self.refused(self.folder.read, self.path))
        os.remove(self.path)
        os.mkfifo(self.path)
        self.assertIn("not a regular file", self.refused(self.folder.read, self.path))
        os.remove(self.path)
        put(self.path, "mine")
        os.link(self.path, os.path.join(self.slot, "again"))
        self.assertIn("second name", self.refused(self.folder.read, self.path))

    def test_a_file_too_large_is_not_read(self):
        put(self.path, "x" * 64)
        with mock.patch.object(copies, "MAX_BYTES", 16):
            self.assertIn("bytes", self.refused(self.folder.read, self.path))

    def test_only_a_name_inside_a_slot_this_adapter_made_is_reached(self):
        for path in (os.path.join(self.slot, "..", "..", "home", ".ssh", "key"),
                     os.path.join(self.tmp, "work", "other", "book.ods"),
                     os.path.join(self.slot, "deeper", "book.ods"), self.secret):
            self.assertIn("made room for", self.refused(self.folder.read, path))

    def test_a_renamed_slot_is_still_reached_by_its_handle_never_by_its_old_path(self):
        put(os.path.join(self.tmp, "home", "Documents", "keep"), "keep")
        os.rename(self.slot, self.slot + "-moved")
        os.symlink(os.path.join(self.tmp, "home", "Documents"), self.slot)
        self.folder.write_new(self.path, b"copy")
        self.assertEqual(os.listdir(os.path.join(self.tmp, "home", "Documents")), ["keep"])
        self.assertEqual(text_of(os.path.join(self.slot + "-moved", "book.ods")), "copy")
        self.assertEqual(self.folder.read(self.path), b"copy")

    def test_replace_and_remove_act_on_a_link_never_on_what_it_points_to(self):
        os.symlink(self.secret, self.path)
        self.folder.replace(self.path, b"old bytes")
        self.assertEqual(text_of(self.secret), SECRET)
        self.assertEqual(self.folder.read(self.path), b"old bytes")
        os.remove(self.path)
        os.symlink(self.secret, self.path)
        self.folder.remove(self.path)
        self.assertTrue(os.path.exists(self.secret))
        self.assertIsNone(self.folder.size(self.path))

    def test_control_without_the_handle_a_swapped_slot_writes_into_the_persons_folder(self):
        os.rename(self.slot, self.slot + "-moved")
        os.symlink(os.path.join(self.tmp, "home", ".ssh"), self.slot)
        with open(self.path, "wb") as f:      # how take wrote the copy before the fix
            f.write(b"copy")
        self.assertIn("book.ods", os.listdir(os.path.join(self.tmp, "home", ".ssh")))


class TestPutBack(unittest.TestCase):
    """L4: the person's file is looked at again just before the rename."""

    def test_a_file_replaced_after_the_digest_is_kept(self):
        tmp = tempfile.mkdtemp(prefix="yantrik-putback-")
        self.addCleanup(shutil.rmtree, tmp, True)
        real = put(os.path.join(tmp, "plan.odt"), "plan")
        copy = copies.Copy(real, real, None, copies.digest_of(b"plan"), 0o600)
        made = copies._write_new  # noqa: SLF001

        def racing(name, data, mode=0o600, dir_fd=None):
            made(name, data, mode, dir_fd)
            put(os.path.join(tmp, "elsewhere"), "the person's")
            os.replace(os.path.join(tmp, "elsewhere"), real)
        with mock.patch.object(copies, "_write_new", racing):
            with self.assertRaises(support.OfficeError) as caught:
                copies.put_back(copy, b"agent's")
        self.assertEqual(str(caught.exception), copies.CHANGED % real)
        self.assertEqual(text_of(real), "the person's")
        self.assertEqual(sorted(os.listdir(tmp)), ["plan.odt"], "no temporary file is left")


if __name__ == "__main__":
    unittest.main()
