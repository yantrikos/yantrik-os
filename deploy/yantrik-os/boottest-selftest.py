#!/usr/bin/env python3
"""Self-test for boottest.py's session checks, the gate's first questions about tty1 (#26).

    python3 deploy/yantrik-os/boottest-selftest.py

The gate boots a real ISO, so what this tests is the logic that judges what the machine said:
the marked block is extracted from boottest.py and fed canned answers — the good machine, and
each way it breaks (getty off the console, the autologin drop-in gone, a console no one is
logged into). A check that stopped failing on a bad answer fails here, before a nightly finds
out by passing one.
"""
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
BOOTTEST = HERE / "boottest.py"
BEGIN = "# >>> session checks (#26)"
END = "# <<< session checks (#26)"

# What the good machine says, as the serial console returns it — login lines keep their
# carriage returns. The `who` line for the gate's own serial login sits next to the tty1
# one, as it does on the machine: the check has to want the tty1 line and not settle for
# any login there is.
GOOD = dict(getty_enabled="\r\nenabled\r", getty_active="\r\nactive\r",
            autologin="\r\n--autologin yantrik\r",
            who="yantrik tty1 2026-10-21 10:00\r\nyantrik ttyS0 2026-10-21 10:01")


def extract_checks():
    """The marked block, exec'd — boottest.py itself boots QEMU the moment it is imported."""
    lines = BOOTTEST.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(BEGIN)
        end = lines.index(END)
    except ValueError:
        raise AssertionError("boottest.py is missing the session check markers")
    assert start < end, "session check markers are out of order"
    ns = {}
    exec("\n".join(lines[start + 1:end]), ns)
    return ns["session_checks"]


session_checks = extract_checks()


def judged(**bad):
    return session_checks(**dict(GOOD, **bad))


class SessionChecks(unittest.TestCase):
    def test_the_good_machine_passes_all_four(self):
        results = session_checks(**GOOD)
        self.assertEqual(len(results), 4)
        self.assertTrue(all(ok for _, ok, _ in results), results)

    def test_a_cloud_machine_answer_fails_the_console_checks_and_says_why(self):
        # The cloud image: cloud-init disables getty@tty1, because there the session IS a
        # unit and the two fight over /dev/tty1. That machine answers like this and is not
        # what this gate boots — and a machine of ours that degraded into this answer is
        # exactly a machine with no desktop, so failing is right either way. The check's own
        # name has to carry the cloud-init comment's words that explain it: user-data.yaml,
        # at disable getty@tty1 — the fight, /dev/tty1, SIGHUP a second after every start.
        results = judged(getty_enabled="disabled", getty_active="inactive")
        self.assertEqual([ok for _, ok, _ in results], [False, False, True, True])
        for needle in ("getty@tty1", "/dev/tty1", "SIGHUP"):
            self.assertIn(needle, results[1][0])

    def test_no_desktop_means_no_active_console(self):
        # failed is a getty that started and died; inactive is none on the console at all.
        # Neither is a machine the desktop can arrive on, however enabled it claims to be.
        for answer in ("failed", "inactive", "deactivating"):
            with self.subTest(answer=answer):
                self.assertFalse(judged(getty_active=answer)[1][1])

    def test_a_console_not_started_at_boot_fails(self):
        # enabled is the only answer of a machine that boots to this console by itself.
        for answer in ("static", "disabled", "generated", "alias"):
            with self.subTest(answer=answer):
                self.assertFalse(judged(getty_enabled=answer)[0][1])

    def test_a_missing_autologin_drop_in_fails_and_takes_the_desktop_with_it(self):
        # No drop-in, no autologin: the console sits at a prompt and tty1 never logs in, so
        # the login check fails on the same machine for its own reason.
        results = judged(autologin="grep: /etc/systemd/system/getty@tty1.service.d/"
                                    "autologin.conf: No such file or directory",
                         who="")
        self.assertEqual([ok for _, ok, _ in results], [True, True, False, False])
        self.assertIn("No such file", results[2][2])

    def test_an_autologin_for_someone_else_fails(self):
        for answer in ("--autologin root",           # a console that logs in the wrong person
                       "--autologin",                 # cut off mid-line
                       ""):                          # a drop-in with no autologin in it
            with self.subTest(answer=answer):
                self.assertFalse(judged(autologin=answer)[2][1])

    def test_a_console_no_one_is_logged_into_fails_the_login_check(self):
        # The gate's own serial login proves nothing about tty1 — that is the machine whose
        # autologin never fired and the prompt is sitting there unseen behind the splash.
        results = judged(who="yantrik ttyS0 2026-10-21 10:01")
        self.assertFalse(results[3][1])
        self.assertIn("ttyS0", results[3][2])
        for answer in ("", "root tty1 2026-10-21 10:00"):
            with self.subTest(answer=answer):
                self.assertFalse(judged(who=answer)[3][1])

    def test_an_ask_that_timed_out_fails_every_check(self):
        results = judged(getty_enabled="", getty_active="", autologin="", who="")
        self.assertEqual([ok for _, ok, _ in results], [False, False, False, False])


if __name__ == "__main__":
    unittest.main(verbosity=2)
