#!/usr/bin/env python3
"""Selftest for the git identity block of yantrik-session (#680): it extracts the block between
the markers and runs it with sh under a temporary HOME, with getent, id and hostname replaced by
small fakes first on PATH. Run:
python3 deploy/yantrik-os/yantrik-session-git-identity-selftest.py"""
import configparser
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SESSION = os.path.join(HERE, "yantrik-session")
BEGIN = "# >>> git identity (#680)"
END = "# <<< git identity (#680)"

FENCES = {
    "getent": '#!/bin/sh\nprintf \'%s\\n\' "${FAKE_USER}:x:1000:1000:${FAKE_GECOS}:/home/${FAKE_USER}:/bin/bash"\n',
    "id": '#!/bin/sh\nprintf \'%s\\n\' "${FAKE_USER}"\n',
    "hostname": '#!/bin/sh\nprintf \'%s\\n\' "${FAKE_HOST}"\n',
}


def extract_block():
    with open(SESSION, encoding="utf-8") as f:
        lines = f.read().splitlines()
    try:
        start = lines.index(BEGIN)
        end = lines.index(END)
    except ValueError:
        raise AssertionError("yantrik-session is missing the git identity markers")
    assert start < end, "git identity markers are out of order"
    return "\n".join(lines[start + 1 : end]) + "\n"


class GitIdentity(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.block = extract_block()

    def run_block(self, gecos, user="asha", host="yantrik-vm", config=None):
        """Run the block as sh with the fakes on PATH, and return the resulting global config."""
        with tempfile.TemporaryDirectory() as tmp:
            home = os.path.join(tmp, "home")
            bin_ = os.path.join(tmp, "bin")
            os.mkdir(home)
            os.mkdir(bin_)
            for name, body in FENCES.items():
                path = os.path.join(bin_, name)
                with open(path, "w", encoding="utf-8") as f:
                    f.write(body)
                os.chmod(path, 0o755)
            if config is not None:
                with open(os.path.join(home, ".gitconfig"), "w", encoding="utf-8") as f:
                    f.write(config)
            env = {
                "PATH": bin_ + os.pathsep + os.environ["PATH"],
                "HOME": home,
                "FAKE_USER": user,
                "FAKE_GECOS": gecos,
                "FAKE_HOST": host,
            }
            run = subprocess.run(
                ["sh", "-c", self.block], env=env, capture_output=True, text=True, timeout=30
            )
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(run.stdout, "", "the block must be quiet on success")
            cfg = configparser.ConfigParser(interpolation=None)
            cfg.read(os.path.join(home, ".gitconfig"), encoding="utf-8")
            return cfg

    def test_neither_set_sets_both_from_the_account(self):
        cfg = self.run_block("Asha Doe")
        self.assertEqual(cfg.get("user", "name"), "Asha Doe")
        self.assertEqual(cfg.get("user", "email"), "asha@yantrik-vm.local")

    def test_a_name_already_set_leaves_both_alone(self):
        cfg = self.run_block("Asha Doe", config="[user]\n\tname = Existing Person\n")
        self.assertEqual(cfg.get("user", "name"), "Existing Person")
        self.assertFalse(cfg.has_option("user", "email"))

    def test_an_empty_gecos_falls_back_to_the_user_name(self):
        cfg = self.run_block("")
        self.assertEqual(cfg.get("user", "name"), "asha")

    def test_only_the_part_before_the_first_comma_of_the_gecos_is_the_name(self):
        cfg = self.run_block("Full Name,,,")
        self.assertEqual(cfg.get("user", "name"), "Full Name")


if __name__ == "__main__":
    unittest.main(verbosity=2)
