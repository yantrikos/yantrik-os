#!/usr/bin/env python3
"""test_lints.py -- self-tests for the two app lints.

Every fixture here is a string, not a file in the repo, so the tests say what
the lints do rather than what the apps currently happen to contain. The shapes
are taken from the real source: the alias declaration Spreadsheet uses, the
discard Snippets' save handler uses, the factory call Download Manager passes
instead of a closure.

    python3 -m unittest discover        # in tests/app-lints
"""

import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import appscan
import lint_dead_handlers as dead
import lint_unset_properties as unset
import run as runner
import srctext


# -- the Slint side: which properties are declared ----------------------------


class ParseProperties(unittest.TestCase):
    def props(self, body, header="export component FixtureApp inherits Window {"):
        return unset.parse_window_properties(header + "\n" + body + "\n}\n")

    def names(self, body):
        return [p.name for p in self.props(body)]

    def test_plain_declaration(self):
        props = self.props("    in property <string> notice;")
        self.assertEqual(["notice"], [p.name for p in props])
        self.assertEqual("in", props[0].kind)
        self.assertEqual("set_notice", props[0].setter)
        self.assertIsNone(props[0].default)

    def test_alias_declaration(self):
        """The form every app in this repo actually uses."""
        props = self.props("    in property <[SpreadsheetCell]> cell-grid <=> sheet.cell-grid;")
        self.assertEqual(["cell-grid"], [p.name for p in props])
        self.assertEqual("set_cell_grid", props[0].setter)
        self.assertEqual("sheet.cell-grid", props[0].alias)
        self.assertEqual("[SpreadsheetCell]", props[0].type_name)

    def test_alias_without_a_declared_type(self):
        props = self.props("    in-out property active-tab <=> inner.active-tab;")
        self.assertEqual(["active-tab"], [p.name for p in props])
        self.assertEqual("in-out", props[0].kind)

    def test_default_values(self):
        props = self.props(
            '    in property <string> status:"Opening your library...";\n'
            "    in property <int> line-count: 1;\n"
            "    in property <bool> syntax: true;"
        )
        self.assertEqual(["status", "line-count", "syntax"], [p.name for p in props])
        self.assertEqual("1", props[1].default)
        self.assertEqual("true", props[2].default)

    def test_multi_line_declaration(self):
        props = self.props(
            "    in property\n"
            "        <[CalendarTimeEvent]>\n"
            "        week-events\n"
            "        <=> cal.week-events;\n"
            "    in-out property <string>\n"
            "        find-query: \n"
            '            "";'
        )
        self.assertEqual(["week-events", "find-query"], [p.name for p in props])
        self.assertEqual("cal.week-events", props[0].alias)

    def test_commented_out_declarations_do_not_count(self):
        props = self.props(
            "    // in property <bool> firewall-enabled <=> net.firewall-enabled;\n"
            "    /* in property <bool> wifi-enabled;\n"
            "       in property <int> channel; */\n"
            "    in property <bool> ethernet-up;"
        )
        self.assertEqual(["ethernet-up"], [p.name for p in props])

    def test_string_containing_a_brace_does_not_close_the_component(self):
        props = self.props(
            '    in property <string> hint: "press } to escape";\n'
            "    in property <int> row-count <=> sheet.row-count;"
        )
        self.assertEqual(["hint", "row-count"], [p.name for p in props])

    def test_properties_on_a_child_element_are_not_the_windows(self):
        props = self.props(
            "    in property <int> row-count <=> sheet.row-count;\n"
            "    sheet := SpreadsheetScreen {\n"
            "        in property <int> inner-only;\n"
            "        max-width: 100000px;\n"
            "    }"
        )
        self.assertEqual(["row-count"], [p.name for p in props])

    def test_out_and_private_properties_are_not_ours(self):
        props = self.props(
            "    out property <int> measured;\n"
            "    private property <bool> hidden;\n"
            "    in property <bool> shown;"
        )
        self.assertEqual(["shown"], [p.name for p in props])

    def test_underscores_and_hyphens_reach_the_same_setter(self):
        props = self.props("    in property <string> doc_file_path;")
        self.assertEqual("set_doc_file_path", props[0].setter)

    def test_no_exported_component_yields_nothing(self):
        self.assertEqual([], unset.parse_window_properties("import { Theme } from 'x';"))


# -- the Rust side: which setters are called ----------------------------------


class FindSetters(unittest.TestCase):
    def called(self, src):
        return set(unset.setters_called([("src/main.rs", src)]))

    def test_setter_on_a_differently_named_handle(self):
        """The receiver is never matched on. `ui`, `u`, `app`, a global -- all count."""
        src = """
        fn wire(app: &App) {
            let weak = app.as_weak();
            app.on_x(move || {
                let Some(ui) = weak.upgrade() else { return };
                ui.set_cell_grid(model);
            });
            let u = app.clone();
            u . set_row_count ( 50 );
            app.global::<ThemeMode>().set_dark(true);
            App::set_col_count(&app, 26);
        }
        """
        self.assertEqual(
            {"set_cell_grid", "set_row_count", "set_dark", "set_col_count"}, self.called(src)
        )

    def test_a_commented_out_setter_is_not_a_call(self):
        src = """
        // ui.set_firewall_enabled(state);
        /* ui.set_wifi_enabled(true); */
        ui.set_notice("".into());
        """
        self.assertEqual({"set_notice"}, self.called(src))

    def test_a_setter_named_inside_a_string_is_not_a_call(self):
        src = 'tracing::info!("should call ui.set_doc_file_path(p) one day");'
        self.assertEqual(set(), self.called(src))

    def test_unset_property_is_reported_and_set_one_is_not(self):
        slint = (
            "export component FixtureApp inherits Window {\n"
            "    in property <[Cell]> cell-grid <=> sheet.cell-grid;\n"
            "    in property <string> notice;\n"
            "    in-out property <string> query;\n"
            "}\n"
        )
        props = unset.parse_window_properties(slint)
        called = unset.setters_called([("src/main.rs", "ui.set_notice(x);")])
        missing = [(p.name, p.kind) for p in props if p.setter not in called]
        self.assertEqual([("cell-grid", "in"), ("query", "in-out")], missing)


# -- the dead-handler lint ----------------------------------------------------


class ClosureBodies(unittest.TestCase):
    def one(self, src):
        handlers = dead.find_handlers(src)
        self.assertEqual(1, len(handlers), "expected exactly one registration in %r" % src)
        return handlers[0]

    def test_empty_braces(self):
        h = self.one("app.on_toggle_fit(|| {});")
        self.assertTrue(h.dead)
        self.assertEqual(["empty body"], h.reasons)

    def test_empty_with_an_ignored_argument(self):
        self.assertTrue(self.one("app.on_agent_context_activated(|_| {});").dead)

    def test_log_only(self):
        h = self.one('app.on_snip_copy(|id| { tracing::info!("Copy snippet {}", id); });')
        self.assertEqual(["log only"], h.reasons)

    def test_log_only_without_braces(self):
        h = self.one('app.on_ai_explain_pressed(|| tracing::info!("not wired in standalone"));')
        self.assertEqual(["log only"], h.reasons)

    def test_println_only(self):
        self.assertEqual(["log only"], self.one('app.on_x(|| { println!("hi"); });').reasons)

    def test_discard_only(self):
        """Snippets' save: the payload is named, then thrown away."""
        h = self.one(
            "app.on_snip_save(|id, title, language, code, tags| {\n"
            '    tracing::info!("Save snippet {}: {} ({})", id, title, language);\n'
            "    let _ = (code, tags); // suppress unused warnings\n"
            "});"
        )
        self.assertTrue(h.dead)
        self.assertEqual(["discarded arguments", "log only"], h.reasons)

    def test_comments_only_is_an_empty_body(self):
        h = self.one("app.on_x(move |v| {\n    // TODO: persist v\n});")
        self.assertEqual(["empty body"], h.reasons)

    def test_a_handler_that_logs_and_works_is_alive(self):
        """The case that must never be flagged."""
        h = self.one(
            "app.on_save(move |path| {\n"
            '    tracing::info!("saving {}", path);\n'
            "    let _ = ui.get_content();\n"
            "    std::fs::write(&path, body).unwrap();\n"
            '    tracing::info!("saved");\n'
            "});"
        )
        self.assertFalse(h.dead)

    def test_a_discard_of_real_work_is_alive(self):
        """Download Manager's every handler. The Result is discarded; the work happened."""
        h = self.one(
            "app.on_dl_pause(move |id| { let _ = settle(&ui, &engine, engine.pause(id)); });"
        )
        self.assertFalse(h.dead)

    def test_nested_closures(self):
        """The outer body spawns work; braces and parens nest several deep."""
        src = (
            "app.on_scan_rescan(move || {\n"
            "    let weak = weak.clone();\n"
            "    std::thread::spawn(move || {\n"
            '        tracing::info!("scanning");\n'
            "        let files = scan(&root);\n"
            "        slint::invoke_from_event_loop(move || {\n"
            "            if let Some(ui) = weak.upgrade() { ui.set_tracks(files.into()); }\n"
            "        }).unwrap();\n"
            "    });\n"
            "});"
        )
        handlers = dead.find_handlers(src)
        self.assertEqual(["scan_rescan"], [h.name for h in handlers])
        self.assertFalse(handlers[0].dead)

    def test_a_nested_dead_registration_is_found_inside_a_live_one(self):
        src = (
            "app.on_open(move |p| {\n"
            "    let child = Child::new().unwrap();\n"
            '    child.on_child_save(|| { tracing::info!("nope"); });\n'
            "    child.show().unwrap();\n"
            "});"
        )
        by_name = {h.name: h for h in dead.find_handlers(src)}
        self.assertFalse(by_name["open"].dead)
        self.assertTrue(by_name["child_save"].dead)

    def test_a_function_path_is_not_read(self):
        h = self.one("app.on_ai_reply_suggest(handler);")
        self.assertEqual("delegated", h.kind)
        self.assertFalse(h.dead)

    def test_a_factory_call_is_not_read(self):
        """Download Manager: the closure is built elsewhere and this is not it."""
        h = self.one("app.on_dl_cancel(id_command(app, &engine, |engine, id| engine.cancel(id)));")
        self.assertEqual("delegated", h.kind)
        self.assertFalse(h.dead)

    def test_window_close_requested_is_not_a_generated_callback(self):
        src = "ui.window().on_close_requested(move || slint::CloseRequestResponse::HideWindow);"
        self.assertEqual([], dead.find_handlers(src))

    def test_a_brace_inside_a_string_does_not_end_the_body(self):
        h = self.one('app.on_x(|| { let s = "}{"; ui.set_label(s.into()); });')
        self.assertFalse(h.dead)

    def test_a_returned_constant_is_not_called_dead(self):
        """Not in the four dead shapes. Silence beats a guess."""
        self.assertFalse(self.one("app.on_x(|| Mode::Compact);").dead)

    def test_move_keyword_and_odd_spacing(self):
        h = self.one("app.on_x (\n    move | a , b |\n    {\n    }\n);")
        self.assertEqual(["empty body"], h.reasons)

    def test_registered_twice_and_dead_both_times(self):
        src = 'app.on_x(|| {});\nother.on_x(|| { tracing::info!("no"); });'
        app = _fixture_app(rust={"main.rs": src})
        self.assertEqual(["x"], [f["name"] for f in dead.check_app(app)])

    def test_registered_twice_and_alive_once_is_alive(self):
        src = "app.on_x(|| {});\nother.on_x(move || { ui.set_y(1); });"
        app = _fixture_app(rust={"main.rs": src})
        self.assertEqual([], dead.check_app(app))


class InertExpressions(unittest.TestCase):
    def test_inert(self):
        for expr in ["(code, tags)", "x", "&ui", "row.text", "7", '""', "(a, b, c)", "list[0]"]:
            self.assertTrue(dead.is_inert(expr), expr)

    def test_not_inert(self):
        for expr in [
            "settle(&ui, &engine, r)",
            "tx.send(v)",
            "engine.pause(id)",
            "vec![]",
            "write(path)?",
            "if a { b } else { c }",
            "Foo { bar: 1 }",
        ]:
            self.assertFalse(dead.is_inert(expr), expr)


class Masking(unittest.TestCase):
    def test_rust_lifetimes_survive(self):
        self.assertIn("'a", srctext.mask_rust("fn f<'a>(x: &'a str) {}"))

    def test_rust_char_literal_is_blanked(self):
        masked = srctext.mask_rust("let c = '(';")
        self.assertNotIn("(", masked)

    def test_rust_raw_string_is_blanked(self):
        masked = srctext.mask_rust('let s = r#"a "quoted" {brace}"#;')
        self.assertNotIn("brace", masked)

    def test_line_numbers_are_preserved(self):
        src = 'a\n// }\nb\n"{\n}"\nc\n'
        masked = srctext.mask_rust(src)
        self.assertEqual(src.count("\n"), masked.count("\n"))
        self.assertEqual(len(src), len(masked))


# -- the runner: allowlist, baseline, exit codes ------------------------------

FIXTURE_SLINT = """\
// A fixture window.
export component FixtureApp inherits Window {
    in property <string> set-one;
    in property <string> never-set;
    in-out property <string> typed-into;
    callback pressed <=> inner.pressed;
    inner := FixtureScreen { }
}
"""
FIXTURE_RUST = """\
fn wire(app: &FixtureApp) {
    app.set_set_one("x".into());
    app.on_pressed(|| { tracing::info!("pressed"); });
}
"""


@contextlib.contextmanager
def _captured_stdout():
    """run.main writes a report; the tests want its exit code, not its noise."""
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer), contextlib.redirect_stderr(io.StringIO()):
        yield buffer


def _fixture_app(slint=None, rust=None, name="fixture"):
    """An appscan.App backed by strings, not by files on disk."""
    app = appscan.App(name, os.path.join("apps", name), "ui/app.slint" if slint else None, [])
    app.read_slint = lambda: slint or ""
    app.read_rust = lambda: [(k, v) for k, v in sorted((rust or {}).items())]
    return app


class Runner(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="app-lints-")
        app_dir = os.path.join(self.tmp, "apps", "fixture")
        os.makedirs(os.path.join(app_dir, "ui"))
        os.makedirs(os.path.join(app_dir, "src"))
        with open(os.path.join(app_dir, "ui", "app.slint"), "w", encoding="utf-8") as fh:
            fh.write(FIXTURE_SLINT)
        with open(os.path.join(app_dir, "src", "main.rs"), "w", encoding="utf-8") as fh:
            fh.write(FIXTURE_RUST)
        self.apps = appscan.discover_apps(self.tmp)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def findings(self):
        return runner.collect(self.apps)

    def test_the_fixture_app_has_one_of_each(self):
        errors = [f for f in self.findings() if f["severity"] == "error"]
        self.assertEqual(
            [("unset-property", "never-set"), ("dead-handler", "pressed")],
            [(f["lint"], f["name"]) for f in errors],
        )
        warnings = [f for f in self.findings() if f["severity"] == "warning"]
        self.assertEqual(["typed-into"], [f["name"] for f in warnings])

    def test_an_empty_baseline_makes_everything_new(self):
        result = runner.grade(self.findings(), self.apps, {}, {})
        self.assertEqual(2, len(result["new"]))
        self.assertEqual([], result["stale"])

    def test_a_matching_baseline_makes_everything_known(self):
        baseline = {"fixture": {"unset-property": ["never-set"], "dead-handler": ["pressed"]}}
        result = runner.grade(self.findings(), self.apps, baseline, {})
        self.assertEqual([], result["new"])
        self.assertEqual(2, len(result["known"]))

    def test_a_fixed_item_still_in_the_baseline_is_stale(self):
        baseline = {
            "fixture": {"unset-property": ["never-set", "already-fixed"], "dead-handler": []}
        }
        result = runner.grade(self.findings(), self.apps, baseline, {})
        self.assertEqual(["already-fixed"], [s["name"] for s in result["stale"]])

    def test_a_baseline_for_an_app_that_is_gone_is_stale(self):
        baseline = {"deleted-app": {"dead-handler": ["gone"]}}
        result = runner.grade(self.findings(), self.apps, baseline, {})
        self.assertEqual(["gone"], [s["name"] for s in result["stale"]])

    def test_an_allowlisted_item_is_neither_new_nor_known(self):
        allow = {("dead-handler", "fixture", "pressed"): "the component does the work"}
        result = runner.grade(self.findings(), self.apps, {}, allow)
        self.assertEqual(["never-set"], [f["name"] for f in result["new"]])
        self.assertEqual(["pressed"], [f["name"] for f in result["allowed"]])

    def test_allowlisting_something_in_the_baseline_makes_the_baseline_stale(self):
        baseline = {"fixture": {"dead-handler": ["pressed"]}}
        allow = {("dead-handler", "fixture", "pressed"): "argued for"}
        result = runner.grade(self.findings(), self.apps, baseline, allow)
        self.assertEqual(["pressed"], [s["name"] for s in result["stale"]])

    def test_an_allowlist_entry_matching_nothing_is_reported(self):
        allow = {("dead-handler", "fixture", "no-such-callback"): "stale"}
        result = runner.grade(self.findings(), self.apps, {}, allow)
        self.assertEqual(["no-such-callback"], [e["name"] for e in result["unused_allowlist"]])

    def test_baseline_round_trip(self):
        path = os.path.join(self.tmp, "baseline.json")
        runner.write_baseline(self.findings(), self.apps, {}, path)
        with open(path, encoding="utf-8") as fh:
            document = json.load(fh)
        self.assertEqual(
            {"unset-property": ["never-set"], "dead-handler": ["pressed"]},
            document["apps"]["fixture"],
        )
        result = runner.grade(self.findings(), self.apps, runner.load_baseline(path), {})
        self.assertEqual([], result["new"])
        self.assertEqual([], result["stale"])

    def test_the_baseline_leaves_allowlisted_items_out(self):
        path = os.path.join(self.tmp, "baseline.json")
        allow = {("dead-handler", "fixture", "pressed"): "argued for"}
        document = runner.write_baseline(self.findings(), self.apps, allow, path)
        self.assertEqual({"unset-property": ["never-set"]}, document["apps"]["fixture"])

    def test_main_exits_nonzero_on_new_violations(self):
        with _captured_stdout() as out:
            code = runner.main(["--root", self.tmp, "--ignore-baseline", "--json"])
        self.assertEqual(1, code)
        self.assertFalse(json.loads(out.getvalue())["ok"])

    def test_main_prints_a_readable_report(self):
        with _captured_stdout() as out:
            runner.main(["--root", self.tmp, "--ignore-baseline", "--show-warnings"])
        text = out.getvalue()
        self.assertIn("fixture", text)
        self.assertIn("never-set", text)
        self.assertIn("typed-into", text)

    def test_main_rejects_an_unknown_app(self):
        with _captured_stdout():
            self.assertEqual(2, runner.main(["--root", self.tmp, "--app", "no-such-app"]))


class AllowlistFile(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="app-lints-allow-")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write(self, text, name="allowlist.toml"):
        path = os.path.join(self.tmp, name)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def test_an_entry_without_a_reason_is_an_error(self):
        path = self.write(
            "[dead-handler]\nfixture = [ { name = \"pressed\" } ]\n"
        )
        entries, problems = runner.load_allowlist(path)
        self.assertEqual({}, entries)
        self.assertEqual(1, len(problems))
        self.assertIn("argued for", problems[0])

    def test_a_blank_reason_is_an_error(self):
        path = self.write('[dead-handler]\nfixture = [ { name = "p", reason = "   " } ]\n')
        entries, problems = runner.load_allowlist(path)
        self.assertEqual({}, entries)
        self.assertEqual(1, len(problems))

    def test_a_good_entry_loads(self):
        path = self.write('[unset-property]\nfixture = [ { name = "n", reason = "because" } ]\n')
        entries, problems = runner.load_allowlist(path)
        self.assertEqual([], problems)
        self.assertEqual({("unset-property", "fixture", "n"): "because"}, entries)

    def test_an_unknown_lint_section_is_an_error(self):
        path = self.write('[not-a-lint]\nfixture = [ { name = "n", reason = "r" } ]\n')
        _, problems = runner.load_allowlist(path)
        self.assertEqual(1, len(problems))

    def test_the_repos_own_allowlist_is_well_formed(self):
        entries, problems = runner.load_allowlist(runner.ALLOWLIST_TOML)
        self.assertEqual([], problems)
        self.assertTrue(all(v.strip() for v in entries.values()))


class ShelvedFile(unittest.TestCase):
    """The third register: whole apps that are in the tree and not in the build."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="app-lints-shelf-")
        self.apps = [_fixture_app(name="fixture"), _fixture_app(name="other")]

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write(self, text, name="shelved.toml"):
        path = os.path.join(self.tmp, name)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def test_a_good_entry_loads(self):
        path = self.write('[fixture]\nreason = "nothing under the screen"\n')
        entries, problems = runner.load_shelved(path, self.apps)
        self.assertEqual([], problems)
        self.assertEqual({"fixture": "nothing under the screen"}, entries)

    def test_an_entry_without_a_reason_is_an_error(self):
        path = self.write("[fixture]\n")
        entries, problems = runner.load_shelved(path, self.apps)
        self.assertEqual({}, entries)
        self.assertIn("argued for", problems[0])

    def test_a_blank_reason_is_an_error(self):
        path = self.write('[fixture]\nreason = "   "\n')
        _, problems = runner.load_shelved(path, self.apps)
        self.assertEqual(1, len(problems))

    def test_shelving_an_app_that_does_not_exist_is_an_error(self):
        path = self.write('[no-such-app]\nreason = "gone"\n')
        entries, problems = runner.load_shelved(path, self.apps)
        self.assertEqual({}, entries)
        self.assertIn("no apps/no-such-app", problems[0])

    def test_no_file_means_nothing_is_shelved(self):
        entries, problems = runner.load_shelved(os.path.join(self.tmp, "absent.toml"), self.apps)
        self.assertEqual(({}, []), (entries, problems))

    def test_the_repos_own_shelf_is_well_formed(self):
        apps = appscan.discover_apps(appscan.repo_root())
        entries, problems = runner.load_shelved(runner.SHELVED_TOML, apps)
        self.assertEqual([], problems)
        self.assertTrue(all(v.strip() for v in entries.values()))


class ShelvedGrading(unittest.TestCase):
    """A shelved app is still linted; its debt just does not fail the run."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="app-lints-shelf-grade-")
        for name in ("fixture", "shelfapp"):
            app_dir = os.path.join(self.tmp, "apps", name)
            os.makedirs(os.path.join(app_dir, "ui"))
            os.makedirs(os.path.join(app_dir, "src"))
            with open(os.path.join(app_dir, "ui", "app.slint"), "w", encoding="utf-8") as fh:
                fh.write(FIXTURE_SLINT)
            with open(os.path.join(app_dir, "src", "main.rs"), "w", encoding="utf-8") as fh:
                fh.write(FIXTURE_RUST)
        self.apps = appscan.discover_apps(self.tmp)
        self.findings = runner.collect(self.apps)
        self.shelf = {"shelfapp": "nothing plays audio yet"}

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_a_shelved_apps_findings_leave_the_shipping_lists(self):
        result = runner.grade(self.findings, self.apps, {}, {}, self.shelf)
        self.assertEqual({"fixture"}, {f["app"] for f in result["new"]})
        self.assertEqual({"shelfapp"}, {f["app"] for f in result["shelved_new"]})

    def test_a_shelved_app_is_still_linted(self):
        """Not skipped. The debt is counted and printed, under its own heading."""
        result = runner.grade(self.findings, self.apps, {}, {}, self.shelf)
        self.assertEqual(
            {("unset-property", "never-set"), ("dead-handler", "pressed")},
            {(f["lint"], f["name"]) for f in result["shelved_new"]},
        )
        self.assertTrue(result["apps"]["shelfapp"]["shelved"])
        self.assertFalse(result["apps"]["fixture"]["shelved"])

    def test_a_shelved_apps_known_debt_is_counted_separately(self):
        baseline = {
            "shelfapp": {"unset-property": ["never-set"], "dead-handler": ["pressed"]},
            "fixture": {"unset-property": ["never-set"], "dead-handler": ["pressed"]},
        }
        result = runner.grade(self.findings, self.apps, baseline, {}, self.shelf)
        self.assertEqual(2, len(result["known"]))
        self.assertEqual(2, len(result["shelved_known"]))

    def test_the_report_names_the_shelf_and_its_reason(self):
        result = runner.grade(self.findings, self.apps, {}, {}, self.shelf)
        text = runner.report(result, [])
        self.assertIn("SHELVED", text)
        self.assertIn("nothing plays audio yet", text)
        self.assertIn("shelved (not counted above)", text)
        # The shipping section does not list it a second time.
        shipping = text.split("SHELVED")[0]
        self.assertIn("fixture", shipping)
        self.assertNotIn("shelfapp", shipping)

    def test_a_shelved_apps_debt_does_not_fail_the_run(self):
        with open(os.path.join(self.tmp, "shelved.toml"), "w", encoding="utf-8") as fh:
            fh.write('[shelfapp]\nreason = "nothing under the screen"\n')
        # Everything the fixture app has is allowlisted, so only the shelved app is
        # left to fail on -- and it must not.
        allow = {
            ("unset-property", "fixture", "never-set"): "fixture",
            ("dead-handler", "fixture", "pressed"): "fixture",
        }
        result = runner.grade(
            self.findings, self.apps, {}, allow, {"shelfapp": "nothing under the screen"}
        )
        self.assertEqual([], result["new"])
        self.assertEqual([], result["stale"])
        self.assertEqual(2, len(result["shelved_new"]))


class RealRepo(unittest.TestCase):
    """The findings the lints exist for. If these stop being caught, the lint is broken.

    These name faults that are in the tree today, so the list shrinks as they are fixed. It used
    to hold document-editor's doc-file-path and snippet-manager's snip_save and snip_copy; all
    three were fixed the day this was written, which failed this test for the right reason. When
    the last app here is fixed, the fixtures above are what is left to prove the lint still
    sees, and that is how it should end.

    Music is shelved and it stays here. A shelved app is still linted -- that is the whole point
    of keeping it a workspace member -- and it holds the last real instances of the unset `in`
    property in the tree. Each lint must keep at least one entry that is not network-manager's,
    or the day Network Manager is finished is the day this test stops looking at anything.

    Spreadsheet was here until October 2026, when it came off the shelf (PR #727): its
    cell-grid, row-count and col-count are set now, and none of its handlers only logs. It moved
    to the fixed apps below, which is how an entry here is meant to leave.
    """

    MUST_FLAG_PROPERTIES = {
        "music-player": ["is-scanning", "scan-progress", "scan-status"],
    }
    MUST_FLAG_HANDLERS = {
        "music-player": ["play_track_index"],
    }

    @classmethod
    def setUpClass(cls):
        cls.apps = {a.name: a for a in appscan.discover_apps(appscan.repo_root())}
        if "music-player" not in cls.apps:
            raise unittest.SkipTest("not run from inside the yantrik-os checkout")

    def test_each_lint_is_guarded_by_something_other_than_network_manager(self):
        """One app being fixed must not leave a lint with nothing proving it still sees."""
        for what, table in (
            ("unset properties", self.MUST_FLAG_PROPERTIES),
            ("dead handlers", self.MUST_FLAG_HANDLERS),
        ):
            others = [a for a in table if a != "network-manager"]
            self.assertTrue(others, "%s has only network-manager to prove it works" % what)

    def test_a_shelved_app_is_still_linted(self):
        """Shelved is not deleted: the source is there, it compiles, and it is checked."""
        entries, problems = runner.load_shelved(runner.SHELVED_TOML, list(self.apps.values()))
        self.assertEqual([], problems)
        for name in entries:
            app = self.apps[name]
            self.assertTrue(
                unset.check_app(app) or dead.check_app(app),
                "%s is shelved and reports nothing -- is it still being read?" % name,
            )

    def test_known_unset_properties_are_flagged(self):
        for app_name, names in self.MUST_FLAG_PROPERTIES.items():
            flagged = {
                f["name"]
                for f in unset.check_app(self.apps[app_name])
                if f["severity"] == "error"
            }
            for name in names:
                self.assertIn(name, flagged, "%s %s" % (app_name, name))

    def test_known_dead_handlers_are_flagged(self):
        for app_name, names in self.MUST_FLAG_HANDLERS.items():
            flagged = {f["name"] for f in dead.check_app(self.apps[app_name])}
            for name in names:
                self.assertIn(name, flagged, "%s %s" % (app_name, name))

    def test_the_apps_that_were_fixed_have_no_dead_handlers(self):
        for app_name in ("notes", "terminal", "text-editor", "snippet-manager", "document-editor", "presentation", "network-manager", "email", "spreadsheet"):
            self.assertEqual([], dead.check_app(self.apps[app_name]), app_name)

    def test_the_apps_that_were_fixed_have_no_unset_in_properties(self):
        for app_name in ("notes", "terminal", "text-editor", "snippet-manager", "document-editor", "presentation", "network-manager", "email", "spreadsheet"):
            errors = [
                f for f in unset.check_app(self.apps[app_name]) if f["severity"] == "error"
            ]
            self.assertEqual([], errors, app_name)

    def test_download_managers_settle_handlers_are_not_called_dead(self):
        """The whole app is `let _ = settle(...)`. Flagging it would be the lint's worst bug."""
        flagged = {f["name"] for f in dead.check_app(self.apps["download-manager"])}
        self.assertNotIn("dl_pause", flagged)
        self.assertNotIn("dl_resume", flagged)
        self.assertNotIn("dl_cancel", flagged)

    def test_the_committed_baseline_is_well_formed(self):
        """Whether the baseline still matches the tree is run.py's question, not this
        file's: the tree changes every day and these tests should not go red for it.
        What is asserted here is that the file itself is readable and says what it
        claims -- known lints, known apps, no duplicate rows."""
        baseline = runner.load_baseline()
        self.assertTrue(baseline, "baseline.json is missing or empty")
        for app_name, lints in baseline.items():
            self.assertIn(app_name, self.apps, "baseline names an app that is not there")
            for lint, names in lints.items():
                self.assertIn(lint, runner.LINTS, "baseline names an unknown lint")
                self.assertEqual(sorted(set(names)), names, "%s/%s" % (app_name, lint))

    def test_no_baseline_row_is_also_allowlisted(self):
        baseline = runner.load_baseline()
        allowlist, problems = runner.load_allowlist()
        self.assertEqual([], problems)
        for key in allowlist:
            lint, app_name, name = key
            self.assertNotIn(
                name,
                baseline.get(app_name, {}).get(lint, []),
                "an item cannot be both known debt and a deliberate exemption",
            )


if __name__ == "__main__":
    unittest.main()
