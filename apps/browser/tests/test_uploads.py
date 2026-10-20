"""Which file `upload` may send (#874): the shell's answer, and the bytes themselves."""

import hashlib
import os
import stat as st
import tempfile
import unittest

import fakes
from yantrik_browser import uploads


class Home(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="yb-upload-")
        self.addCleanup(self.tmp.cleanup)
        self.home = os.path.realpath(self.tmp.name)
        os.makedirs(os.path.join(self.home, "Pictures"))
        self.photo = self.write("Pictures/emblem.png", b"\x89PNG emblem")
        self.stat = fakes.files_stat(self.home)

    def write(self, rel, data):
        path = os.path.join(self.home, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)
        return path

    def check(self, path, stat=None, **kw):
        return uploads.check(path, stat=stat or self.stat, home=self.home, **kw)

    def refused(self, path, stat=None, **kw):
        with self.assertRaises(uploads.Refused) as caught:
            self.check(path, stat=stat, **kw)
        return str(caught.exception)


class TheFileThatMayGo(Home):
    def test_a_file_in_the_home_is_read_and_hashed(self):
        facts = self.check("~/Pictures/emblem.png")
        self.assertEqual(facts.path, self.photo)
        self.assertEqual(facts.size, len(b"\x89PNG emblem"))
        self.assertEqual(facts.sha256, hashlib.sha256(b"\x89PNG emblem").hexdigest())
        self.assertEqual(facts.shown, facts.sha256[:uploads.SHA_SHOWN])
        self.assertEqual(facts.data, b"\x89PNG emblem")

    def test_the_limit_is_a_named_25_mib(self):
        self.assertEqual(uploads.UPLOAD_LIMIT, 25 * 1024 * 1024)


class TheGuardIsTheShells(Home):
    def test_a_protected_path_is_refused_with_the_reason_and_never_opened(self):
        key = self.write(".ssh/id_ed25519", b"-----BEGIN OPENSSH PRIVATE KEY-----")
        os.chmod(key, 0)        # opening it would fail differently; it must not get that far
        message = self.refused("~/.ssh/id_ed25519")
        self.assertIn("protected place", message)
        self.assertIn("Nothing was uploaded", message)

    def test_every_reason_the_shell_gives_is_a_refusal(self):
        for reason in ("protected", "outside", "not_allowed", "broken_link", "hard_link",
                       "not_a_path", "not_found", "something_new"):
            message = self.refused(self.photo, stat=lambda p, r=reason: {"exists": "unknown", "reason": r})
            self.assertIn("Nothing was uploaded", message, reason)

    def test_a_shell_that_cannot_be_asked_refuses(self):
        def gone(path):
            raise ConnectionRefusedError("no shell")
        self.assertIn("could not say whether", self.refused(self.photo, stat=gone))

    def test_a_shell_answer_that_is_not_a_file_is_refused(self):
        os.makedirs(os.path.join(self.home, "folder"))
        self.assertIn("not a file", self.refused("~/folder"))

    def test_a_key_or_pem_anywhere_is_refused(self):
        self.write("work/server.key", b"key")
        self.assertIn("protected place", self.refused("~/work/server.key"))

    def test_outside_the_home_is_refused(self):
        self.assertIn("outside the person's home", self.refused("/etc/hostname"))


class NoLinkAnywhere(Home):
    def test_a_link_to_an_allowed_file_is_refused(self):
        os.symlink(self.photo, os.path.join(self.home, "link.png"))
        self.assertIn("through a link", self.refused("~/link.png"))

    def test_a_linked_folder_on_the_way_is_refused(self):
        os.symlink(os.path.join(self.home, "Pictures"), os.path.join(self.home, "pics"))
        self.assertIn("through a link", self.refused("~/pics/emblem.png"))

    def test_a_link_is_refused_here_even_when_the_shell_misses_it(self):
        # Defence in depth: the walk from / is this module's own.
        link = os.path.join(self.home, "link.png")
        os.symlink(self.photo, link)
        lying = lambda p: {"exists": True, "kind": "file", "real": p, "via_link": False}
        message = self.refused(link, stat=lying)
        self.assertIn("goes through a link (%s)" % link, message)

    def test_a_link_above_the_home_is_refused(self):
        alias = os.path.join(self.home, "..", os.path.basename(self.home) + "-alias")
        os.symlink(self.home, alias)
        self.addCleanup(os.unlink, alias)
        path = os.path.normpath(os.path.join(alias, "Pictures", "emblem.png"))
        lying = lambda p: {"exists": True, "kind": "file", "real": p, "via_link": False}
        self.assertIn("goes through a link", self.refused(path, stat=lying))


class OnlyARegularFileOfOneName(Home):
    lying = staticmethod(lambda p: {"exists": True, "kind": "file", "real": p, "via_link": False})

    def test_a_fifo_is_refused_without_blocking(self):
        fifo = os.path.join(self.home, "pipe.png")
        os.mkfifo(fifo)
        self.assertIn("not a regular file", self.refused(fifo, stat=self.lying))

    def test_a_device_is_refused(self):
        # /dev/null as if the shell had said it was a file: the bytes check still refuses.
        self.assertIn("not a regular file", uploads_refusal("/dev/null", self.lying))

    def test_a_second_name_is_refused(self):
        os.link(self.photo, os.path.join(self.home, "twin.png"))
        self.assertIn("second name", self.refused("~/twin.png"))
        self.assertIn("changed while it was being checked", self.refused("~/twin.png", stat=self.lying))

    def test_an_unreadable_file_is_refused(self):
        if os.geteuid() == 0:
            self.skipTest("root reads a file of mode 000")
        os.chmod(self.photo, 0)
        self.assertIn("cannot be opened as your account", self.refused(self.photo))


class TheSizeLimit(Home):
    def test_a_26_mib_file_is_refused_before_it_is_read(self):
        big = os.path.join(self.home, "big.bin")
        with open(big, "wb") as f:
            f.truncate(26 * 1024 * 1024)
        message = self.refused(big)
        self.assertIn("26.0 MiB", message)
        self.assertIn("over the 25.0 MiB", message)

    def test_exactly_the_limit_is_allowed_and_one_byte_more_is_not(self):
        path = self.write("edge.bin", b"x" * 64)
        self.assertEqual(self.check(path, limit=64).size, 64)
        self.assertIn("over the 63 bytes", self.refused(path, limit=63))

    def test_a_file_that_grows_after_the_look_is_still_read_no_further_than_the_limit(self):
        path = self.write("grows.bin", b"x" * 10)
        real_fstat = os.fstat

        def small(fd):
            info = real_fstat(fd)
            return os.stat_result((info.st_mode, info.st_ino, info.st_dev, info.st_nlink,
                                   info.st_uid, info.st_gid, 5, info.st_atime, info.st_mtime,
                                   info.st_ctime))
        os.fstat = small
        try:
            self.assertIn("over the 8 bytes", self.refused(path, limit=8))
        finally:
            os.fstat = real_fstat


class WhatTheCardCanShow(Home):
    def test_controls_and_bidi_marks_in_a_path_are_refused(self):
        for bad in ("~/Pictures/a‮gnp.exe", "~/Pictures/a\x1b[2J.png", "~/a​.png",
                    "~/a\nb.png", "~/a b.png"):
            self.assertIn("characters a card cannot show", self.refused(bad), repr(bad))

    def test_relative_and_dotdot_paths_are_refused(self):
        self.assertIn("not an absolute path", self.refused("Pictures/emblem.png"))
        self.assertIn("goes up with", self.refused("~/Pictures/../.ssh/id_ed25519"))


class TheStagedCopy(Home):
    def setUp(self):
        super().setUp()
        self.runtime = os.path.join(self.home, "run")
        os.mkdir(self.runtime, 0o700)

    def test_the_page_gets_a_private_read_only_copy_of_the_checked_bytes(self):
        facts = self.check(self.photo)
        staged = uploads.stage(facts, self.runtime)
        self.assertEqual(os.path.basename(staged), "emblem.png")
        self.assertNotEqual(staged, self.photo)
        with open(staged, "rb") as f:
            self.assertEqual(f.read(), b"\x89PNG emblem")
        self.assertEqual(st.S_IMODE(os.stat(staged).st_mode), 0o400)
        self.assertEqual(st.S_IMODE(os.stat(os.path.dirname(staged)).st_mode), 0o700)
        # The person's file changing afterwards changes nothing that leaves.
        with open(self.photo, "wb") as f:
            f.write(b"something else")
        with open(staged, "rb") as f:
            self.assertEqual(f.read(), b"\x89PNG emblem")

    def test_a_planted_link_for_the_private_folder_is_refused(self):
        os.symlink("/tmp", os.path.join(self.runtime, "yantrik-browser"))
        with self.assertRaises(uploads.Refused):
            uploads.stage(self.check(self.photo), self.runtime)

    def test_a_folder_open_to_others_is_refused(self):
        os.mkdir(os.path.join(self.runtime, "yantrik-browser"), 0o755)
        os.chmod(os.path.join(self.runtime, "yantrik-browser"), 0o755)
        with self.assertRaises(uploads.Refused):
            uploads.stage(self.check(self.photo), self.runtime)

    def test_no_runtime_folder_refuses(self):
        with self.assertRaises(uploads.Refused):
            uploads.stage(self.check(self.photo), "")

    def test_old_copies_are_swept(self):
        old = uploads.stage(self.check(self.photo), self.runtime)
        root = uploads.staging_root(self.runtime)
        os.utime(os.path.dirname(old), (0, 0))
        uploads.sweep(root)
        self.assertFalse(os.path.exists(old))


def uploads_refusal(path, stat):
    try:
        uploads.check(path, stat=stat, home="/")
    except uploads.Refused as e:
        return str(e)
    raise AssertionError("%s was not refused" % path)


if __name__ == "__main__":
    unittest.main()
