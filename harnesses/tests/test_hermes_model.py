"""hermes_model.py: "Use Yantrik models" for a Hermes that runs as the mind account (#411). The
desktop sends four settings on stdin; the account's tool writes them under `model:` and nothing else
(above all not `platform_toolsets`, the allowlist that keeps Hermes's own terminal, file, code,
browser and web tools off), and Revert puts the earlier section back. Needs PyYAML, as Hermes's does.
"""

import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None

LIB = Path(__file__).resolve().parents[1] / "lib" / "install"
sys.path.insert(0, str(LIB))
_spec = importlib.util.spec_from_file_location("hermes_model", LIB / "hermes_model.py")
hermes_model = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hermes_model)
hermes_config = hermes_model.hermes_config

TOKEN = "ygw-SECRET-token-123"
SPEC = {"default": "picked", "provider": "custom", "base_url": "http://127.0.0.1:7460/v1", "api_key": TOKEN}
THEIRS = "model:\n  default: nvidia/nemotron\n  provider: nvidia\nmemory:\n  provider: yantrikdb\n"


def run(argv, stdin=""):
    out, err = io.StringIO(), io.StringIO()
    old = sys.stdin
    sys.stdin = io.StringIO(stdin)
    try:
        with redirect_stdout(out), redirect_stderr(err):
            code = hermes_model.main(["hermes_model.py", *argv])
    finally:
        sys.stdin = old
    return code, out.getvalue(), err.getvalue()


@unittest.skipIf(yaml is None, "PyYAML")
class ModelTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = os.path.join(self.dir.name, "config.yaml")

    def load(self):
        with open(self.path, encoding="utf-8") as f:
            return yaml.safe_load(f)

    def test_apply_sets_the_four_keys_and_keeps_the_rest(self):
        Path(self.path).write_text(THEIRS + "platform_toolsets:\n  yantrik: [skills, yantrik_os]\n")
        code, out, err = run(["apply", self.path], json.dumps(SPEC))
        self.assertEqual(code, 0, err)
        got = self.load()
        self.assertEqual(got["model"], SPEC)
        self.assertEqual(got["memory"], {"provider": "yantrikdb"})
        self.assertEqual(got["platform_toolsets"], {"yantrik": ["skills", "yantrik_os"]}, "the allowlist is not touched")
        self.assertNotIn(TOKEN, out + err, "the token is never printed")

    def test_the_toolsets_stay_restricted_after_the_model_is_set(self):
        # install writes the allowlist, then "Use Yantrik models" writes the model: the check holds after both
        hermes_config.apply(self.path)
        self.assertEqual(run(["apply", self.path], json.dumps(SPEC))[0], 0)
        got = self.load()
        self.assertEqual(got["platform_toolsets"]["yantrik"], hermes_config.ALLOWED)
        for own in ("terminal", "file", "code_execution", "browser", "web"):
            self.assertNotIn(own, got["platform_toolsets"]["yantrik"])

    def test_revert_puts_the_earlier_model_back_and_a_second_apply_keeps_the_first_copy(self):
        Path(self.path).write_text(THEIRS)
        run(["apply", self.path], json.dumps(SPEC))
        run(["apply", self.path], json.dumps(dict(SPEC, default="other/model")))
        self.assertEqual(run(["revert", self.path])[0], 0)
        self.assertEqual(self.load()["model"], {"default": "nvidia/nemotron", "provider": "nvidia"})
        self.assertFalse(os.path.exists(self.path + hermes_model.KEPT_SUFFIX))
        self.assertEqual(run(["revert", self.path])[0], 1, "nothing left to put back")

    def test_revert_removes_a_model_section_the_file_never_had(self):
        Path(self.path).write_text("memory:\n  provider: yantrikdb\n")
        run(["apply", self.path], json.dumps(SPEC))
        run(["revert", self.path])
        self.assertNotIn("model", self.load())

    def test_an_address_that_is_not_this_machines_is_refused_and_nothing_is_written(self):
        Path(self.path).write_text(THEIRS)
        code, out, err = run(["apply", self.path], json.dumps(dict(SPEC, base_url="https://evil.example/v1")))
        self.assertEqual(code, 1)
        self.assertEqual(Path(self.path).read_text(), THEIRS)
        self.assertNotIn(TOKEN, out + err)

    def test_only_the_gateway_itself_passes_userinfo_and_lookalikes_do_not(self):
        # L1: the old prefix test took http://127.0.0.1:@evil.example/v1, whose host is evil.example.
        for url in ("http://127.0.0.1:@evil.example/v1", "http://127.0.0.1:7460@evil.example/v1",
                    "http://localhost:x@evil.example:7460/v1", "http://user:pw@127.0.0.1:7460/v1",
                    "http://127.0.0.1:7460:@evil.example/", "https://127.0.0.1:7460/v1",
                    "http://127.0.0.1:7461/v1", "http://127.0.0.1/v1", "http://127.0.0.1.evil.example:7460/v1",
                    "http://[::1]:7460@evil.example/v1", "http://127.0.0.1:7460\\@evil.example/v1", "ftp://127.0.0.1:7460/"):
            self.assertFalse(hermes_model.is_gateway(url), url)
            Path(self.path).write_text(THEIRS)
            code, out, err = run(["apply", self.path], json.dumps(dict(SPEC, base_url=url)))
            self.assertEqual(code, 1, url)
            self.assertEqual(Path(self.path).read_text(), THEIRS, url)
        for url in ("http://127.0.0.1:7460/v1", "http://localhost:7460/v1", "http://[::1]:7460/v1"):
            self.assertTrue(hermes_model.is_gateway(url), url)

    def test_revert_after_a_pre_move_apply_removes_the_gateway_keys_and_the_token(self):
        # L3: the copied config has the gateway's model section and no kept copy beside it.
        Path(self.path).write_text(
            "model:\n  default: picked\n  provider: custom\n  base_url: http://127.0.0.1:7460/v1\n"
            "  api_key: %s\nmemory:\n  provider: yantrikdb\n" % TOKEN)
        code, out, err = run(["revert", self.path])
        self.assertEqual(code, 0, err)
        self.assertNotIn("model", self.load())
        self.assertNotIn(TOKEN, Path(self.path).read_text())
        self.assertEqual(self.load()["memory"], {"provider": "yantrikdb"})
        # A model of the person's own is not Yantrik's to remove.
        Path(self.path).write_text(THEIRS)
        self.assertEqual(run(["revert", self.path])[0], 1)
        self.assertEqual(Path(self.path).read_text(), THEIRS)

    def test_a_malformed_file_is_reported_without_quoting_it(self):
        # The YAML library quotes the offending line, which may be the api_key one.
        Path(self.path).write_text("api_key: %s\nmodel: [unclosed\n" % TOKEN)
        code, out, err = run(["apply", self.path], json.dumps(SPEC))
        self.assertEqual(code, 1)
        self.assertNotIn(TOKEN, out + err)

    def test_other_shapes_are_refused(self):
        for body in ("not json", json.dumps({**SPEC, "extra": "x"}), json.dumps({"default": "picked"}), json.dumps(dict(SPEC, api_key=5))):
            Path(self.path).write_text(THEIRS)
            code, out, err = run(["apply", self.path], body)
            self.assertEqual(code, 1, body)
            self.assertEqual(Path(self.path).read_text(), THEIRS)
            self.assertNotIn(TOKEN, out + err)

    def test_a_link_is_not_followed(self):
        target = os.path.join(self.dir.name, "elsewhere.yaml")
        Path(target).write_text(THEIRS)
        os.symlink(target, self.path)
        self.assertEqual(run(["apply", self.path], json.dumps(SPEC))[0], 1)
        self.assertEqual(Path(target).read_text(), THEIRS)

    def test_a_bare_string_model_becomes_the_mapping(self):
        Path(self.path).write_text("model: anthropic/claude-opus\n")
        self.assertEqual(run(["apply", self.path], json.dumps(SPEC))[0], 0)
        self.assertEqual(self.load()["model"], SPEC)
        run(["revert", self.path])
        self.assertEqual(self.load()["model"], "anthropic/claude-opus")

    def read_json(self, path):
        with open(path, encoding="utf-8") as f:
            return json.load(f)

    def test_a_stray_kept_section_and_an_edited_live_one_are_both_kept(self):
        # #1201: a kept section no apply of this tool made, and a live model section edited since.
        old = {"had": True, "model": {"default": "old/model", "provider": "old"}}
        Path(self.path + hermes_model.KEPT_SUFFIX).write_text(json.dumps(old))
        Path(self.path).write_text("model:\n  default: newer/edit\n  provider: nvidia\n")
        os.chmod(self.path, 0o640)
        code, out, err = run(["apply", self.path], json.dumps(SPEC))
        self.assertEqual(code, 0, err)
        copies = [f for f in os.listdir(self.dir.name) if ".model-replaced-" in f]
        self.assertEqual(len(copies), 1, copies)
        copy = os.path.join(self.dir.name, copies[0])
        self.assertEqual(self.read_json(copy)["model"], {"default": "newer/edit", "provider": "nvidia"})
        self.assertEqual(os.stat(copy).st_mode & 0o7777, 0o640, "the copy has the original's mode")
        self.assertIn(copy, out, "Apply names the copy too, as a NOTE line the desktop shows")
        code, out, err = run(["revert", self.path])
        self.assertEqual(code, 0, err)
        self.assertIn(copy, out, "the result names the copy")
        self.assertIn("sudo cat", out, "it says how the person can open a copy the desktop cannot read")
        self.assertEqual(self.load()["model"], old["model"])
        self.assertFalse(os.path.exists(self.path + hermes_model.KEPT_SUFFIX), "set aside, so a later Revert does not go back to it")
        aside = [f for f in os.listdir(self.dir.name) if ".found-" in f]
        self.assertEqual(len(aside), 1, aside)
        self.assertEqual(self.read_json(os.path.join(self.dir.name, aside[0]))["model"], old["model"], "set aside, not deleted")
        self.assertEqual(self.read_json(copy)["model"]["default"], "newer/edit")
        self.assertNotIn(TOKEN, out + err)

    def test_a_second_cycle_keeps_the_second_edit_and_names_only_what_it_holds(self):
        # The review's five steps: stray kept section, live edited to A; apply, revert; edit to B; apply, revert.
        old = {"default": "old/model", "provider": "old"}
        Path(self.path + hermes_model.KEPT_SUFFIX).write_text(json.dumps({"had": True, "model": old}))
        Path(self.path).write_text("model:\n  default: edit/A\n")
        run(["apply", self.path], json.dumps(SPEC))
        code, out, err = run(["revert", self.path])
        self.assertEqual(code, 0, err)
        self.assertEqual(self.load()["model"], old)
        Path(self.path).write_text("model:\n  default: edit/B\n")
        run(["apply", self.path], json.dumps(SPEC))
        code, out, err = run(["revert", self.path])
        self.assertEqual(code, 0, err)
        held = [self.read_json(os.path.join(self.dir.name, f))["model"]["default"] for f in os.listdir(self.dir.name) if ".model-replaced-" in f]
        self.assertEqual(self.load()["model"], {"default": "edit/B"}, "edit/B is on disk")
        self.assertEqual(held, ["edit/A"])
        for f in os.listdir(self.dir.name):
            if ".model-replaced-" in f:
                self.assertNotIn(os.path.join(self.dir.name, f), out, "the second Revert names no copy that holds something else")

    def test_a_kept_file_that_cannot_be_read_is_reported_in_words_and_left_alone(self):
        kept = self.path + hermes_model.KEPT_SUFFIX
        Path(self.path).write_text(THEIRS)
        Path(kept).write_text('{"had": tr')
        for argv, stdin in ((["apply", self.path], json.dumps(SPEC)), (["revert", self.path], "")):
            code, out, err = run(argv, stdin)
            self.assertEqual(code, 1)
            self.assertIn("cannot be read", err)
            self.assertNotIn("Traceback", err)
        self.assertEqual(Path(kept).read_text(), '{"had": tr')
        self.assertEqual(Path(self.path).read_text(), THEIRS)

    def test_the_kept_file_is_replaced_whole_never_truncated_in_place(self):
        # Rewriting the record writes a temp file at 600 and renames it over: the old record is whole until then.
        kept = self.path + hermes_model.KEPT_SUFFIX
        Path(kept).write_text(json.dumps({"had": True, "model": {"default": "old"}}))
        Path(self.path).write_text("model:\n  default: edit\n")
        # A second name keeps the first file's inode taken, so the filesystem cannot hand it out again.
        hold = kept + ".hold"
        os.link(kept, hold)
        before = os.stat(kept).st_ino
        run(["apply", self.path], json.dumps(SPEC))
        self.assertNotEqual(os.stat(kept).st_ino, before, "a new file was put in place")
        self.assertEqual(json.loads(Path(hold).read_text()), {"had": True, "model": {"default": "old"}}, "the old record stayed whole")
        os.unlink(hold)
        self.assertEqual(os.stat(kept).st_mode & 0o777, 0o600)
        self.assertEqual([f for f in os.listdir(self.dir.name) if ".tmp-" in f], [])

    def test_an_edit_while_applied_is_kept_by_the_next_apply(self):
        Path(self.path).write_text(THEIRS)
        run(["apply", self.path], json.dumps(SPEC))
        Path(self.path).write_text("model:\n  default: edited/while/applied\n")
        code, out, err = run(["apply", self.path], json.dumps(SPEC))
        self.assertEqual(code, 0, err)
        self.assertEqual(len([f for f in os.listdir(self.dir.name) if ".model-replaced-" in f]), 1)

    def test_an_ordinary_apply_writes_no_replaced_copy(self):
        Path(self.path).write_text(THEIRS)
        run(["apply", self.path], json.dumps(SPEC))
        run(["apply", self.path], json.dumps(SPEC))
        self.assertEqual([f for f in os.listdir(self.dir.name) if "replaced" in f], [])


if __name__ == "__main__":
    unittest.main()
