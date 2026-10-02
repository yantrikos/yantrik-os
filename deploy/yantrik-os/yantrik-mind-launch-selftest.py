#!/usr/bin/env python3
"""Selftest for yantrik-mind-launch: how it reads the Mind's settings, the two names it gives
the Mind when the settings leave them out, and the person uid it never takes from them.
Run: python3 deploy/yantrik-os/yantrik-mind-launch-selftest.py"""
import importlib.machinery
import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
_loader = importlib.machinery.SourceFileLoader("mind_launch", os.path.join(HERE, "yantrik-mind-launch"))
_spec = importlib.util.spec_from_loader("mind_launch", _loader)
launch = importlib.util.module_from_spec(_spec)
_loader.exec_module(launch)

SYSTEM = (
    "root:x:0:0:root:/root:/bin/bash\n"
    "yantrik-mind:x:999:999:Yantrik Mind:/var/lib/yantrik-mind:/usr/sbin/nologin\n"
    "nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n"
)


class Passwd:
    def __init__(self, text):
        self.text = text

    def __enter__(self):
        fd, self.path = tempfile.mkstemp()
        with os.fdopen(fd, "w") as f:
            f.write(self.text)
        return self.path

    def __exit__(self, *_):
        os.unlink(self.path)


class PersonName(unittest.TestCase):
    def name(self, extra):
        with Passwd(SYSTEM + extra) as p:
            return launch.person_name(p)

    def test_the_one_person_is_named_by_their_full_name(self):
        self.assertEqual(self.name("yantrik:x:1000:1000:Yantrik Live,,,:/home/yantrik:/bin/bash\n"), "Yantrik Live")

    def test_a_person_without_a_full_name_is_their_login(self):
        self.assertEqual(self.name("asha:x:1000:1000::/home/asha:/bin/bash\n"), "asha")

    def test_no_person_or_several_is_no_guess(self):
        self.assertIsNone(self.name(""))
        self.assertIsNone(self.name(
            "asha:x:1000:1000:Asha:/home/asha:/bin/bash\nravi:x:1001:1001:Ravi:/home/ravi:/bin/bash\n"))

    def test_a_locked_out_account_in_the_person_range_is_not_a_person(self):
        self.assertEqual(self.name(
            "asha:x:1000:1000:Asha:/home/asha:/bin/bash\nsvc:x:1001:1001:Svc:/:/usr/sbin/nologin\n"), "Asha")

    def test_control_characters_never_reach_the_persona(self):
        self.assertEqual(self.name("a:x:1000:1000:Asha\x1b[2J\x07 Rao:/home/a:/bin/bash\n"), "Asha[2J Rao")

    def test_an_unreadable_file_is_no_name(self):
        self.assertIsNone(launch.person_name("/nonexistent/passwd"))


class FillDefaults(unittest.TestCase):
    PERSON = "yantrik:x:1000:1000:Yantrik Live:/home/yantrik:/bin/bash\n"

    def filled(self, env, passwd=PERSON):
        with Passwd(SYSTEM + passwd) as p:
            launch.fill_defaults(env, p)
        return env

    def test_the_mind_is_given_the_pickers_name_and_its_persons(self):
        env = self.filled({})
        self.assertEqual(env["YM_MIND_NAME"], "Yantrik Mind")
        self.assertEqual(env["YM_OPERATOR"], "Yantrik Live")

    def test_the_settings_always_win(self):
        env = self.filled({"YM_MIND_NAME": "Friday", "YM_OPERATOR": ""})
        self.assertEqual(env, {"YM_MIND_NAME": "Friday", "YM_OPERATOR": ""})

    def test_with_no_one_person_the_operator_is_left_unset(self):
        env = self.filled({}, passwd="")
        self.assertNotIn("YM_OPERATOR", env)
        self.assertEqual(env["YM_MIND_NAME"], "Yantrik Mind")


class PersonUid(unittest.TestCase):
    """Whose shell vouches for memory credentials (#447). Without it every `mem-` bearer is a 401."""

    PERSON = "yantrik:x:1000:1000:Yantrik Live:/home/yantrik:/bin/bash\n"

    def uid(self, env, unit_set="", passwd=PERSON):
        with Passwd(SYSTEM + passwd) as p:
            return launch.person_uid(env, unit_set, p), env

    def test_the_one_person_is_who_vouches(self):
        why, env = self.uid({})
        self.assertIsNone(why)
        self.assertEqual(env["YANTRIK_PERSON_UID"], "1000")

    def test_the_settings_file_can_never_choose_who_vouches(self):
        # Read from the file into env by setdefault, as main() does, and still not believed: the
        # mind account writes that file, and would otherwise pick whose shell answers for it.
        why, env = self.uid({"YANTRIK_PERSON_UID": "0"})
        self.assertIsNone(why)
        self.assertEqual(env["YANTRIK_PERSON_UID"], "1000")
        why, env = self.uid({"YANTRIK_PERSON_UID": "1000"}, passwd="")
        self.assertNotIn("YANTRIK_PERSON_UID", env, "a file's value survives no person either")

    def test_the_units_own_value_wins_over_the_one_person(self):
        why, env = self.uid({"YANTRIK_PERSON_UID": "1001"}, unit_set="1002")
        self.assertIsNone(why)
        self.assertEqual(env["YANTRIK_PERSON_UID"], "1002")

    def test_no_person_or_several_leaves_it_unset_and_says_why_in_one_line(self):
        why, env = self.uid({}, passwd="")
        self.assertNotIn("YANTRIK_PERSON_UID", env)
        self.assertIn("no person's account", why)
        why, env = self.uid({}, passwd=self.PERSON + "ravi:x:1001:1001:Ravi:/home/ravi:/bin/bash\n")
        self.assertNotIn("YANTRIK_PERSON_UID", env)
        self.assertIn("2 people's accounts", why)
        self.assertNotIn("\n", why)

    def test_an_unreadable_passwd_is_no_guess(self):
        env = {}
        why = launch.person_uid(env, "", "/nonexistent/passwd")
        self.assertNotIn("YANTRIK_PERSON_UID", env)
        self.assertIn("cannot be read", why)

    def test_the_uid_and_the_operator_name_come_from_the_same_person(self):
        # One lookup for both, so the Mind cannot greet one person and vouch through another.
        with Passwd(SYSTEM + "svc:x:1001:1001:Svc:/:/usr/sbin/nologin\n" + self.PERSON) as p:
            env = {}
            launch.fill_defaults(env, p)
            launch.person_uid(env, "", p)
        self.assertEqual((env["YM_OPERATOR"], env["YANTRIK_PERSON_UID"]), ("Yantrik Live", "1000"))


class Parse(unittest.TestCase):
    def test_settings_are_read_as_systemd_reads_an_environment_file(self):
        self.assertEqual(
            launch.parse("# c\n\nexport A=1\nB='x y'\nC=\"q \\\"z\\\"\"\nbad line\n1X=no\n"),
            {"A": "1", "B": "x y", "C": 'q "z"'},
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
