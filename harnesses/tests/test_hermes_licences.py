"""The licence gate on the Hermes tree that goes into the image (#1166). Offline, stdlib-only."""

import json
import tempfile
import unittest
from pathlib import Path

import support

support.sys.path.insert(0, str(support.HARNESSES / "lib" / "install"))
import hermes_licences  # noqa: E402


class HermesLicenceTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.home = Path(tmp.name)

    def npm(self, name, licence):
        folder = self.home / "hermes-agent" / "node_modules" / name
        folder.mkdir(parents=True)
        (folder / "package.json").write_text(json.dumps({"name": name, "license": licence}))

    def wheel(self, name, *headers):
        info = self.home / "installs" / "abc" / "lib" / ("%s-1.0.dist-info" % name)
        info.mkdir(parents=True)
        (info / "METADATA").write_text("Name: %s\n%s\n\nbody: GPL is not a header\n" % (name, "\n".join(headers)))

    def test_a_permissive_tree_passes(self):
        self.npm("left-pad", "MIT")
        self.npm("mpl-thing", "MPL-2.0")
        self.wheel("requests", "License-Expression: Apache-2.0")
        self.wheel("odd", "License: UNKNOWN")
        self.assertEqual(hermes_licences.restrictive(self.home), [])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 0)

    def test_gsaps_no_charge_licence_stops_the_build(self):
        self.npm("gsap", "Standard 'no charge' license: https://gsap.com/standard-license.")
        self.assertEqual([name for name, _ in hermes_licences.restrictive(self.home)], ["gsap"])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 1)

    def test_copyleft_wheels_stop_the_build_and_only_the_header_is_read(self):
        self.wheel("gplthing", "License-Expression: GPL-3.0-or-later")
        self.wheel("lgplthing", "Classifier: License :: OSI Approved :: GNU Lesser General Public License v3 (LGPLv3)")
        self.wheel("fine", "License-Expression: MIT")
        names = [name for name, _ in hermes_licences.restrictive(self.home)]
        self.assertEqual(names, ["gplthing-1.0.dist-info", "lgplthing-1.0.dist-info"])

    def test_the_arguments_are_checked(self):
        self.assertEqual(hermes_licences.main(["x"]), 2)


if __name__ == "__main__":
    unittest.main()
