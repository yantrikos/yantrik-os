#!/usr/bin/env python3
"""lint_unset_properties.py -- the `in` property nothing ever writes.

A Slint window declares `in property <string> doc-file-path;`. Slint gives it
the type's default. The Rust side never calls the generated `set_doc_file_path`.
The default is then drawn on screen, and read by everyone as a measurement.

That is Document Editor's Save, which always takes its early return because the
path is empty. It is Network Manager's "Firewall: Off", in warning colour,
which is a hardcoded `false` the app never looked up. It is Spreadsheet's
`cell-grid`, never populated, so `row_count() == 0` and cell-click, cell-edit
and the formula bar all fail their guards behind a perfect blank 50x26 grid.

The check: for every `in` and `in-out` property declared on an app's exported
window, assert that something under that app's `src/` calls the generated
setter. Slint's `foo-bar` becomes Rust's `set_foo_bar`.

A framework macro can be the writer. `yantrik_app::rail_window!(SnippetManagerApp)` implements
`RailView` for the window, and its `render_rail` writes the rail's five properties on every
change; the expansion is not text under the app's `src/`, so no `set_agent_context` is ever
found there. Such a write counts when the app calls the method the macro put it in -- see
`macro_setters` below. An app that implements the rail and never draws it has the same empty
window as one that never wrote the property.

Two severities, because the two kinds of property do not mean the same thing:

  * an `in` property is written by Rust by definition. Never written is a
    failure.
  * an `in-out` property may be driven entirely by the UI -- a text field the
    user types into, a panel the user opens. Never written from Rust is worth
    seeing but is not by itself a fault, so it is reported separately and does
    not fail the run.

Run standalone:  python3 lint_unset_properties.py [--app NAME]
"""

import argparse
import json
import os
import re
import sys

from appscan import discover_apps, repo_root
from srctext import depth_map, line_of, mask_rust, mask_slint, match_bracket, skip_ws

LINT = "unset-property"

# `export component NetworkManagerApp inherits Window {`
COMPONENT_RE = re.compile(
    r"\bexport\s+component\s+(?P<name>[A-Za-z_][\w-]*)"
    r"(?:\s+inherits\s+(?P<base>[A-Za-z_][\w-]*))?\s*\{"
)
# The start of a property declaration. `in-out` first so `in` cannot win the
# prefix; \b on the tail keeps `input` and `internal` out.
PROPERTY_RE = re.compile(r"(?<![\w-])(?P<kind>in-out|in|out|private)\s+property\b")
NAME_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_-]*")
# `yantrik_app::rail_window!(SnippetManagerApp);` -- an app handing part of its window to a
# framework macro. The name is the `macro_rules!` one and the identifier the component it is
# implemented for.
MACRO_INVOCATION_RE = re.compile(
    r"\byantrik_app\s*::\s*(?P<macro>[a-z_]\w*)\s*!\s*\(\s*(?P<component>[A-Za-z_]\w*)\s*\)"
)
MACRO_DEF_RE = re.compile(r"\bmacro_rules!\s*(?P<name>[A-Za-z_]\w*)\s*\{")
MACRO_FN_RE = re.compile(r"\bfn\s+(?P<name>[A-Za-z_]\w*)")
CALLEE_RE = re.compile(r"(?:\.|::)\s*(?P<name>[A-Za-z_]\w*)\s*\(")
SETTER_RE = re.compile(r"(?:\.|::)\s*(?P<setter>set_[A-Za-z0-9_]+)\s*\(")
# Where the macros that build an app's window live.
FRAMEWORK_MACROS = os.path.join("frameworks", "yantrik-app", "src")


class Property:
    def __init__(self, name, kind, line, default=None, alias=None, type_name=None):
        self.name = name
        self.kind = kind          # "in" or "in-out"
        self.line = line
        self.default = default    # source text of the default, or None
        self.alias = alias        # `<=>` target, or None
        self.type_name = type_name

    @property
    def setter(self):
        """Slint normalises - and _ to the same identifier; Rust gets underscores."""
        return "set_" + self.name.replace("-", "_")

    def __repr__(self):
        return "Property(%r, %s, line=%d)" % (self.name, self.kind, self.line)


def parse_window_properties(slint_text):
    """Every `in`/`in-out` property declared directly on the exported component.

    Only the exported component's own body counts: a property declared inside a
    child element belongs to that child and has no generated setter on the
    window. Depth is measured over masked text, so a brace in a string or a
    comment cannot move it.
    """
    masked = mask_slint(slint_text)
    match = COMPONENT_RE.search(masked)
    if not match:
        return []
    body_open = masked.index("{", match.start())
    body_end = match_bracket(masked, body_open)
    body_start = body_open + 1

    depths = depth_map(masked)
    base_depth = depths[body_start]

    props = []
    for decl in PROPERTY_RE.finditer(masked, body_start, body_end):
        if depths[decl.start()] != base_depth:
            continue  # inside a child element, not on the window
        kind = decl.group("kind")
        parsed = _parse_declaration(masked, decl.end(), body_end)
        if parsed is None:
            continue
        name, type_name, default, alias = parsed
        if kind in ("in", "in-out"):
            props.append(
                Property(name, kind, line_of(masked, decl.start()), default, alias, type_name)
            )
    return props


def _parse_declaration(text, pos, limit):
    """Read `<TYPE> name [: default | <=> target] ;` starting after the `property`.

    Returns (name, type_name, default_text, alias_text) or None when the text
    does not look like a declaration at all. Handles the declaration spanning
    any number of lines, because it reads characters rather than lines.
    """
    pos = skip_ws(text, pos)
    type_name = None
    if pos < limit and text[pos] == "<" and not text.startswith("<=>", pos):
        depth, j = 0, pos
        while j < limit:
            if text[j] == "<":
                depth += 1
            elif text[j] == ">":
                depth -= 1
                if depth == 0:
                    j += 1
                    break
            j += 1
        type_name = text[pos + 1 : j - 1].strip()
        pos = skip_ws(text, j)

    name_match = NAME_RE.match(text, pos)
    if not name_match:
        return None
    name = name_match.group(0)
    pos = skip_ws(text, name_match.end())

    default = alias = None
    if text.startswith("<=>", pos):
        alias = _read_to_semicolon(text, pos + 3, limit).strip()
    elif pos < limit and text[pos] == ":":
        default = _read_to_semicolon(text, pos + 1, limit).strip()
    # Anything else (`;`, or a `{` opening a callback-style body) leaves both None.
    return name, type_name, default, alias


def _read_to_semicolon(text, pos, limit):
    """Text up to the `;` that ends the statement, ignoring any inside brackets."""
    depth = 0
    for i in range(pos, limit):
        c = text[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth < 0:
                return text[pos:i]
        elif c == ";" and depth == 0:
            return text[pos:i]
    return text[pos:limit]


def setters_called(rust_sources):
    """Every `set_*` name called anywhere in the app's Rust.

    Matched on `.set_name(` and `::set_name(` with no regard for the receiver,
    which is the point: a setter reached through a differently-named handle, a
    weak upgrade, a helper function or `global::<Theme>()` is still a write.
    Comments and string literals are masked out first, so a commented-out
    setter does not count as a call.

    Returns {name: [(path, line)]}.
    """
    found = {}
    for path, text in rust_sources:
        masked = mask_rust(text)
        for m in SETTER_RE.finditer(masked):
            found.setdefault(m.group("setter"), []).append((path, line_of(masked, m.start())))
    return found


def _read(path):
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        return fh.read()


def framework_macros(framework_dir=None):
    """{macro name: definition body} for every `macro_rules!` the apps can invoke.

    The macros an app hands its window to are in the framework's own src, next to the trait they
    implement. The bodies come back masked, so a brace in one of them cannot move a scan.
    """
    framework_dir = framework_dir or os.path.join(repo_root(), FRAMEWORK_MACROS)
    macros = {}
    if not os.path.isdir(framework_dir):
        return macros
    for name in sorted(os.listdir(framework_dir)):
        if not name.endswith(".rs"):
            continue
        masked = mask_rust(_read(os.path.join(framework_dir, name)))
        for decl in MACRO_DEF_RE.finditer(masked):
            start = masked.index("{", decl.start())
            macros[decl.group("name")] = masked[start : match_bracket(masked, start)]
    return macros


def macro_setters(rust_sources, framework_dir=None):
    """Setters written by a framework macro the app invokes *and* calls.

    A macro's expansion is not under the app's `src/`, so its writes are invisible to
    `setters_called`. They count on one condition: the app's own Rust has to call the method the
    macro defines the write inside -- `render_rail`, for the rail. That is the app proving it
    draws what the macro knows how to draw; an app that implements the rail and never renders it
    still shows an empty one, and stays reported.

    Returns {setter: (macro, method)}.
    """
    invoked, calls = set(), set()
    for _path, text in rust_sources:
        masked = mask_rust(text)
        invoked.update(m.group("macro") for m in MACRO_INVOCATION_RE.finditer(masked))
        calls.update(m.group("name") for m in CALLEE_RE.finditer(masked))

    owners = {}
    if not invoked:
        return owners
    macros = framework_macros(framework_dir)
    for macro in sorted(invoked):
        for setter, method in _setters_by_method(macros.get(macro, "")):
            if method in calls:
                owners.setdefault(setter, (macro, method))
    return owners


def _setters_by_method(macro_body):
    """[(setter, method)] for the setters in a macro body, each by the `fn` it sits in.

    Which `fn` matters because it is the method the app has to call. A setter outside any `fn`
    -- a write in the macro's own scope, run when the type is built rather than when the app
    draws -- is returned under None, which no app can ever call.
    """
    methods = []
    for decl in MACRO_FN_RE.finditer(macro_body):
        start = macro_body.find("{", decl.end())
        if start < 0:
            continue
        methods.append((decl.group("name"), start, match_bracket(macro_body, start)))

    owners = {}
    for m in SETTER_RE.finditer(macro_body):
        for name, start, end in methods:
            if start <= m.start() < end:
                owners.setdefault(m.group("setter"), name)
                break
        else:
            owners.setdefault(m.group("setter"), None)
    return sorted(owners.items())


def check_app(app):
    """Findings for one app: a list of dicts, each with a severity."""
    if not app.slint_path:
        return []
    props = parse_window_properties(app.read_slint())
    rust = app.read_rust()
    called = setters_called(rust)
    by_macro = macro_setters(rust)

    findings = []
    for prop in props:
        if prop.setter in called or prop.setter in by_macro:
            continue
        findings.append(
            {
                "lint": LINT,
                "app": app.name,
                "name": prop.name,
                "kind": prop.kind,
                "severity": "error" if prop.kind == "in" else "warning",
                "setter": prop.setter,
                "line": prop.line,
                "type": prop.type_name,
                "default": prop.default,
                "alias": prop.alias,
                "detail": _detail(prop),
            }
        )
    return findings


def _detail(prop):
    where = "%s property" % prop.kind
    if prop.default is not None:
        shown = prop.default if len(prop.default) <= 40 else prop.default[:37] + "..."
        return "%s, never set from Rust; the window shows its default %s" % (where, shown)
    if prop.alias is not None:
        return "%s aliased to %s, never set from Rust" % (where, prop.alias)
    return "%s, never set from Rust; the window shows the type default" % where


def run(apps):
    return [f for app in apps for f in check_app(app)]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--app", help="check one app instead of all of them")
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    args = parser.parse_args(argv)

    apps = discover_apps(repo_root())
    if args.app:
        apps = [a for a in apps if a.name == args.app]
        if not apps:
            print("no app named %s" % args.app, file=sys.stderr)
            return 2

    findings = run(apps)
    if args.json:
        print(json.dumps(findings, indent=2))
    else:
        for f in findings:
            print(
                "%-18s %-8s %-28s %s:%d  %s"
                % (f["app"], f["severity"], f["name"], "ui/app.slint", f["line"], f["detail"])
            )
    return 1 if any(f["severity"] == "error" for f in findings) else 0


if __name__ == "__main__":
    sys.exit(main())
