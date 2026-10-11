"""Every code sample in docs/sdk is code that CI runs.

The guide carries no code of its own. Each code block in it is one of:

* **quoted** from a file this repository builds and tests, marked on the line before the block:

      <!-- from: examples/hello_surface.py -->

  and found in that file line for line — the block may leave out an indentation its lines share
  in the file, and nothing else;
* **output**, marked `<!-- output: <name> -->`, produced here by running what the name says and
  compared with the block.

A `rust`, `python`, `toml`, `ini` or `json` block with neither mark fails, as does a quote that is
no longer in its file, a file that has gone, an output that changed, or a link to a file that does
not exist. Command lines and transcripts (`sh`, `text`) are what a person types and sees; they were
captured from real runs, and the tests that drive each example assert the lines that matter in them.

    python3 -m unittest discover -s docs/sdk/samples -v
"""

import importlib.util
import json
import os
import re
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
GUIDE = os.path.dirname(HERE)
REPO = os.path.abspath(os.path.join(GUIDE, "..", ".."))
SDK = os.path.join(REPO, "sdk", "python")
if SDK not in sys.path:
    sys.path.insert(0, SDK)

from yantrik_surface import Authority, Mode, decide  # noqa: E402

HELD = {"rust", "python", "toml", "ini", "json"}
MARK = re.compile(r"<!--\s*(from|output):\s*(\S+)\s*-->")
FENCE = re.compile(r"^(```+)\s*([\w+-]*)")


def pages():
    return sorted(os.path.join(GUIDE, name) for name in os.listdir(GUIDE) if name.endswith(".md"))


def blocks(path):
    """Every fenced block in a page: (line number, language, mark or None, lines)."""
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    found, i = [], 0
    while i < len(lines):
        opened = FENCE.match(lines[i])
        if not opened:
            i += 1
            continue
        fence, language = opened.group(1), opened.group(2)
        mark = MARK.fullmatch(lines[i - 1].strip()) if i else None
        body, j = [], i + 1
        while j < len(lines) and not lines[j].startswith(fence):
            body.append(lines[j])
            j += 1
        found.append((i + 1, language, mark.groups() if mark else None, body))
        i = j + 1
    return found


def quoted_in(body, text):
    """Whether `body` appears in `text` as consecutive lines, less one shared indentation."""
    want = [line.rstrip() for line in body]
    have = [line.rstrip() for line in text.split("\n")]
    first = next((w for w in want if w), None)
    if first is None:
        return False
    lead = want.index(first)
    for i, line in enumerate(have):
        if not line.endswith(first):
            continue
        indent = line[:len(line) - len(first)]
        if indent.strip():
            continue
        start = i - lead
        if start < 0 or start + len(want) > len(have):
            continue
        if all((have[start + k] == (indent + w if w else "")) or (not w and not have[start + k])
               for k, w in enumerate(want)):
            return True
    return False


# ── outputs: what a block marked `output: <name>` must show ──────────────────


class _PrivateHome:
    """HOME and XDG_RUNTIME_DIR in a temporary directory, so nothing reads the developer's."""

    def __enter__(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="sdk-guide-")
        self.saved = {k: os.environ.get(k) for k in ("HOME", "XDG_RUNTIME_DIR")}
        os.environ["HOME"] = os.environ["XDG_RUNTIME_DIR"] = self.tmp.name
        return self

    def __exit__(self, *exc):
        for key, value in self.saved.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
        self.tmp.cleanup()
        return False


def _load(path, name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(REPO, path))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def template_describe():
    """The Python template's describe, as `app.describe` answers it."""
    with _PrivateHome():
        return _load("templates/python-surface/my_surface.py", "guide_my_surface") \
            .surface.describe_json()


def outcome(grade, mode, description, ceiling="sensitive"):
    refusal = decide(Authority(ceiling, Mode(mode, ())), "app", "act", grade, description)
    if refusal is None:
        return "runs"
    if refusal.startswith("CEILING:"):
        return "refused: ceiling"
    if mode == "plan":
        return "refused: plan"
    return "asks"


def grade_table():
    """What the dispatch does with an act, by grade and mode, on the ceiling this OS ships."""
    modes = ("plan", "ask", "auto", "bypass", "bypass_all")
    rows = [("%-36s %s" % ("sensitive ceiling", "".join("%-17s" % m for m in modes))).rstrip()]
    for label, description in (("", "Do the thing"),
                               (", says it cannot be undone", "Do the thing. It cannot be undone"),
                               (", runs anything it is given",
                                "Do the thing. What it runs can do anything you can.")):
        for grade in ("safe", "standard", "sensitive", "dangerous"):
            cells = "".join("%-17s" % outcome(grade, m, description) for m in modes)
            rows.append(("%-36s %s" % (grade + label, cells)).rstrip())
    return "\n".join(rows)


OUTPUTS = {
    "python-template-describe": ("json", template_describe),
    "grade-table": ("text", grade_table),
}


class TestTheGuide(unittest.TestCase):
    def test_a_quote_is_found_only_where_it_is_line_for_line(self):
        text = "fn main() {\n    let a = 1;\n\n    let b = 2;\n}\n"
        self.assertTrue(quoted_in(["let a = 1;", "", "let b = 2;"], text), "less the indentation")
        self.assertTrue(quoted_in(["    let a = 1;"], text))
        self.assertFalse(quoted_in(["let a = 1;", "let b = 2;"], text), "a line left out")
        self.assertFalse(quoted_in(["let a = 2;"], text), "a line changed")
        self.assertFalse(quoted_in(["fn main() {", "let a = 1;"], text), "indentation not shared")
        self.assertFalse(quoted_in([""], text), "nothing is not a quote")

    def test_there_is_a_guide(self):
        names = {os.path.basename(p) for p in pages()}
        for page in ("README.md", "rust-quickstart.md", "python-quickstart.md", "wrap-an-app.md",
                     "grades.md", "describe.md", "found-while-closed.md", "checking.md"):
            self.assertIn(page, names)

    def test_every_code_block_is_quoted_from_code_ci_runs_or_is_its_output(self):
        seen_outputs = set()
        for page in pages():
            name = os.path.relpath(page, REPO)
            for line, language, mark, body in blocks(page):
                where = "%s:%d" % (name, line)
                if mark is None:
                    self.assertNotIn(language, HELD, "%s: a %s block with no `from:` or `output:` "
                                     "mark — quote it from a file CI runs" % (where, language))
                    continue
                kind, target = mark
                if kind == "from":
                    path = os.path.join(REPO, target)
                    self.assertTrue(os.path.isfile(path), "%s quotes %s, which is not there"
                                    % (where, target))
                    with open(path, encoding="utf-8") as f:
                        self.assertTrue(quoted_in(body, f.read()),
                                        "%s is no longer in %s, line for line; quote it again"
                                        % (where, target))
                else:
                    self.assertIn(target, OUTPUTS, "%s: no producer for output `%s`" % (where, target))
                    seen_outputs.add(target)
                    form, produce = OUTPUTS[target]
                    produced = produce()
                    if form == "json":
                        self.assertEqual(json.loads("\n".join(body)), produced,
                                         "%s: the output changed; it is now:\n%s"
                                         % (where, json.dumps(produced, indent=2, ensure_ascii=False)))
                    else:
                        self.assertEqual("\n".join(body).rstrip(), produced,
                                         "%s: the output changed; it is now:\n%s" % (where, produced))
        self.assertEqual(seen_outputs, set(OUTPUTS), "an output nobody shows")

    def test_every_link_to_a_file_leads_to_one(self):
        link = re.compile(r"\]\(([^)#\s]+)(#[^)]*)?\)")
        for page in pages():
            with open(page, encoding="utf-8") as f:
                text = f.read()
            for target, _ in link.findall(text):
                if re.match(r"[a-z]+://", target):
                    continue
                path = os.path.normpath(os.path.join(os.path.dirname(page), target))
                self.assertTrue(os.path.exists(path), "%s links to %s, which is not there"
                                % (os.path.relpath(page, REPO), target))


class TestTheExamplesAndTemplates(unittest.TestCase):
    def test_a_program_with_a_window_ends_its_loop_through_run_until_closed(self):
        # #198's guard reads apps/*/src/main.rs; these are what an author copies, so they are read
        # here: a main that runs an event loop runs it through run_until_closed, never unwrapped.
        mains = [os.path.join(root, name)
                 for top in ("examples", "templates")
                 for root, _, names in os.walk(os.path.join(REPO, top))
                 for name in names if name == "main.rs"]
        self.assertGreaterEqual(len(mains), 3, mains)
        windowed = 0
        for path in mains:
            with open(path, encoding="utf-8") as f:
                # Comments may name the mistake; the code may not make it.
                flat = "".join(re.sub(r"//.*", "", f.read()).split())
            self.assertNotIn(".run().unwrap()", flat, path)
            self.assertNotIn(".run().expect(", flat, path)
            if "slint::" in flat:
                windowed += 1
                self.assertIn("run_until_closed(", flat, path)
        self.assertGreaterEqual(windowed, 1, "the windowed example was not found")


if __name__ == "__main__":
    unittest.main()
