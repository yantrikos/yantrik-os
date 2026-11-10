"""The licence gate on the Hermes tree that goes into the image (#1166). Offline, stdlib-only.

An allow-list that fails closed (review of #1169, M2): what is not on it, or declares nothing,
stops the build unless it is a reviewed exception, and so does a tool the image does not ship.
"""

import json
import tempfile
import unittest
from pathlib import Path

import support

support.sys.path.insert(0, str(support.HARNESSES / "lib" / "install"))
import hermes_licences  # noqa: E402

OSI = "Classifier: License :: OSI Approved :: "


class HermesLicenceTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.home = Path(tmp.name)

    def npm(self, name, licence):
        folder = self.home / "hermes-agent" / "node_modules" / name
        folder.mkdir(parents=True)
        body = {"name": name}
        if licence is not None:
            body["license"] = licence
        (folder / "package.json").write_text(json.dumps(body))

    def wheel(self, name, *headers):
        info = self.home / "installs" / "abc" / "lib" / ("%s-1.0.dist-info" % name)
        info.mkdir(parents=True)
        (info / "METADATA").write_text("Name: %s\n%s\n\nbody: GPL is not a header\n" % (name, "\n".join(headers)))

    def tools(self, *entries, recorded=()):
        tools = self.home / "tools"
        tools.mkdir(parents=True)
        for entry in entries:
            (tools / entry).mkdir()
        (tools / ".install.lock").write_text("")
        (tools / "facts.json").write_text(json.dumps({"schema": 1, "packages": {n: {} for n in recorded}}))

    def names(self):
        return [line.split(":", 1)[0] for line in hermes_licences.problems(self.home)]

    def test_the_shapes_the_real_tree_declares_all_pass(self):
        self.npm("left-pad", "MIT")
        self.npm("mpl-thing", "MPL-2.0")
        self.npm("either", "(MIT OR CC0-1.0)")
        self.npm("blue", "BlueOak-1.0.0")
        self.npm("objlic", {"type": "ISC", "url": "x"})
        self.wheel("requests", "License-Expression: Apache-2.0")
        self.wheel("classified", "License: UNKNOWN", OSI + "ISC License (ISCL)")
        self.wheel("dual", "License: Dual License", OSI + "BSD License", OSI + "Apache Software License")
        self.wheel("wordy", "License: Apache License, Version 2.0", OSI + "Apache Software License")
        self.wheel("both", "License-Expression: MPL-2.0 AND MIT")
        self.wheel("psf", "License: PSFL", OSI + "Python Software Foundation License")
        self.wheel("bsd3", "License: 3-Clause BSD License")
        self.wheel("cmu", "License: MIT-CMU")
        self.tools("uv-0.12.3-linux-x64", "python-3.14.7-linux-x64", "node-26-linux-x64", "npm-12-linux-x64", "ripgrep-15-linux-x64",
                   recorded=("uv", "python", "node", "npm", "ripgrep"))
        self.assertEqual(hermes_licences.problems(self.home), [])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 0)

    def test_gsaps_no_charge_licence_stops_the_build(self):
        self.npm("gsap", "Standard 'no charge' license: https://gsap.com/standard-license.")
        self.assertEqual(self.names(), ["gsap"])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 1)

    def test_copyleft_wheels_stop_the_build_and_only_the_header_is_read(self):
        self.wheel("gplthing", "License-Expression: GPL-3.0-or-later")
        self.wheel("lgplthing", OSI + "GNU Lesser General Public License v3 (LGPLv3)")
        self.wheel("agplthing", OSI + "GNU Affero General Public License v3")
        self.wheel("fine", "License-Expression: MIT")
        self.assertEqual(self.names(), ["agplthing", "gplthing", "lgplthing"])

    def test_undeclared_unknown_and_proprietary_fail_closed(self):
        self.wheel("silent")
        self.wheel("odd", "License: Weird-1.0")
        self.wheel("half", "License-Expression: MIT AND Weird-1.0")
        self.wheel("placeholder", "License: UNKNOWN")
        self.npm("closed", "UNLICENSED")
        self.npm("elsewhere", "SEE LICENSE IN LICENSE.txt")
        self.npm("nothing", None)
        self.assertEqual(self.names(), ["closed", "elsewhere", "half", "nothing", "odd", "placeholder", "silent"])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 1)

    def test_a_reviewed_exception_passes_only_while_it_declares_nothing(self):
        self.wheel("resvg_py")
        self.wheel("agent_client_protocol")
        self.npm("@hermes/ink", None)
        self.assertEqual(self.names(), [])
        self.wheel("socksio", "License-Expression: GPL-3.0")
        self.assertEqual(self.names(), ["socksio"], "a licence it now declares is read, not the exception")

    def test_a_tool_the_image_does_not_ship_stops_the_build(self):
        self.tools("uv-0.12.3-linux-x64", "ffmpeg-9.0.1-linux-x64", recorded=("uv", "chromium"))
        self.assertEqual(hermes_licences.tool_problems(self.home),
                         ["tools/ffmpeg-9.0.1-linux-x64: not a tool the image ships",
                          "facts.json records chromium: not a tool the image ships"])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 1)

    def test_the_arguments_are_checked(self):
        self.assertEqual(hermes_licences.main(["x"]), 2)


if __name__ == "__main__":
    unittest.main()
