"""The keep-the-person's-config rule, run from the table both sides share:
fixtures/keep_live_cases.json IS THE RULE. The desktop (crates/yantrik-ui/src/provider_handoff,
rule_table_tests.rs) runs every row of it too; this file runs every row that applies to Hermes's
account tool, hermes_model.py. A change to the rule is a change to the table first.
"""

import json
import os
import tempfile
import unittest
from pathlib import Path

from test_hermes_model import SPEC, hermes_model, run, yaml

TABLE = json.loads((Path(__file__).parent / "fixtures" / "keep_live_cases.json").read_text())


def files_with(directory, infix):
    return sorted(os.path.join(directory, f) for f in os.listdir(directory) if infix in f and ".tmp-" not in f)


def model_of(path):
    with open(path, encoding="utf-8") as f:
        record = json.load(f)
    return record["model"] if record.get("had", True) else None


def canon(values):
    return sorted(json.dumps(v, sort_keys=True) for v in values)


@unittest.skipIf(yaml is None, "PyYAML")
class TableTests(unittest.TestCase):
    def run_case(self, case):
        with tempfile.TemporaryDirectory() as d:
            self._run(case, d)

    def _write_config(self, path, model):
        doc = {"memory": {"provider": "yantrikdb"}}
        if model is not None:
            doc["model"] = model
        Path(path).write_text(yaml.safe_dump(doc))

    def _live(self, path):
        return yaml.safe_load(Path(path).read_text()).get("model")

    def _named(self, d, out, expected, what):
        named = [model_of(p) for p in files_with(d, ".model-replaced-") if p in out]
        self.assertEqual(canon(named), canon(expected), "%s: the copies it names; it said %r" % (what, out))
        for line in out.splitlines():
            self.assertNotIn(SPEC["api_key"], line)

    def _run(self, case, d):
        path = os.path.join(d, "config.yaml")
        kept = path + hermes_model.KEPT_SUFFIX
        self._write_config(path, case["live"])
        ever = [v for v in (case["live"], case["stray"]) if v is not None]
        if case["stray"] is not None:
            Path(kept).write_text(json.dumps({"had": True, "model": case["stray"]}))
        for i, step in enumerate(case["steps"]):
            what = "%s, step %d (%s)" % (case["name"], i, step["do"])
            if step["do"] == "apply":
                code, out, err = run(["apply", path], json.dumps(SPEC))
                self.assertEqual(code, 0, what + err)
                self._named(d, out, step["names"], what)
            elif step["do"] == "revert":
                code, out, err = run(["revert", path])
                self.assertEqual(code, 0, what + err)
                self._named(d, out, step["names"], what)
            elif step["do"] == "edit":
                self._write_config(path, step["value"])
                ever.append(step["value"])
            else:
                self.fail("unknown step %s" % step["do"])
        expect = case["expect"]
        live = self._live(path)
        if expect["live"] == "applied":
            self.assertEqual(live.get("base_url"), SPEC["base_url"], case["name"])
        else:
            self.assertEqual(live, expect["live"], case["name"] + ": live")
        copies = [model_of(p) for p in files_with(d, ".model-replaced-")]
        found = [model_of(p) for p in files_with(d, ".model-before-yantrik.json.found-")]
        stray = model_of(kept) if os.path.exists(kept) else None
        self.assertEqual(canon(copies), canon(expect["copies"]), case["name"] + ": copies")
        self.assertEqual(canon(found), canon(expect["found"]), case["name"] + ": found")
        self.assertEqual(stray, expect["stray"], case["name"] + ": what is left at the kept name")
        on_disk = canon(copies + found + [live, stray])
        for v in ever:
            self.assertIn(canon([v])[0], on_disk, "%s: %s was lost" % (case["name"], v))
        # A copy has the config's mode: it can hold a key.
        mode = os.stat(path).st_mode & 0o7777
        for c in files_with(d, ".model-replaced-"):
            self.assertEqual(os.stat(c).st_mode & 0o7777, mode, c)

    def test_every_row_of_the_shared_table(self):
        ran = 0
        for case in TABLE["cases"]:
            if "python" not in case.get("only", ["rust", "python"]):
                continue
            with self.subTest(case["name"]):
                self.run_case(case)
            ran += 1
        self.assertGreaterEqual(ran, 8)


if __name__ == "__main__":
    unittest.main()
