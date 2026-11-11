"""The licence gate on the Hermes tree that goes into the image (#1166). Offline, stdlib-only.

An allow-list that fails closed (review of #1169, M2): what is not on it, or declares nothing,
stops the build unless it is a reviewed exception, and so does a tool the image does not ship. And
it fails closed on a tree that shows it nothing (review of #1169, L-D): an empty tree, no Hermes
wheel, too few packages, no readable `facts.json`, a tool folder that only starts like a real one,
a stranger that borrows a workspace package's name.
"""

import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import support

support.sys.path.insert(0, str(support.HARNESSES / "lib" / "install"))
import hermes_licences  # noqa: E402

OSI = "Classifier: License :: OSI Approved :: "
TOOLS = ("uv-0.12.3-linux-x64", "python-3.14.7-linux-x64", "node-26-linux-x64", "npm-12-linux-x64", "ripgrep-15-linux-x64")
RECORDED = ("uv", "python", "node", "npm", "ripgrep")


class HermesLicenceTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.home = Path(tmp.name)
        self.healthy_tree()

    def healthy_tree(self):
        """The least a real Hermes tree holds: its own wheel, enough packages of each kind, the five tools."""
        self.wheel("hermes_agent", "License-Expression: MIT")
        for n in range(hermes_licences.MIN_PYTHON_PACKAGES):
            self.wheel("filler%d" % n, "License-Expression: MIT")
        for n in range(hermes_licences.MIN_NPM_PACKAGES):
            self.npm("filler%d" % n, "MIT")
        self.tools(*TOOLS, recorded=RECORDED)

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

    def workspace(self, place, name, licence=None):
        """A workspace package: its folder inside the checkout, linked into node_modules as npm does."""
        folder = self.home / "hermes-agent" / place
        folder.mkdir(parents=True)
        body = {"name": name, "private": True}
        if licence:
            body["license"] = licence
        (folder / "package.json").write_text(json.dumps(body))
        link = self.home / "hermes-agent" / "node_modules" / name
        link.parent.mkdir(parents=True, exist_ok=True)
        try:
            os.symlink(folder, link, target_is_directory=True)
        except (OSError, NotImplementedError):
            self.skipTest("symlinks are not available here")

    def tools(self, *entries, recorded=()):
        tools = self.home / "tools"
        shutil.rmtree(tools, ignore_errors=True)
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
        self.workspace("ui-tui/packages/hermes-ink", "@hermes/ink")
        self.assertEqual(self.names(), [])
        self.wheel("socksio", "License-Expression: GPL-3.0")
        self.assertEqual(self.names(), ["socksio"], "a licence it now declares is read, not the exception")

    def test_a_tool_the_image_does_not_ship_stops_the_build(self):
        self.tools("uv-0.12.3-linux-x64", "ffmpeg-9.0.1-linux-x64", recorded=("uv", "chromium"))
        self.assertEqual(hermes_licences.tool_problems(self.home),
                         ["facts.json records chromium: not a tool the image ships",
                          "tools/ffmpeg-9.0.1-linux-x64: not a tool the image ships"])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 1)

    def test_an_empty_tree_is_not_a_clean_one(self):
        shutil.rmtree(self.home)
        self.home.mkdir()
        self.assertNotEqual(hermes_licences.problems(self.home), [])
        self.assertEqual(hermes_licences.main(["x", str(self.home)]), 1)

    def test_a_tree_without_hermes_itself_or_with_too_few_packages_fails(self):
        shutil.rmtree(self.home / "installs")
        self.wheel("filler", "License-Expression: MIT")
        found = hermes_licences.problems(self.home)
        self.assertTrue(any("only 1 Python packages" in line for line in found), found)
        self.assertTrue(any("hermes-agent itself" in line for line in found), found)
        shutil.rmtree(self.home / "hermes-agent")
        self.npm("lonely", "MIT")
        self.assertTrue(any("only 1 npm packages" in line for line in hermes_licences.problems(self.home)))

    def test_hermes_itself_must_be_among_the_wheels_even_with_enough_of_them(self):
        shutil.rmtree(self.home / "installs")
        for n in range(hermes_licences.MIN_PYTHON_PACKAGES + 1):
            self.wheel("filler%d" % n, "License-Expression: MIT")
        self.assertEqual([l for l in hermes_licences.problems(self.home)],
                         ["tree: hermes-agent itself is not among the Python packages read"])

    def test_a_facts_file_that_is_missing_or_unreadable_stops_the_build(self):
        facts = self.home / "tools" / "facts.json"
        for broken in (None, "{not json", "[]", "{}", '{"packages": null}'):
            if broken is None:
                facts.unlink()
            else:
                facts.write_text(broken)
            found = hermes_licences.tool_problems(self.home)
            self.assertEqual(found, ["tools/facts.json: missing or unreadable, so the tools cannot be checked"], broken)
            self.assertEqual(hermes_licences.main(["x", str(self.home)]), 1)

    def test_a_tool_folder_must_match_exactly_not_start_like_one(self):
        self.tools(*TOOLS, "node-ffmpeg-7", "uv-evil-linux-x64", "python3-3.14-linux-x64", recorded=RECORDED)
        self.assertEqual(hermes_licences.tool_problems(self.home),
                         ["tools/node-ffmpeg-7: not a tool the image ships",
                          "tools/python3-3.14-linux-x64: not a tool the image ships",
                          "tools/uv-evil-linux-x64: not a tool the image ships"])

    def test_a_tool_folder_facts_json_does_not_record_is_refused(self):
        self.tools(*TOOLS, recorded=("uv", "python", "node", "npm"))
        self.assertEqual(hermes_licences.tool_problems(self.home), ["tools/ripgrep-15-linux-x64: not a tool the image ships"])

    def test_a_workspace_exception_belongs_to_its_place_in_the_checkout_not_to_its_name(self):
        self.npm("web", None)
        self.assertEqual(self.names(), ["web"], "a stranger called web, declaring nothing")
        shutil.rmtree(self.home / "hermes-agent" / "node_modules" / "web")
        self.workspace("web", "web")
        self.workspace("apps/shared", "@hermes/shared")
        self.workspace("ui-tui", "hermes-tui")
        self.assertEqual(self.names(), [], "the real workspace packages, where Hermes keeps them")
        self.workspace("elsewhere/web", "other-web")
        self.assertEqual(self.names(), ["other-web"])

    def test_the_arguments_are_checked(self):
        self.assertEqual(hermes_licences.main(["x"]), 2)


if __name__ == "__main__":
    unittest.main()
