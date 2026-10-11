#!/usr/bin/env python3
"""Self-test for private_addresses.py, the boot test's check that no real address ships.

    python3 deploy/yantrik-os/private-addresses-selftest.py

Holds the check against made-up files, and the updater in this tree against the check — read by
the guest's own way (`grep -HnoE`) and by the Python scanner, which must agree — so a fixture
from a real network, or a lost selftest marker, fails here before it fails a boot test.
"""
import pathlib
import subprocess
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import private_addresses as pa  # noqa: E402

UPDATER = HERE / "yantrik-update"


def check(files):
    return pa.problems({path: pa.scan(text) for path, text in files.items()})


# A stand-in updater: a range definition, the marked selftest with its fixtures, then more script.
GOOD = "\n".join((
    "#!/bin/bash",
    'lan4 = ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")',
    "# YANTRIK-SELFTEST-BEGIN",
    "cmd_selftest() {",
    "  direct 192.168.77.42 8888; direct ::ffff:10.20.30.7 11434",
    "}",
    "# YANTRIK-SELFTEST-END",
    "usage() { :; }",
))


class Check(unittest.TestCase):
    def test_a_clean_tree_passes(self):
        self.assertEqual(check({"/opt/yantrik/config.yaml": "user_name: User\n",
                                "/opt/yantrik/update.conf": "HOST=releases.yantrikos.com\n",
                                "/opt/yantrik/bin/yantrik-update": GOOD}), [])

    def test_any_private_address_in_configuration_fails(self):
        for addr in ("192.168.77.1", "10.0.2.2", "172.20.0.5", "10.0.0.0/8"):
            with self.subTest(addr=addr):
                bad = check({"/opt/yantrik/config.yaml": "api_base_url: http://%s:11434\n" % addr})
                self.assertEqual(len(bad), 1, bad)
                self.assertIn("shipped configuration", bad[0])
        bad = check({"/opt/yantrik/update.conf": "HOST=192.168.77.9\n"})
        self.assertIn("update.conf:1: 192.168.77.9", bad[0])

    def test_public_and_lookalike_numbers_pass_in_configuration(self):
        self.assertEqual(check({"/opt/yantrik/config.yaml":
                                "a: 1.1.1.1\nb: 172.32.0.1\nc: 110.0.0.1\nd: 2.10.3.4\nv: 127.0.0.1\n"}), [])

    def test_a_host_address_outside_the_selftest_fails(self):
        text = GOOD.replace("usage() { :; }", 'SEARX="http://10.20.30.40:8888"')
        bad = check({"yantrik-update": text})
        self.assertEqual(bad, ["yantrik-update:8: 10.20.30.40 is a private host address outside the selftest"])
        text = GOOD.replace("#!/bin/bash", "RELEASES=192.168.77.28")
        self.assertIn("yantrik-update:1: 192.168.77.28", check({"yantrik-update": text})[0])

    def test_cidr_must_be_a_network_not_a_host_with_a_prefix(self):
        text = GOOD.replace("10.0.0.0/8", "10.0.0.7/8")
        bad = check({"yantrik-update": text})
        self.assertEqual(len(bad), 1, bad)
        self.assertIn("10.0.0.7/8", bad[0])

    def test_the_developers_lan_fails_everywhere_even_in_the_selftest(self):
        for addr in ("192.168.4.42", "192.168.4.0/24", "192.168.004.042"):
            with self.subTest(addr=addr):
                bad = check({"yantrik-update": GOOD.replace("192.168.77.42", addr)})
                self.assertEqual(len(bad), 1, bad)
                self.assertIn("developer's own LAN", bad[0])
        bad = check({"/opt/yantrik/config.yaml": "u: http://192.168.4.28/\n"})
        self.assertIn("developer's own LAN", bad[0])

    def test_missing_or_misplaced_markers_fail(self):
        cases = {
            "no begin": GOOD.replace("# YANTRIK-SELFTEST-BEGIN\n", ""),
            "no end": GOOD.replace("# YANTRIK-SELFTEST-END\n", ""),
            "neither": GOOD.replace("# YANTRIK-SELFTEST-BEGIN\n", "").replace("# YANTRIK-SELFTEST-END\n", ""),
            "reversed": GOOD.replace("YANTRIK-SELFTEST-BEGIN", "X").replace("YANTRIK-SELFTEST-END", "YANTRIK-SELFTEST-BEGIN").replace("X", "YANTRIK-SELFTEST-END"),
            "twice": GOOD + "\n# YANTRIK-SELFTEST-BEGIN\n# YANTRIK-SELFTEST-END\n",
        }
        for name, text in cases.items():
            with self.subTest(name):
                bad = check({"yantrik-update": text})
                self.assertTrue(any("wants one YANTRIK-SELFTEST-BEGIN" in b for b in bad), bad)
        # An updater with no lines grep could find at all is missing its markers too.
        self.assertTrue(pa.problems({"/opt/yantrik/bin/yantrik-update": []}))
        # With the markers gone, the fixtures are no longer excused either.
        bad = check({"yantrik-update": cases["neither"]})
        self.assertTrue(any("10.20.30.7" in b for b in bad), bad)

    def test_parse_grep_reads_what_grep_prints(self):
        out = ("/opt/yantrik/bin/yantrik-update:12:YANTRIK-SELFTEST-BEGIN\r\n"
               "/opt/yantrik/bin/yantrik-update:20: 192.168.77.42\r\n"
               "/opt/yantrik/config.yaml:3:/10.0.0.1\r\n")
        self.assertEqual(pa.parse_grep(out), {
            "/opt/yantrik/bin/yantrik-update": [(12, "YANTRIK-SELFTEST-BEGIN"), (20, " 192.168.77.42")],
            "/opt/yantrik/config.yaml": [(3, "/10.0.0.1")]})


class ThisTree(unittest.TestCase):
    def test_the_updater_ships_no_real_address(self):
        self.assertEqual(pa.main([str(UPDATER)]), 0)

    def test_the_guests_grep_and_the_scanner_agree_on_the_updater(self):
        r = subprocess.run(["grep", "-HnoE", pa.GREP_ERE, str(UPDATER)],
                           capture_output=True, text=True, check=True)
        grepped = pa.parse_grep(r.stdout)
        self.assertEqual(grepped, {str(UPDATER): pa.scan(UPDATER.read_text(encoding="utf-8"))})
        self.assertEqual(pa.problems(grepped), [])
        # And it does see what it excuses: the ranges and the fixtures are there to be excused.
        seen = [m for _, m in grepped[str(UPDATER)]]
        self.assertIn(pa.BEGIN, seen)
        self.assertIn(pa.END, seen)
        self.assertTrue(any("10.0.0.0/8" in m for m in seen), seen)
        self.assertTrue(any("192.168.77.42" in m for m in seen), seen)

    def test_the_default_config_ships_no_private_address(self):
        self.assertEqual(pa.main([str(HERE / "config-default.yaml")]), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
