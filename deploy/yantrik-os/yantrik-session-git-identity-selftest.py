#!/usr/bin/env python3
"""Selftest for git's default identity (#680, moved by #548): yantrik-session-install writes
user.name and user.email into the SYSTEM gitconfig of the root it installs into, so the session
no longer reads or writes ~/.gitconfig before the lock screen. It runs the real script against a
scratch root, with an etc/passwd and etc/hostname of the test's own. Run:
python3 deploy/yantrik-os/yantrik-session-git-identity-selftest.py"""
import configparser
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
INSTALL = os.path.join(HERE, "yantrik-session-install")
SESSION = os.path.join(HERE, "yantrik-session")


class GitIdentity(unittest.TestCase):
    def run_install(self, gecos, user="asha", host="yantrik-vm", config=None):
        """Run the install script into a scratch root and return the system gitconfig it left."""
        with tempfile.TemporaryDirectory() as tmp:
            os.makedirs(os.path.join(tmp, "etc"))
            with open(os.path.join(tmp, "etc", "passwd"), "w", encoding="utf-8") as f:
                f.write("%s:x:1000:1000:%s:/home/%s:/bin/bash\n" % (user, gecos, user))
            with open(os.path.join(tmp, "etc", "hostname"), "w", encoding="utf-8") as f:
                f.write(host + "\n")
            if config is not None:
                with open(os.path.join(tmp, "etc", "gitconfig"), "w", encoding="utf-8") as f:
                    f.write(config)
            run = subprocess.run(
                ["sh", INSTALL, "--root", tmp, "--user", user,
                 "--src", os.path.join(HERE, "session"),
                 "--labwc-src", os.path.join(HERE, "..", "..", "config", "labwc")],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(run.returncode, 0, run.stderr)
            cfg = configparser.ConfigParser(interpolation=None)
            cfg.read(os.path.join(tmp, "etc", "gitconfig"), encoding="utf-8")
            return cfg

    def test_neither_set_sets_both_from_the_account(self):
        cfg = self.run_install("Asha Doe")
        self.assertEqual(cfg.get("user", "name").strip('"'), "Asha Doe")
        self.assertEqual(cfg.get("user", "email"), "asha@yantrik-vm.local")

    def test_a_name_already_set_leaves_both_alone(self):
        cfg = self.run_install("Asha Doe", config="[user]\n\tname = Existing Person\n")
        self.assertEqual(cfg.get("user", "name"), "Existing Person")
        self.assertFalse(cfg.has_option("user", "email"))

    def test_an_empty_gecos_falls_back_to_the_user_name(self):
        self.assertEqual(self.run_install("").get("user", "name").strip('"'), "asha")

    def test_only_the_part_before_the_first_comma_of_the_gecos_is_the_name(self):
        self.assertEqual(self.run_install("Full Name,,,").get("user", "name").strip('"'), "Full Name")

    def test_a_quote_in_the_name_cannot_break_the_file(self):
        cfg = self.run_install('Asha "Ash" \\ Doe')
        self.assertNotIn('"', cfg.get("user", "name").strip('"'))

    def test_the_session_no_longer_touches_the_home_gitconfig(self):
        with open(SESSION, encoding="utf-8") as f:
            code = "\n".join(l for l in f.read().splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("git config", code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
