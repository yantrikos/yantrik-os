"""The walk behind the service-methods table in docs/app-control.md (#161).

A service answers its `app.describe` / `app.act` through a gate, and its own JSON-RPC methods
beside them with no gate at all: `sysmon.kill_process` signals any pid, `network.wifi_connect`
joins any network, and the ceiling, the mind's mode and the grant are no part of either call.
Gating the methods waits for #43 — a service cannot yet tell the person's own window from any
other peer on the socket, and System Monitor's End button walks in through the method. Until
then the table in docs/app-control.md, section "The methods a service answers beside the gate",
is the record: every method every service answers, whether it changes anything, and the graded
action beside it.

This test holds the record against the source. It reads the `match method` dispatch of every
`services/*/src/main.rs` — string-literal arms, `method::CONST` arms and bare `CONST` arms, the
latter two resolved through crates/yantrik-ipc-contracts — and fails when a dispatched method
has no row in the table, when a row outlives its method, or when a row marked `change` names
neither a grade nor `none`. A new method, mutating or not, cannot join a service until somebody
has written down what it is beside the gate.

Python 3 standard library only. It reads source text; it builds nothing, starts nothing and
writes nothing. Run it with:

    python3 -m unittest discover -s tests/service-methods -v

Parsing limits, stated because a lint that cannot see is worse than one that says so:
  * only `main.rs` is read — every service's dispatch lives there today;
  * everything from the `#[cfg(test)] mod ...` on is ignored, so fixtures are not methods;
  * an arm is recognised by its pattern at the start of a line, so a method name appearing in
    an error string or a client `.call(...)` is not a dispatch;
  * an all-caps arm this cannot resolve through the contracts fails the run rather than being
    skipped — an arm the walk cannot read is an arm the table cannot vouch for.
"""

import re
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SERVICES_DIR = REPO / "services"
CONTRACTS_DIR = REPO / "crates" / "yantrik-ipc-contracts" / "src"
DOC = REPO / "docs" / "app-control.md"

TABLE_OPEN = "<!-- service-methods:"
TABLE_CLOSE = "<!-- /service-methods -->"

GRADES = ("safe", "standard", "sensitive", "dangerous")
KINDS = ("read", "change")

# The two the issue names. Pinned so that no edit of this file, the table or a service can
# quietly drop them: these are the mutations that must stay visible beside their grades.
KNOWN_MUTATIONS = ("sysmon.kill_process", "network.wifi_connect")

CONST_RE = re.compile(r'pub const ([A-Z][A-Z_0-9]*): &str = "([^"]+)";')

# One arm pattern: a method-like token, or several joined by `|`. Anchored at the line start so
# that bodies, error strings and client calls are not read as dispatches. `=>` may sit on a
# later line than the pattern started on.
_TOKEN = r'(?:"[a-z][a-z0-9_.\-]*"|method::[A-Z][A-Z_0-9]*|[A-Z][A-Z_0-9]+)'
ARM_RE = re.compile(r"(?m)^[ \t]*(" + _TOKEN + r"(?:\s*\|\s*" + _TOKEN + r")*)\s*=>")


def read(path):
    return path.read_text(encoding="utf-8")


# The test module, and only the test module: services also carry `#[cfg(test)] use ...` lines
# above their dispatch, and cutting at the first attribute would cut the dispatch away with them.
TEST_MOD_RE = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?mod\s+\w+")


def before_tests(text):
    """The part of a main.rs that serves; fixtures inside `mod tests` are not dispatches."""
    m = TEST_MOD_RE.search(text)
    return text if m is None else text[:m.start()]


def contract_consts():
    """(top-level, in-`method`-module) const name -> wire name, from the contracts crate.

    Two dicts because services import the two shapes differently: network, calendar and email
    dispatch `method::CONST` against a nested `pub mod method`, while notifications globs the
    top level and dispatches bare `CONST`. `MARK_READ` exists in both shapes — email's nested,
    notifications' top-level — and keeping them apart is what makes the bare arm unambiguous.
    """
    top, nested = {}, {}
    for path in sorted(CONTRACTS_DIR.glob("*.rs")):
        text = read(path)
        starts = [m.end() for m in re.finditer(r"pub mod method \{", text)]
        spans = []
        for start in starts:
            depth, i = 1, start
            while i < len(text) and depth:
                depth += (text[i] == "{") - (text[i] == "}")
                i += 1
            spans.append((start, i))

        def nested_at(pos):
            return any(a <= pos < b for a, b in spans)

        for m in CONST_RE.finditer(text):
            target = nested if nested_at(m.start()) else top
            name, value = m.group(1), m.group(2)
            if target.setdefault(name, value) != value:
                raise AssertionError(
                    f"{path.name}: const {name} is defined with two different wire names"
                )
    return top, nested


def dispatched_methods(main_rs, top_consts, nested_consts, errors):
    """Every wire name this service's dispatch answers, `app.*` protocol methods aside."""
    methods = set()
    for arm in ARM_RE.findall(before_tests(read(main_rs))):
        for token in re.findall(_TOKEN, arm):
            if token.startswith('"'):
                name = token.strip('"')
                if "." in name and not name.startswith("app."):
                    methods.add(name)
            elif token.startswith("method::"):
                name = token.split("::", 1)[1]
                if name in nested_consts:
                    methods.add(nested_consts[name])
                else:
                    errors.append(f"{main_rs}: `method::{name}` is not a contracts method const")
            else:
                if token in top_consts:
                    methods.add(top_consts[token])
                else:
                    errors.append(
                        f"{main_rs}: all-caps arm `{token}` resolves to no top-level "
                        "contracts const — if it is a method, name it in the contracts; "
                        "if it is not, this walk cannot read the dispatch"
                    )
    return methods


def scan_services():
    """service directory name -> methods dispatched, plus parse errors to fail on."""
    top_consts, nested_consts = contract_consts()
    errors, scanned = [], {}
    for main_rs in sorted(SERVICES_DIR.glob("*/src/main.rs")):
        methods = dispatched_methods(main_rs, top_consts, nested_consts, errors)
        # A service whose dispatch reads as empty is a parser that broke, not a service that
        # answers nothing: every service here serves at least two methods.
        if not methods:
            errors.append(f"{main_rs}: no dispatched methods found — the walk misread the file")
        scanned[main_rs.parents[1].name] = methods
    return scanned, errors


def table_section():
    text = read(DOC)
    open_at = text.find(TABLE_OPEN)
    close_at = text.find(TABLE_CLOSE)
    if open_at < 0 or close_at < 0 or close_at < open_at:
        raise AssertionError(
            f"docs/app-control.md lost the `{TABLE_OPEN}` / `{TABLE_CLOSE}` markers "
            "around the service-methods table"
        )
    return text[open_at:close_at]


def table_rows():
    """(method -> (service, kind, counterpart)) from the marked table, row by row."""
    rows = {}
    for line in table_section().splitlines():
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.split("|")[1:-1]]
        if len(cells) != 5 or cells[1] in ("Method", "") or set(cells[1]) == {"-"}:
            continue  # header or separator
        service, method, kind, counterpart = cells[0], cells[1].strip("`"), cells[2], cells[3]
        problems = []
        if "." not in method:
            problems.append(f"row `{method}`: the Method cell must be the wire name, with its dot")
        if kind not in KINDS:
            problems.append(f"row `{method}`: kind is `{kind}`, not one of {KINDS}")
        if kind == "change" and counterpart != "none":
            if not any(re.search(rf"\b{g}\b", counterpart) for g in GRADES):
                problems.append(
                    f"row `{method}`: a `change` must stand beside a graded action "
                    f"(name the grade) or say `none`; the cell says `{counterpart}`"
                )
        if kind == "read" and counterpart != "—":
            problems.append(f"row `{method}`: a `read` has no counterpart action; write `—`")
        if method in rows:
            problems.append(f"row `{method}`: listed twice")
        if problems:
            raise AssertionError("; ".join(problems))
        rows[method] = (service, kind, counterpart)
    return rows


class ServiceMethodsTable(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scanned, cls.scan_errors = scan_services()
        cls.rows = table_rows()

    def test_walk_reads_every_service_dispatch(self):
        self.assertEqual([], self.scan_errors, "\n".join(self.scan_errors))
        self.assertGreaterEqual(len(self.scanned), 10, "a service directory went unscanned")

    def test_every_dispatched_method_has_a_row(self):
        missing = sorted(
            f"{method} ({service})"
            for service, methods in self.scanned.items()
            for method in methods
            if method not in self.rows
        )
        self.assertEqual(
            [], missing,
            "the dispatch answers methods the table does not list — add a row for each, "
            "graded beside its `app.act` counterpart if it changes anything:\n  "
            + "\n  ".join(missing),
        )

    def test_every_row_still_has_a_method(self):
        served = {m for methods in self.scanned.values() for m in methods}
        stale = sorted(m for m in self.rows if m not in served)
        self.assertEqual(
            [], stale,
            "the table lists methods no service answers any more — remove the rows:\n  "
            + "\n  ".join(stale),
        )

    def test_known_mutations_are_listed_as_changes(self):
        for method in KNOWN_MUTATIONS:
            self.assertIn(method, self.rows, f"{method} fell out of the table")
            self.assertEqual(
                "change", self.rows[method][1],
                f"{method} is a mutation; the table must not soften it to `read`",
            )

    def test_the_table_is_not_empty(self):
        # A table that parsed to nothing would pass every check above vacuously.
        self.assertGreaterEqual(len(self.rows), 60, "the table lost its rows")
        changes = [m for m, (_, kind, _) in self.rows.items() if kind == "change"]
        self.assertGreaterEqual(len(changes), 20, "the table lost its mutations")


if __name__ == "__main__":
    unittest.main()
